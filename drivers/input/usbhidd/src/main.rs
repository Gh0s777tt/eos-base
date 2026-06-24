use anyhow::{Context, Result};
use std::{
    collections::{BTreeMap, HashMap},
    env,
    sync::mpsc,
    thread, time,
};

use inputd::ProducerHandle;
use orbclient::KeyEvent as OrbKeyEvent;
use rehid::{
    hidreport::{Report, Usage},
    report_desc::{ReportTy, REPORT_DESC_TY},
    report_handler::ReportHandler,
    usage_tables::{GenericDesktopUsage, UsagePage},
};
use xhcid_interface::{
    ConfigureEndpointsReq, DevDesc, EndpDirection, EndpointTy, PortId, PortReqRecipient,
    XhciClientHandle,
};

use crate::descs::HidDescriptor;

mod descs;
mod overrides;
mod reqs;

fn send_key_event(display: &mut ProducerHandle, usage_page: u16, usage: u16, pressed: bool) {
    let scancode = match usage_page {
        0x07 => match usage {
            0x04 => orbclient::K_A,
            0x05 => orbclient::K_B,
            0x06 => orbclient::K_C,
            0x07 => orbclient::K_D,
            0x08 => orbclient::K_E,
            0x09 => orbclient::K_F,
            0x0A => orbclient::K_G,
            0x0B => orbclient::K_H,
            0x0C => orbclient::K_I,
            0x0D => orbclient::K_J,
            0x0E => orbclient::K_K,
            0x0F => orbclient::K_L,
            0x10 => orbclient::K_M,
            0x11 => orbclient::K_N,
            0x12 => orbclient::K_O,
            0x13 => orbclient::K_P,
            0x14 => orbclient::K_Q,
            0x15 => orbclient::K_R,
            0x16 => orbclient::K_S,
            0x17 => orbclient::K_T,
            0x18 => orbclient::K_U,
            0x19 => orbclient::K_V,
            0x1A => orbclient::K_W,
            0x1B => orbclient::K_X,
            0x1C => orbclient::K_Y,
            0x1D => orbclient::K_Z,
            0x1E => orbclient::K_1,
            0x1F => orbclient::K_2,
            0x20 => orbclient::K_3,
            0x21 => orbclient::K_4,
            0x22 => orbclient::K_5,
            0x23 => orbclient::K_6,
            0x24 => orbclient::K_7,
            0x25 => orbclient::K_8,
            0x26 => orbclient::K_9,
            0x27 => orbclient::K_0,
            0x28 => orbclient::K_ENTER,
            0x29 => orbclient::K_ESC,
            0x2A => orbclient::K_BKSP,
            0x2B => orbclient::K_TAB,
            0x2C => orbclient::K_SPACE,
            0x2D => orbclient::K_MINUS,
            0x2E => orbclient::K_EQUALS,
            0x2F => orbclient::K_BRACE_OPEN,
            0x30 => orbclient::K_BRACE_CLOSE,
            0x31 => orbclient::K_BACKSLASH,
            // 0x32 non-us # and ~
            0x32 => 0x56,
            0x33 => orbclient::K_SEMICOLON,
            0x34 => orbclient::K_QUOTE,
            0x35 => orbclient::K_TICK,
            0x36 => orbclient::K_COMMA,
            0x37 => orbclient::K_PERIOD,
            0x38 => orbclient::K_SLASH,
            0x39 => orbclient::K_CAPS,
            0x3A => orbclient::K_F1,
            0x3B => orbclient::K_F2,
            0x3C => orbclient::K_F3,
            0x3D => orbclient::K_F4,
            0x3E => orbclient::K_F5,
            0x3F => orbclient::K_F6,
            0x40 => orbclient::K_F7,
            0x41 => orbclient::K_F8,
            0x42 => orbclient::K_F9,
            0x43 => orbclient::K_F10,
            0x44 => orbclient::K_F11,
            0x45 => orbclient::K_F12,
            0x46 => orbclient::K_PRTSC,
            0x47 => orbclient::K_SCROLL,
            // 0x48 pause
            0x49 => orbclient::K_INS,
            0x4A => orbclient::K_HOME,
            0x4B => orbclient::K_PGUP,
            0x4C => orbclient::K_DEL,
            0x4D => orbclient::K_END,
            0x4E => orbclient::K_PGDN,
            0x4F => orbclient::K_RIGHT,
            0x50 => orbclient::K_LEFT,
            0x51 => orbclient::K_DOWN,
            0x52 => orbclient::K_UP,
            0x53 => orbclient::K_NUM,
            0x54 => orbclient::K_NUM_SLASH,
            0x55 => orbclient::K_NUM_ASTERISK,
            0x56 => orbclient::K_NUM_MINUS,
            0x57 => orbclient::K_NUM_PLUS,
            0x58 => orbclient::K_NUM_ENTER,
            0x59 => orbclient::K_NUM_1,
            0x5A => orbclient::K_NUM_2,
            0x5B => orbclient::K_NUM_3,
            0x5C => orbclient::K_NUM_4,
            0x5D => orbclient::K_NUM_5,
            0x5E => orbclient::K_NUM_6,
            0x5F => orbclient::K_NUM_7,
            0x60 => orbclient::K_NUM_8,
            0x61 => orbclient::K_NUM_9,
            0x62 => orbclient::K_NUM_0,
            // 0x62 num .
            // 0x64 non-us \ and |
            0x64 => orbclient::K_APP,
            0x66 => orbclient::K_POWER,
            // 0x67 num =
            // unmapped values
            0xE0 => orbclient::K_LEFT_CTRL,
            0xE1 => orbclient::K_LEFT_SHIFT,
            0xE2 => orbclient::K_ALT,
            0xE3 => orbclient::K_LEFT_SUPER,
            0xE4 => orbclient::K_RIGHT_CTRL,
            0xE5 => orbclient::K_RIGHT_SHIFT,
            0xE6 => orbclient::K_ALT_GR,
            0xE7 => orbclient::K_RIGHT_SUPER,
            // reserved values
            _ => {
                log::warn!("unknown usage_page {:#x} usage {:#x}", usage_page, usage);
                return;
            }
        },
        _ => {
            log::warn!("unknown usage_page {:#x}", usage_page);
            return;
        }
    };

    let key_event = OrbKeyEvent {
        character: '\0',
        scancode,
        pressed,
    };

    match display.write_event(key_event.to_event()) {
        Ok(_) => (),
        Err(err) => {
            log::warn!("failed to send key event to orbital: {}", err);
        }
    }
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);

    const USAGE: &'static str = "usbhidd <scheme> <port> <interface>";

    let scheme = args.next().expect(USAGE);
    let port = args
        .next()
        .expect(USAGE)
        .parse::<PortId>()
        .expect("Expected port ID");
    let interface_num = args
        .next()
        .expect(USAGE)
        .parse::<u8>()
        .expect("Expected integer as input of interface");
    let mut report_desc_override = None;
    if let Some(override_name) = args.next() {
        for (name, bytes) in overrides::OVERRIDES.iter() {
            if name == &override_name {
                report_desc_override = Some(bytes.to_vec());
                break;
            }
        }
    }

    let name = format!("{}_{}_{}_hid", scheme, port, interface_num);
    common::setup_logging(
        "usb",
        "usbhid",
        &name,
        common::output_level(),
        common::file_level(),
    );

    log::info!(
        "USB HID driver spawned with scheme `{}`, port {}, interface {}",
        scheme,
        port,
        interface_num
    );

    let handle = XhciClientHandle::new(scheme, port).context("Failed to open XhciClientHandle")?;
    let desc: DevDesc = handle
        .get_standard_descs()
        .context("Failed to get standard descriptors")?;

    log::info!(
        "USB HID driver: {:?} serial {:?}",
        desc.product_str.as_ref().map(|s| s.as_str()).unwrap_or(""),
        desc.serial_str.as_ref().map(|s| s.as_str()).unwrap_or(""),
    );

    log::debug!("{:X?}", desc);

    let mut endp_count = 0;
    let (conf_desc, (if_desc, endp_desc_opt, hid_desc_opt)) = desc
        .config_descs
        .iter()
        .find_map(|conf_desc| {
            let if_desc = conf_desc.interface_descs.iter().find_map(|if_desc| {
                if if_desc.number == interface_num {
                    let endp_desc_opt = if_desc.endpoints.iter().find_map(|endp_desc| {
                        endp_count += 1;
                        if endp_desc.ty() == EndpointTy::Interrupt
                            && endp_desc.direction() == EndpDirection::In
                        {
                            log::warn!(
                                "using endpoint 0x{:x} {:?} {:?}",
                                endp_desc.address,
                                endp_desc.ty(),
                                endp_desc.direction()
                            );
                            Some((endp_count, endp_desc.clone()))
                        } else {
                            log::warn!(
                                "ignoring endpoint 0x{:x} {:?} {:?}",
                                endp_desc.address,
                                endp_desc.ty(),
                                endp_desc.direction()
                            );
                            None
                        }
                    });
                    let hid_desc_opt = if_desc.unknown_descs.iter().find_map(|unknown_desc| {
                        //TODO: should we do any filtering?
                        HidDescriptor::from_bytes(&unknown_desc.all_bytes).ok()
                    });
                    Some((if_desc.clone(), endp_desc_opt, hid_desc_opt))
                } else {
                    endp_count += if_desc.endpoints.len();
                    None
                }
            })?;
            Some((conf_desc.clone(), if_desc))
        })
        .context("Failed to find suitable configuration")?;

    handle
        .configure_endpoints(&ConfigureEndpointsReq {
            config_desc: conf_desc.configuration_value,
            interface_desc: Some(interface_num),
            alternate_setting: Some(if_desc.alternate_setting),
            hub_ports: None,
        })
        .context("Failed to configure endpoints")?;

    //TODO: do we need to set protocol to report? It fails for mice.

    //TODO: dynamically create good values, fix xhcid so it does not block on each request
    // This sets all reports to a duration of 4ms
    if let Err(err) = reqs::set_idle(&handle, 1, 0, interface_num as u16) {
        log::warn!("failed to set idle: {}", err);
    }

    let report_desc_bytes = if let Some(hid_desc) = hid_desc_opt {
        let report_desc_len = hid_desc.get_report_desc()?.desc_len;

        let mut report_desc_bytes = vec![0u8; report_desc_len as usize];
        handle
            .get_descriptor(
                PortReqRecipient::Interface,
                REPORT_DESC_TY,
                0,
                //TODO: should this be an index into interface_descs?
                interface_num as u16,
                &mut report_desc_bytes,
            )
            .context("Failed to retrieve report descriptor")?;

        report_desc_bytes
    } else {
        report_desc_override.expect("failed to find report descriptor")
    };

    let mut handler =
        ReportHandler::new(&report_desc_bytes).expect("failed to parse report descriptor");

    let report_len = match endp_desc_opt {
        Some((_endp_num, endp_desc)) => endp_desc.max_packet_size as usize,
        None => handler.total_byte_length as usize,
    };

    let mut report_collections = HashMap::new();
    for report in handler.descriptor.input_reports() {
        for field in report.fields() {
            let key = (report.report_id().map(u8::from), u32::from(field.id()));
            if let Some(old) = report_collections.get(&key) {
                log::warn!(
                    "key {:?} has old collections {:?} and new collections {:?}",
                    key,
                    old,
                    field.collections()
                );
                continue;
            }
            report_collections.insert(key, field.collections().to_vec());
        }
    }

    let mut display = ProducerHandle::new().context("Failed to open input socket")?;
    let mut endpoint_opt = match endp_desc_opt {
        Some((endp_num, _endp_desc)) => match handle.open_endpoint(endp_num as u8) {
            Ok(ok) => Some(ok),
            Err(err) => {
                log::warn!("failed to open endpoint {endp_num}: {err}");
                None
            }
        },
        None => None,
    };

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || -> Result<()> {
        let mut report_buffer = vec![0u8; report_len];
        let report_ty = ReportTy::Input;
        let report_id = 0;
        loop {
            //TODO: get frequency from device
            //TODO: use sleeps when accuracy is better: thread::sleep(time::Duration::from_millis(10));
            let timer = time::Instant::now();
            while timer.elapsed() < time::Duration::from_millis(1) {
                thread::yield_now();
            }

            if let Some(endpoint) = &mut endpoint_opt {
                // interrupt transfer
                endpoint
                    .transfer_read(&mut report_buffer)
                    .context("failed to get report")?;
            } else {
                // control transfer
                reqs::get_report(
                    &handle,
                    report_ty,
                    report_id,
                    //TODO: should this be an index into interface_descs?
                    interface_num as u16,
                    &mut report_buffer,
                )
                .context("failed to get report")?;
            }

            tx.send(report_buffer.clone())
                .context("failed to send report to main thread")?;
        }
    });

    let mut left_shift = false;
    let mut right_shift = false;
    let mut last_mouse_pos = (0, 0);
    let mut last_buttons = [false, false, false];
    let mut gamepad_state = HashMap::new();
    loop {
        let mut mouse_pos = last_mouse_pos;
        let mut mouse_dx = 0i32;
        let mut mouse_dy = 0i32;
        let mut scroll_y = 0i32;
        let mut buttons = last_buttons;
        match rx.try_recv() {
            Ok(report_buffer) => {
                for event in handler
                    .handle(&report_buffer)
                    .expect("failed to parse report")
                {
                    let mut gamepad = false;
                    if let Some(collections) =
                        report_collections.get(&(event.report_id, event.field_id))
                    {
                        for collection in collections {
                            for usage in collection.usages() {
                                match UsagePage::from_repr(usage.usage_page.into()) {
                                    Some(UsagePage::GenericDesktop) => {
                                        match GenericDesktopUsage::from_repr(usage.usage_id.into())
                                        {
                                            Some(GenericDesktopUsage::GamePad) => {
                                                gamepad = true;
                                            }
                                            _ => {}
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    } else {
                        log::warn!(
                            "collections missing for report {:?} field {}",
                            event.report_id,
                            event.field_id
                        );
                    }

                    // Special handling for gamepad
                    //TODO: make this more generic
                    if gamepad {
                        let last = gamepad_state
                            .get(&(event.usage_page, event.usage))
                            .map_or(0, |x: &(i64, i64)| x.1);
                        gamepad_state.insert((event.usage_page, event.usage), (event.value, last));
                        continue;
                    }

                    match UsagePage::from_repr(event.usage_page) {
                        Some(UsagePage::GenericDesktop) => {
                            match GenericDesktopUsage::from_repr(event.usage) {
                                Some(GenericDesktopUsage::X) => {
                                    if event.relative {
                                        mouse_dx += event.value as i32;
                                    } else {
                                        mouse_pos.0 = event.value as i32;
                                    }
                                }
                                Some(GenericDesktopUsage::Y) => {
                                    if event.relative {
                                        mouse_dy += event.value as i32;
                                    } else {
                                        mouse_pos.1 = event.value as i32;
                                    }
                                }
                                Some(GenericDesktopUsage::Wheel) => {
                                    //TODO: what is X scroll?
                                    if event.relative {
                                        scroll_y += event.value as i32;
                                    } else {
                                        log::warn!("absolute mouse wheel not supported");
                                    }
                                }
                                unsupported => {
                                    log::info!(
                                        "unsupported generic desktop usage 0x{:X}:0x{:X} ({:?}) value {}",
                                        event.usage_page,
                                        event.usage,
                                        unsupported,
                                        event.value
                                    );
                                }
                            }
                        }
                        Some(UsagePage::KeyboardOrKeypad) => {
                            let (pressed, shift_opt) = if event.value != 0 {
                                (true, Some(left_shift | right_shift))
                            } else {
                                (false, None)
                            };
                            if event.usage == 0xE1 {
                                left_shift = pressed;
                            } else if event.usage == 0xE5 {
                                right_shift = pressed;
                            }
                            send_key_event(&mut display, event.usage_page, event.usage, pressed);
                        }
                        Some(UsagePage::Button) => {
                            if event.usage > 0 && event.usage as usize <= buttons.len() {
                                buttons[event.usage as usize - 1] = event.value != 0;
                            } else {
                                log::info!(
                                    "unsupported buttons usage 0x{:X}:0x{:X} value {}",
                                    event.usage_page,
                                    event.usage,
                                    event.value
                                );
                            }
                        }
                        _ => {
                            if event.usage_page >= 0xFF00 {
                                // Ignore vendor defined event
                            } else {
                                log::info!(
                                    "unsupported usage 0x{:X}:0x{:X} value {}",
                                    event.usage_page,
                                    event.usage,
                                    event.value
                                );
                            }
                        }
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => {
                //TODO: get frequency from device
                //TODO: use sleeps when accuracy is better: thread::sleep(time::Duration::from_millis(10));
                let timer = time::Instant::now();
                while timer.elapsed() < time::Duration::from_millis(1) {
                    thread::yield_now();
                }
            }
            Err(err) => {
                anyhow::bail!("failed to recv report buffer from thread: {:?}", err);
            }
        }

        for (&(usage_page, usage), &(value, last)) in gamepad_state.iter() {
            let gamepad_axis = |value| {
                let deadzone = 8096;
                if value < -deadzone {
                    value + deadzone
                } else if value > deadzone {
                    value - deadzone
                } else {
                    0
                }
            };
            let mut gamepad_key = |scancode, pressed| {
                let key_event = OrbKeyEvent {
                    character: '\0',
                    scancode,
                    pressed,
                };

                match display.write_event(key_event.to_event()) {
                    Ok(_) => (),
                    Err(err) => {
                        log::warn!("failed to send key event to orbital: {}", err);
                    }
                }
            };
            let mut gamepad_axis_keys = |value, last, k_neg, k_pos| {
                let threshold = 10240;
                let press_neg = value < -threshold;
                let press_pos = value > threshold;
                if press_neg != (last < -threshold) {
                    gamepad_key(k_neg, press_neg);
                }
                if press_pos != (last > threshold) {
                    gamepad_key(k_pos, press_pos);
                }
            };
            match UsagePage::from_repr(usage_page) {
                Some(UsagePage::GenericDesktop) => match GenericDesktopUsage::from_repr(usage) {
                    Some(GenericDesktopUsage::X) => {
                        gamepad_axis_keys(value, last, orbclient::K_A, orbclient::K_D);
                    }
                    Some(GenericDesktopUsage::Y) => {
                        gamepad_axis_keys(value, last, orbclient::K_S, orbclient::K_W);
                    }
                    Some(GenericDesktopUsage::Z) => {
                        let pressed = value > 64;
                        if pressed != (last > 64) {
                            gamepad_key(orbclient::K_Q, pressed);
                        }
                    }
                    Some(GenericDesktopUsage::Rx) => {
                        mouse_dx += (gamepad_axis(value) / 4096) as i32;
                    }
                    Some(GenericDesktopUsage::Ry) => {
                        mouse_dy -= (gamepad_axis(value) / 4096) as i32;
                    }
                    Some(GenericDesktopUsage::Rz) => {
                        let pressed = value > 64;
                        if pressed != (last > 64) {
                            gamepad_key(orbclient::K_E, pressed);
                        }
                    }
                    Some(GenericDesktopUsage::DpadLeft) => {
                        if value != last {
                            gamepad_key(orbclient::K_LEFT, value != 0);
                        }
                    }
                    Some(GenericDesktopUsage::DpadRight) => {
                        if value != last {
                            gamepad_key(orbclient::K_RIGHT, value != 0);
                        }
                    }
                    Some(GenericDesktopUsage::DpadUp) => {
                        if value != last {
                            gamepad_key(orbclient::K_UP, value != 0);
                        }
                    }
                    Some(GenericDesktopUsage::DpadDown) => {
                        if value != last {
                            gamepad_key(orbclient::K_DOWN, value != 0);
                        }
                    }
                    _ => {}
                },
                Some(UsagePage::Button) => match usage {
                    // A
                    1 => {
                        if value != last {
                            gamepad_key(orbclient::K_SEMICOLON, value != 0)
                        }
                    }
                    // B
                    2 => {
                        if value != last {
                            gamepad_key(orbclient::K_L, value != 0)
                        }
                    }
                    // X
                    3 => {
                        if value != last {
                            gamepad_key(orbclient::K_O, value != 0)
                        }
                    }
                    // Y
                    4 => {
                        if value != last {
                            gamepad_key(orbclient::K_K, value != 0)
                        }
                    }
                    // LB
                    5 => {
                        if value != last {
                            gamepad_key(orbclient::K_I, value != 0)
                        }
                    }
                    // RB
                    6 => {
                        if value != last {
                            gamepad_key(orbclient::K_P, value != 0)
                        }
                    }
                    // Select
                    7 => {
                        if value != last {
                            gamepad_key(orbclient::K_SPACE, value != 0)
                        }
                    }
                    // Start
                    8 => {
                        if value != last {
                            gamepad_key(orbclient::K_ENTER, value != 0)
                        }
                    }
                    _ => {
                        //log::warn!("unknown gamepad button {}", usage)
                    }
                },
                _ => {}
            }
        }
        for (_, (value, last)) in gamepad_state.iter_mut() {
            *last = *value;
        }

        if mouse_pos != last_mouse_pos {
            last_mouse_pos = mouse_pos;

            // TODO
            // ps2d uses 0..=65535 as range, while usb uses 0..=32767. orbital
            // expects the former range, so multiply by two here to temporarily
            // align with orbital expectation. This workaround will make cursor
            //  looks out of sync in QEMU using virtio-vga with usb-tablet.
            let mouse_event = orbclient::event::MouseEvent {
                x: mouse_pos.0 * 2,
                y: mouse_pos.1 * 2,
            };

            match display.write_event(mouse_event.to_event()) {
                Ok(_) => (),
                Err(err) => {
                    log::warn!("failed to send mouse event to orbital: {}", err);
                }
            }
        }

        if mouse_dx != 0 || mouse_dy != 0 {
            // TODO: This is a filter to prevent random mouse jumps
            if mouse_dx > -127 && mouse_dx < 127 {
                let mouse_event = orbclient::event::MouseRelativeEvent {
                    dx: mouse_dx,
                    dy: mouse_dy,
                };

                match display.write_event(mouse_event.to_event()) {
                    Ok(_) => (),
                    Err(err) => {
                        log::warn!("failed to send mouse event to orbital: {}", err);
                    }
                }
            }
        }

        if scroll_y != 0 {
            let scroll_event = orbclient::event::ScrollEvent { x: 0, y: scroll_y };

            match display.write_event(scroll_event.to_event()) {
                Ok(_) => (),
                Err(err) => {
                    log::warn!("failed to send scroll event to orbital: {}", err);
                }
            }
        }

        if buttons != last_buttons {
            last_buttons = buttons;

            let button_event = orbclient::event::ButtonEvent {
                left: buttons[0],
                right: buttons[1],
                middle: buttons[2],
            };

            match display.write_event(button_event.to_event()) {
                Ok(_) => (),
                Err(err) => {
                    log::warn!("failed to send button event to orbital: {}", err);
                }
            }
        }

        // log::trace!("took {}ms", timer.elapsed().as_millis())
    }
}
