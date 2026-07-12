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

    // --- DESC DUMP (diagnostic): ground-truth of what the device presents so we pick the
    // right config/interface/endpoints. Remove once selection is confirmed.
    for c in &desc.config_descs {
        println!("usbnetd: DUMP config cfgval={} ifaces={}", c.configuration_value, c.interface_descs.len());
        for i in &c.interface_descs {
            println!(
                "usbnetd: DUMP   if num={} alt={} class={:#x} sub={:#x} proto={:#x} neps={}",
                i.number, i.alternate_setting, i.class, i.sub_class, i.protocol, i.endpoints.len()
            );
            for e in &i.endpoints {
                println!(
                    "usbnetd: DUMP     ep addr={:#04x} attr={:#04x} bulk={} intr={} dir={:?}",
                    e.address, e.attributes, e.is_bulk(), e.is_interrupt(), e.direction()
                );
            }
        }
    }

    // Find the configuration + the CDC-Data interface (class 0x0A) that carries the two
    // bulk endpoints, and the Communications control interface (class 0x02) for RNDIS.
    // The xHCI subdriver numbers endpoints by their 1-based position within the
    // interface's endpoint list (not the raw bEndpointAddress) — same convention usbscsid
    // uses. The CDC-Data interface has exactly the two bulk endpoints.
    let mut chosen: Option<(u8, u8, u8, u8)> = None; // (config_value, data_if_num, alt, ctrl_if_num)
    let mut bulk_in_num = 0u8;
    let mut bulk_out_num = 0u8;
    // xhcid's `endpoints/<n>` numbers endpoints by a 1-based counter that runs across
    // EVERY interface of the chosen config (see `get_endp_desc`), NOT the position within
    // a single interface. RNDIS has a Communications control interface (1 interrupt EP)
    // *before* the CDC-Data interface, so the data interface's bulk IN/OUT land at global
    // indices 2/3, not 1/2. We therefore mirror that exact global walk here.
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

    let mut bulk_in = handle
        .open_endpoint(bulk_in_num)
        .expect("usbnetd: open bulk in");
    let mut bulk_out = handle
        .open_endpoint(bulk_out_num)
        .expect("usbnetd: open bulk out");

    // --- SELFTEST (diagnostic): SYNCHRONOUS TX on the main thread BEFORE the background RX
    // thread exists. Isolates whether concurrent transfers on different endpoints of the same
    // device deadlock (a blocked RX read holding a per-device lock would stall TX). If this
    // logs "tx ok" and the pcap shows the ARP request+reply, TX works and the RX must be made
    // non-blocking-relative-to-TX. Remove once RX is confirmed.
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
        println!("usbnetd: SELFTEST sync TX ARP ({} eth bytes, {} rndis)", arp.len(), msg.len());
        match bulk_out.transfer_write(&msg) {
            Ok(st) => println!("usbnetd: SELFTEST sync TX ok {st:?}"),
            Err(e) => println!("usbnetd: SELFTEST sync TX ERR {e:?}"),
        }
    }

    // Background RX: block on bulk-in, unwrap RNDIS, queue the Ethernet frame, then poke
    // a notify pipe so the (otherwise scheme-driven) event loop wakes and delivers a READ
    // fevent to the netstack — without this, asynchronously-received frames (e.g. DHCP
    // OFFER/ACK) would sit in the queue until the next unrelated scheme op.
    let rx_queue: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::new(Mutex::new(VecDeque::new()));
    let (mut rx_notify_r, rx_notify_w) = std::io::pipe().expect("usbnetd: rx notify pipe");
    {
        let rx_queue = Arc::clone(&rx_queue);
        let mut rx_notify_w = rx_notify_w;
        let mut bulk_in = bulk_in;
        thread::spawn(move || {
            println!("usbnetd: RX thread started");
            let mut buf = vec![0u8; 2048];
            let mut rx_count: u32 = 0;
            let mut attempts: u32 = 0;
            loop {
                let r = bulk_in.transfer_read(&mut buf);
                if attempts < 8 {
                    match &r {
                        Ok(s) => println!(
                            "usbnetd: RX read#{attempts} Ok status={:?} first4={:02x}{:02x}{:02x}{:02x}",
                            s, buf[0], buf[1], buf[2], buf[3]
                        ),
                        Err(e) => println!("usbnetd: RX read#{attempts} Err={e:?}"),
                    }
                    attempts += 1;
                }
                match r {
                    Ok(_) => {
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
                    Err(_) => {
                        // transient (short-packet mismatch, etc.) — back off briefly
                        thread::sleep(std::time::Duration::from_millis(2));
                    }
                }
            }
        });
    }

    let adapter = UsbNet {
        mac,
        bulk_out,
        rx_queue,
        tx_count: 0,
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
    tx_count: u32,
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
        let res = self.bulk_out.transfer_write(&msg);
        if self.tx_count < 4 {
            match &res {
                Ok(st) => println!(
                    "usbnetd: TX frame #{} ({} bytes) -> ok {st:?}",
                    self.tx_count, buf.len()
                ),
                Err(e) => println!(
                    "usbnetd: TX frame #{} ({} bytes) -> ERR {e:?}",
                    self.tx_count, buf.len()
                ),
            }
            self.tx_count += 1;
        }
        res.map_err(|_| Error::new(EIO))?;
        Ok(buf.len())
    }
}
