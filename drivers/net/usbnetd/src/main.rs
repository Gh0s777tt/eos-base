//! E-OS USB network driver (RNDIS over USB — USB-Ethernet dongles / QEMU `usb-net`).
//!
//! Written clean from the public RNDIS / CDC specifications (protocol constants are
//! non-copyrightable facts) to E-OS's licensing policy. Runs as a userspace xHCI
//! subdriver and exposes a standard `network.*` scheme via the shared `driver-network`
//! crate, so the smoltcp netstack treats a USB NIC exactly like a PCI one.
//!
//! Endpoint model: the CDC-Data interface's two BULK endpoints carry Ethernet frames
//! wrapped in RNDIS_PACKET_MSG headers; the RNDIS control channel (INITIALIZE / QUERY /
//! SET) runs over EP0 class requests (SEND/GET_ENCAPSULATED_*) to the Communications
//! interface. Receive uses a background thread + queue because the xHCI transfer API is
//! synchronous, so the NetworkScheme event loop never blocks on RX.
//!
//! KNOWN LIMITATION (xhcid transfer model): xhcid serves its scheme with a single-threaded
//! `block_on` per transfer, so a blocking bulk-IN read outstanding on the RX thread stalls
//! any concurrent bulk-OUT write (TX). A steady bidirectional flow therefore needs xhcid to
//! grow non-blocking / event-completed transfers (return EAGAIN + post a completion fevent
//! when the fd is O_NONBLOCK). Until then usbnetd enumerates, runs the RNDIS handshake,
//! reads the MAC and registers its `network.*` scheme, and single half-duplex exchanges
//! work (verified: an ARP request egressed and its reply was received and unwrapped), but a
//! continuous DHCP/ping flow deadlocks against that serialization. Tracked in
//! docs/roadmap-connectivity.md.

use std::collections::VecDeque;
use std::env;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::thread;

use driver_network::{NetworkAdapter, NetworkScheme};
use event::{user_data, EventQueue};
use syscall::{Error, Result, EIO};
use xhcid_interface::{
    ConfigureEndpointsReq, DeviceReqData, EndpDirection, PortId, PortReqRecipient, PortReqTy,
    XhciClientHandle, XhciEndpHandle,
};

// ---- RNDIS message types (public spec constants) ----
const RNDIS_PACKET_MSG: u32 = 0x0000_0001;
const RNDIS_INITIALIZE_MSG: u32 = 0x0000_0002;
const RNDIS_INITIALIZE_CMPLT: u32 = 0x8000_0002;
const RNDIS_QUERY_MSG: u32 = 0x0000_0004;
const RNDIS_QUERY_CMPLT: u32 = 0x8000_0004;
const RNDIS_SET_MSG: u32 = 0x0000_0005;
const RNDIS_SET_CMPLT: u32 = 0x8000_0005;

const OID_802_3_PERMANENT_ADDRESS: u32 = 0x0101_0101;
const OID_GEN_CURRENT_PACKET_FILTER: u32 = 0x0001_010E;
// directed + multicast + broadcast + all-multicast + promiscuous
const RNDIS_PACKET_FILTER: u32 = 0x0000_002F;

// CDC class requests on EP0
const SEND_ENCAPSULATED_COMMAND: u8 = 0x00;
const GET_ENCAPSULATED_RESPONSE: u8 = 0x01;

const RNDIS_HDR_LEN: usize = 44; // RNDIS_PACKET_MSG data header

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn push32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn main() {
    daemon::Daemon::new(daemon);
}

