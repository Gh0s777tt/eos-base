use std::fs::File;

use pcid_interface::{irq_helpers, PciFunctionHandle};

use crate::transport::{Error, InterruptMethod};

/// Set up the device's interrupt.
///
/// E-OS aarch64 has no usable MSI/MSI-X controller, so VirtIO devices fall back
/// to the legacy, level-triggered PCI INTx line. On non-x86 targets
/// `pci_allocate_interrupt_vector` always returns a Legacy vector (see
/// `pcid::irq_helpers`), matching how nvmed/xhcid bind on this platform.
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
