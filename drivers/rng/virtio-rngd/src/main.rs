//! virtio-rng — VirtIO entropy (hardware RNG) driver.
//!
//! The device exposes a single virtqueue (the "requestq"). The driver hands it a
//! WRITE_ONLY buffer; the device fills it with random bytes and reports how many
//! it wrote via the used ring. This is a genuine entropy source — especially
//! valuable on aarch64 cores without FEAT_RNG, where `randd` otherwise leans on
//! kernel RNDR/RNDRRS emulation + CNTVCT jitter (see R-401b).

use common::dma::Dma;
use pcid_interface::*;
use virtio_core::spec::{Buffer, ChainBuilder, DescriptorFlags};

fn main() {
    pcid_interface::pci_daemon(daemon_runner);
}

fn daemon_runner(redox_daemon: daemon::Daemon, pcid_handle: PciFunctionHandle) -> ! {
    daemon(redox_daemon, pcid_handle).expect("virtio-rngd: failed to start");
    unreachable!();
}

fn daemon(daemon: daemon::Daemon, mut pcid_handle: PciFunctionHandle) -> anyhow::Result<()> {
    common::setup_logging(
        "rng",
        "pci",
        "virtio-rngd",
        common::output_level(),
        common::file_level(),
    );

    let pci_config = pcid_handle.config();
    log::info!("virtio-rng: initiating startup sequence :^)");

    let device = virtio_core::probe_device(&mut pcid_handle)?;
    device.transport.finalize_features();

    // virtio-rng has a single virtqueue: the requestq (queue index 0).
    let queue = device
        .transport
        .setup_queue(virtio_core::MSIX_PRIMARY_VECTOR, &device.irq_handle)?;

    // The device is alive from here on.
    device.transport.run_device();

    // Prove the entropy source works end-to-end before reporting readiness: hand
    // the device a write-only buffer and read back what it fills in.
    //
    // SAFETY: we only ever read up to `written` bytes (what the device reports it
    // produced), so the uninitialised tail of the buffer is never observed.
    let buffer = unsafe { Dma::<[u8]>::zeroed_slice(16)?.assume_init() };
    let chain = ChainBuilder::new()
        .chain(Buffer::new_unsized(&buffer).flags(DescriptorFlags::WRITE_ONLY))
        .build();
    let written = (futures::executor::block_on(queue.send(chain)) as usize).min(buffer.len());
    log::info!(
        "virtio-rng: {} ready; pulled {} bytes of entropy: {:02x?}",
        pci_config.func.display(),
        written,
        &buffer[..written],
    );

    // `Daemon::ready` returns `()` (it just writes the readiness byte), so there
    // is nothing to `.expect()` here.
    daemon.ready();

    libredox::call::setrens(0, 0).expect("virtio-rngd: failed to enter null namespace");

    // Phase 1: the device is bound and producing entropy. A follow-up will feed
    // this into `randd`'s pool (or expose a scheme) so the system consumes it.
    loop {
        std::thread::park();
    }
}
