use std::fs::File;
use std::sync::Arc;

use common::MemoryType;
use pcid_interface::*;

use crate::spec::*;
use crate::transport::{Error, InterruptMethod, StandardTransport, Transport};

pub struct Device {
    pub transport: Arc<dyn Transport>,
    pub device_space: *const u8,
    pub irq_handle: File,
}

// FIXME(andypython): `device_space` should not be `Send` nor `Sync`. Take
// it out of `Device`.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

pub const MSIX_PRIMARY_VECTOR: u16 = 0;

/// VirtIO Device Probe
///
/// ## Device State
/// After this function, the device will have been successfully reseted and is ready for use.
///
/// The caller is required to do the following:
/// * Negotiate the device and driver supported features (finialize via [`StandardTransport::finalize_features`])
/// * Create the device specific virtio queues (via [`StandardTransport::setup_queue`]). This is *required* to be done
///   before starting the device.
/// * Finally start the device (via [`StandardTransport::run_device`]). At this point, the device
///   is alive.
///
/// ## Errors
/// Returns [`Error::InCapable`] if the device exposes no modern (virtio 1.0)
/// PCI capabilities — i.e. a legacy-only device — so the calling driver can
/// exit gracefully instead of aborting.
///
/// ## Panics
/// This function panics if the device is not a virtio device.
pub fn probe_device(pcid_handle: &mut PciFunctionHandle) -> Result<Device, Error> {
    let pci_config = pcid_handle.config();

    assert_eq!(
        pci_config.func.full_device_id.vendor_id, 6900,
        "virtio_core::probe_device: not a virtio device"
    );

    let mut common_addr = None;
    let mut notify_addr = None;
    let mut device_addr = None;
    let mut isr_addr = None;

    for raw_capability in pcid_handle.get_vendor_capabilities() {
        // SAFETY: We have verified that the length of the data is correct.
        let capability = unsafe { &*(raw_capability.data.as_ptr() as *const PciCapability) };

        match capability.cfg_type {
            CfgType::Common | CfgType::Notify | CfgType::Device | CfgType::Isr => {}
            _ => continue,
        }

        let mapped_bar = unsafe { pcid_handle.map_bar(capability.bar, MemoryType::Uncacheable) };
        let address = mapped_bar.ptr.expose_provenance().get() + capability.offset as usize;

        match capability.cfg_type {
            CfgType::Common => {
                debug_assert!(common_addr.is_none());
                common_addr = Some(address);
            }

            CfgType::Notify => {
                debug_assert!(notify_addr.is_none());

                // SAFETY: The capability type is `Notify`, so its safe to access
                //         the `notify_multiplier` field.
                let multiplier = unsafe {
                    (&*(raw_capability.data.as_ptr() as *const PciCapability
                        as *const PciCapabilityNotify))
                        .notify_off_multiplier()
                };
                notify_addr = Some((address, multiplier));
            }

            CfgType::Device => {
                debug_assert!(device_addr.is_none());
                device_addr = Some(address);
            }

            CfgType::Isr => {
                debug_assert!(isr_addr.is_none());
                isr_addr = Some(address);
            }

            _ => unreachable!(),
        }
    }

    // A legacy-only (pre-virtio-1.0) device exposes none of the modern PCI
    // capabilities. Don't panic — report it and let the driver exit cleanly
    // (QEMU: such a device needs disable-legacy=off,disable-modern=off to be
    // transitional; pure-legacy is unsupported by this driver stack).
    if common_addr.is_none() {
        log::error!(
            "virtio-core: device {}:{} has no modern (virtio 1.0) PCI capabilities — \
             legacy-only devices are unsupported",
            pci_config.func.full_device_id.vendor_id,
            pci_config.func.full_device_id.device_id
        );
    }
    let common_addr = common_addr.ok_or(Error::InCapable(CfgType::Common))?;
    let device_addr = device_addr.ok_or(Error::InCapable(CfgType::Device))?;
    let (notify_addr, notify_multiplier) = notify_addr.ok_or(Error::InCapable(CfgType::Notify))?;

    // FIXME this is explicitly allowed by the virtio specification to happen
    assert!(
        notify_multiplier != 0,
        "virtio-core::device_probe: device uses the same Queue Notify addresses for all queues"
    );

    let common = unsafe { &mut *(common_addr as *mut CommonCfg) };
    let device_space = unsafe { &mut *(device_addr as *mut u8) };

    // Setup interrupts: MSI-X where the platform's interrupt controller supports
    // it (x86), otherwise legacy PCI INTx (aarch64/riscv64). See
    // `crate::arch::setup_interrupt`.
    let (irq_handle, interrupt_method) = crate::arch::setup_interrupt(pcid_handle)?;

    // The ISR status register is only needed in INTx mode, to de-assert the
    // level-triggered line on each interrupt. It is mapped above if present.
    let isr_addr = isr_addr.unwrap_or(0);
    if interrupt_method == InterruptMethod::Intx && isr_addr == 0 {
        log::warn!("virtio: INTx selected but the device exposes no ISR capability");
    }

    let transport = StandardTransport::new(
        common,
        notify_addr as *const u8,
        notify_multiplier,
        device_space,
        isr_addr as *const u8,
        interrupt_method,
    );

    log::debug!("virtio: using standard PCI transport ({interrupt_method:?})");

    let device = Device {
        transport,
        device_space,
        irq_handle,
    };

    device.transport.reset();
    reinit(&device)?;

    Ok(device)
}

pub fn reinit(device: &Device) -> Result<(), Error> {
    // XXX: According to the virtio specification v1.2, setting the ACKNOWLEDGE and DRIVER bits
    //      in `device_status` is required to be done in two steps.
    device
        .transport
        .insert_status(DeviceStatusFlags::ACKNOWLEDGE);

    device.transport.insert_status(DeviceStatusFlags::DRIVER);
    Ok(())
}
