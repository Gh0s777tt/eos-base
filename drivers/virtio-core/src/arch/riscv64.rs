use std::fs::File;

use pcid_interface::{irq_helpers, PciFunctionHandle};

use crate::transport::{Error, InterruptMethod};

/// Set up the device's interrupt. Like aarch64, E-OS riscv64 uses the legacy,
/// level-triggered PCI INTx line rather than MSI/MSI-X.
pub fn setup_interrupt(
    pcid_handle: &mut PciFunctionHandle,
) -> Result<(File, InterruptMethod), Error> {
    let vector = irq_helpers::pci_allocate_interrupt_vector(pcid_handle, "virtio");
    let irq_handle = vector
        .irq_handle()
        .try_clone()
        .expect("virtio-core: failed to clone INTx IRQ handle");
    Ok((irq_handle, InterruptMethod::Intx))
}