fn daemon(daemon: daemon::Daemon) -> ! {
    let mut args = env::args().skip(1);
    const USAGE: &str = "usbnetd <scheme> <port> <if_num>";
    let scheme = args.next().expect(USAGE);
    let port = args
        .next()
        .expect(USAGE)
        .parse::<PortId>()
        .expect("usbnetd: bad port id");
    let data_if: u16 = args.next().expect(USAGE).parse().expect("usbnetd: bad if_num");

    println!("usbnetd: USB net driver on scheme `{scheme}` port {port} data-if {data_if}");

    let handle = XhciClientHandle::new(scheme.clone(), port).expect("usbnetd: XhciClientHandle");
    let desc = handle
        .get_standard_descs()
        .expect("usbnetd: get_standard_descs");

    // Pick the configuration + the CDC-Data interface (class 0x0A) that carries the two bulk
    // endpoints, plus the Communications control interface (class 0x02) for the RNDIS channel.
    //
    // IMPORTANT — endpoint numbering: xhcid's `endpoints/<n>` handle numbers endpoints by a
    // 1-based counter that runs across EVERY interface of the chosen config, in descriptor
    // order (see xhcid `get_endp_desc`), NOT the position within a single interface. RNDIS
    // places a Communications control interface (1 interrupt endpoint) *before* the CDC-Data
    // interface, so the data interface's bulk IN/OUT land at global indices 2/3, not 1/2. We
    // therefore mirror that exact global walk here. (usbscsid gets away with position+1 only
    // because its mass-storage interface is the first interface, where the two coincide.)
    let mut chosen: Option<(u8, u8, u8, u8)> = None; // (config_value, data_if_num, alt, ctrl_if_num)
    let mut bulk_in_num = 0u8;
    let mut bulk_out_num = 0u8;
    for config in &desc.config_descs {
        let ctrl_if = config
            .interface_descs
            .iter()
            .find(|i| i.class == 0x02)
            .map(|i| i.number);
        let mut global = 0u8;
        let mut found_in = 0u8;
        let mut found_out = 0u8;
        let mut data_num_alt: Option<(u8, u8)> = None;
        for ifd in &config.interface_descs {
            for ep in &ifd.endpoints {
                global += 1;
                if ifd.class == 0x0A && ep.is_bulk() {
                    match ep.direction() {
                        EndpDirection::In if found_in == 0 => {
                            found_in = global;
                            data_num_alt.get_or_insert((ifd.number, ifd.alternate_setting));
                        }
                        EndpDirection::Out if found_out == 0 => {
                            found_out = global;
                            data_num_alt.get_or_insert((ifd.number, ifd.alternate_setting));
                        }
                        _ => {}
                    }
                }
            }
        }
        if let (Some((num, alt_s)), true, true) =
            (data_num_alt, found_in != 0, found_out != 0)
        {
            bulk_in_num = found_in;
            bulk_out_num = found_out;
            chosen = Some((
                config.configuration_value,
                num,
                alt_s,
                ctrl_if.unwrap_or(num.saturating_sub(1)),
            ));
            break;
        }
    }
    let (config_value, data_if_num, alt, ctrl_if) =
        chosen.expect("usbnetd: no CDC-Data interface");
    println!(
        "usbnetd: config {config_value} data-if {data_if_num} ctrl-if {ctrl_if} bulk in {bulk_in_num} out {bulk_out_num}"
    );

    handle
        .configure_endpoints(&ConfigureEndpointsReq {
            config_desc: config_value,
            interface_desc: Some(data_if_num),
            alternate_setting: Some(alt),
            hub_ports: None,
        })
        .expect("usbnetd: configure_endpoints");

    // ---- RNDIS control handshake (over EP0 to the control interface) ----
    let ctrl = u16::from(ctrl_if);
    rndis_initialize(&handle, ctrl).expect("usbnetd: RNDIS INITIALIZE failed");
    let mac = rndis_query_mac(&handle, ctrl).expect("usbnetd: RNDIS QUERY MAC failed");
    println!(
        "usbnetd: RNDIS up, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
    rndis_set_filter(&handle, ctrl, RNDIS_PACKET_FILTER).expect("usbnetd: RNDIS SET filter failed");

    // The bulk-IN (RX) endpoint is opened non-blocking so a receive in flight never blocks the
    // single-threaded xhcid scheme loop — which is what would otherwise deadlock a concurrent TX
    // (or another USB device) on the same controller. The bulk-OUT (TX) endpoint stays blocking:
    // a write completes as soon as the device accepts it.
    let bulk_in = handle
        .open_endpoint_nonblock(bulk_in_num)
        .expect("usbnetd: open bulk in");
    let mut bulk_out = handle
        .open_endpoint(bulk_out_num)
        .expect("usbnetd: open bulk out");

    // Background RX: arm a non-blocking bulk-IN read and poll it (yielding between polls so TX and
    // other devices keep running); on each received RNDIS packet, unwrap the Ethernet frame, queue
    // it, and poke a notify pipe so the (otherwise scheme-driven) event loop wakes and delivers a
    // READ fevent to the netstack.
    let rx_queue: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::new(Mutex::new(VecDeque::new()));
    let (mut rx_notify_r, rx_notify_w) = std::io::pipe().expect("usbnetd: rx notify pipe");
    {
        let rx_queue = Arc::clone(&rx_queue);
        let mut rx_notify_w = rx_notify_w;
        let mut bulk_in = bulk_in;
        thread::spawn(move || {
            let mut buf = vec![0u8; 2048];
            let mut rx_count: u32 = 0;
            loop {
                if bulk_in.arm_read(buf.len() as u32).is_err() {
                    thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                let completed = loop {
                    match bulk_in.poll_read(&mut buf) {
                        Ok(Some(_status)) => break true,
                        Ok(None) => thread::sleep(std::time::Duration::from_millis(1)),
                        Err(_) => break false,
                    }
                };
                if !completed {
                    // transient error (short-packet mismatch, etc.) — re-arm after a short pause
                    thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                if buf.len() >= 16 && le32(&buf[0..4]) == RNDIS_PACKET_MSG {
                    let data_off = le32(&buf[8..12]) as usize + 8;
                    let data_len = le32(&buf[12..16]) as usize;
                    if data_len > 0 && data_off + data_len <= buf.len() {
                        let frame = buf[data_off..data_off + data_len].to_vec();
                        if rx_count < 4 {
                            println!("usbnetd: RX frame #{rx_count} ({data_len} bytes)");
                            rx_count += 1;
                        }
                        if let Ok(mut q) = rx_queue.lock() {
                            q.push_back(frame);
                        }
                        let _ = rx_notify_w.write(&[0u8]);
                    }
                }
            }
        });
    }

    {
        let mut arp = Vec::with_capacity(42);
        arp.extend_from_slice(&[0xff; 6]);
        arp.extend_from_slice(&mac);
        arp.extend_from_slice(&[0x08, 0x06, 0x00, 0x01, 0x08, 0x00, 6, 4, 0x00, 0x01]);
        arp.extend_from_slice(&mac);
        arp.extend_from_slice(&[10, 0, 2, 15]);
        arp.extend_from_slice(&[0x00; 6]);
        arp.extend_from_slice(&[10, 0, 2, 2]);
        let mut msg = Vec::with_capacity(RNDIS_HDR_LEN + arp.len());
        push32(&mut msg, RNDIS_PACKET_MSG);
        push32(&mut msg, (RNDIS_HDR_LEN + arp.len()) as u32);
        push32(&mut msg, 36);
        push32(&mut msg, arp.len() as u32);
        msg.extend_from_slice(&[0u8; 28]);
        msg.extend_from_slice(&arp);
        match bulk_out.transfer_write(&msg) {
            Ok(st) => println!("usbnetd: SELFTEST TX ARP ok {st:?}"),
            Err(e) => println!("usbnetd: SELFTEST TX ARP ERR {e:?}"),
        }
    }

    let adapter = UsbNet {
        mac,
        bulk_out,
        rx_queue,
    };

    let name = format!("usb-{scheme}+{port}");
    let mut scheme_obj = NetworkScheme::new(move || adapter, daemon, format!("network.{name}"));

    user_data! { enum Src { Scheme, Rx } }
    let event_queue = EventQueue::<Src>::new().expect("usbnetd: event queue");
    event_queue
        .subscribe(scheme_obj.event_handle().raw(), Src::Scheme, event::EventFlags::READ)
        .expect("usbnetd: subscribe scheme");
    event_queue
        .subscribe(rx_notify_r.as_raw_fd() as usize, Src::Rx, event::EventFlags::READ)
        .expect("usbnetd: subscribe rx");
    scheme_obj.tick().unwrap();
    for event in event_queue.map(|e| e.expect("usbnetd: event")) {
        match event.user_data {
            Src::Scheme => scheme_obj.tick().expect("usbnetd: scheme tick"),
            Src::Rx => {
                let mut drain = [0u8; 64];
                let _ = rx_notify_r.read(&mut drain);
                scheme_obj.tick().expect("usbnetd: rx tick");
            }
        }
    }
    std::process::exit(0);
}

fn encap_send(handle: &XhciClientHandle, ctrl: u16, msg: &[u8]) -> Result<()> {
    handle
        .device_request(
            PortReqTy::Class,
            PortReqRecipient::Interface,
            SEND_ENCAPSULATED_COMMAND,
            0,
            ctrl,
            DeviceReqData::Out(msg),
        )
        .map_err(|_| Error::new(EIO))
}
fn encap_recv(handle: &XhciClientHandle, ctrl: u16, buf: &mut [u8]) -> Result<()> {
    handle
        .device_request(
            PortReqTy::Class,
            PortReqRecipient::Interface,
            GET_ENCAPSULATED_RESPONSE,
            0,
            ctrl,
            DeviceReqData::In(buf),
        )
        .map_err(|_| Error::new(EIO))
}

fn rndis_initialize(handle: &XhciClientHandle, ctrl: u16) -> Result<()> {
    let mut m = Vec::new();
    push32(&mut m, RNDIS_INITIALIZE_MSG);
    push32(&mut m, 24); // MessageLength
    push32(&mut m, 1); // RequestId
    push32(&mut m, 1); // MajorVersion
    push32(&mut m, 0); // MinorVersion
    push32(&mut m, 0x4000); // MaxTransferSize
    encap_send(handle, ctrl, &m)?;
    let mut resp = [0u8; 256];
    encap_recv(handle, ctrl, &mut resp)?;
    if le32(&resp[0..4]) != RNDIS_INITIALIZE_CMPLT {
        return Err(Error::new(EIO));
    }
    Ok(())
}

fn rndis_query_mac(handle: &XhciClientHandle, ctrl: u16) -> Result<[u8; 6]> {
    let mut m = Vec::new();
    push32(&mut m, RNDIS_QUERY_MSG);
    push32(&mut m, 28); // MessageLength
    push32(&mut m, 2); // RequestId
    push32(&mut m, OID_802_3_PERMANENT_ADDRESS);
    push32(&mut m, 0); // InformationBufferLength
    push32(&mut m, 0); // InformationBufferOffset
    push32(&mut m, 0); // Reserved (DeviceVcHandle)
    encap_send(handle, ctrl, &m)?;
    let mut resp = [0u8; 256];
    encap_recv(handle, ctrl, &mut resp)?;
    if le32(&resp[0..4]) != RNDIS_QUERY_CMPLT {
        return Err(Error::new(EIO));
    }
    // InformationBufferOffset is relative to the RequestId field (byte 8).
    let info_off = le32(&resp[20..24]) as usize + 8;
    let info_len = le32(&resp[16..20]) as usize;
    if info_len < 6 || info_off + 6 > resp.len() {
        return Err(Error::new(EIO));
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&resp[info_off..info_off + 6]);
    Ok(mac)
}

fn rndis_set_filter(handle: &XhciClientHandle, ctrl: u16, filter: u32) -> Result<()> {
    let mut m = Vec::new();
    push32(&mut m, RNDIS_SET_MSG);
    push32(&mut m, 28 + 4); // MessageLength
    push32(&mut m, 3); // RequestId
    push32(&mut m, OID_GEN_CURRENT_PACKET_FILTER);
    push32(&mut m, 4); // InformationBufferLength
    push32(&mut m, 20); // InformationBufferOffset (data at byte 28)
    push32(&mut m, 0); // Reserved
    push32(&mut m, filter);
    encap_send(handle, ctrl, &m)?;
    let mut resp = [0u8; 256];
    encap_recv(handle, ctrl, &mut resp)?;
    if le32(&resp[0..4]) != RNDIS_SET_CMPLT {
        return Err(Error::new(EIO));
    }
    Ok(())
}

struct UsbNet {
    mac: [u8; 6],
    bulk_out: XhciEndpHandle,
    rx_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
}

impl NetworkAdapter for UsbNet {
    fn mac_address(&mut self) -> [u8; 6] {
        self.mac
    }

    fn available_for_read(&mut self) -> usize {
        self.rx_queue.lock().map(|q| q.len()).unwrap_or(0)
    }

    fn read_packet(&mut self, buf: &mut [u8]) -> Result<Option<usize>> {
        let mut q = self.rx_queue.lock().map_err(|_| Error::new(EIO))?;
        match q.pop_front() {
            Some(frame) => {
                let n = frame.len().min(buf.len());
                buf[..n].copy_from_slice(&frame[..n]);
                Ok(Some(n))
            }
            None => Ok(None),
        }
    }

    fn write_packet(&mut self, buf: &[u8]) -> Result<usize> {
        let mut msg = Vec::with_capacity(RNDIS_HDR_LEN + buf.len());
        push32(&mut msg, RNDIS_PACKET_MSG);
        push32(&mut msg, (RNDIS_HDR_LEN + buf.len()) as u32); // MessageLength
        push32(&mut msg, 36); // DataOffset (from byte 8 -> data at 44)
        push32(&mut msg, buf.len() as u32); // DataLength
        msg.extend_from_slice(&[0u8; 28]); // OOB/PerPacket/Reserved fields
        msg.extend_from_slice(buf);
        self.bulk_out
            .transfer_write(&msg)
            .map_err(|_| Error::new(EIO))?;
        Ok(buf.len())
    }
}
