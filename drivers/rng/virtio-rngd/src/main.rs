//! virtio-rng — VirtIO entropy (hardware RNG) driver.
//!
//! The device exposes a single virtqueue (the "requestq"). The driver hands it a
//! WRITE_ONLY buffer; the device fills it with random bytes and reports how many
//! it wrote via the used ring.
//!
//! The entropy is not just logged — it is fed into `randd`'s pool by writing to
//! `/scheme/rand` (the backing scheme for `/dev/{u,}random`). randd mixes each
//! write with 512 bits of its own CSPRNG output and reseeds its ChaCha20 state
//! (the standard "write to /dev/urandom" model), so this is a genuine entropy
//! source for the whole system. It is especially valuable on aarch64 cores
//! without FEAT_RNG, where randd otherwise seeds from a zero (INSECURE) seed.

use std::time::Duration;

use common::dma::Dma;
use pcid_interface::*;
use virtio_core::spec::{Buffer, ChainBuilder, DescriptorFlags};

/// Bytes pulled from the device per request (256 bits, matching randd's seed size).
const ENTROPY_BYTES: usize = 32;
/// How often to top up randd's pool with fresh hardware entropy.
const REFEED_SECS: u64 = 30;
/// The backing scheme for `/dev/random` and `/dev/urandom`.
const RAND_SCHEME: &str = "/scheme/rand";

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

    // One reusable DMA buffer; the device refills it with fresh entropy each time
    // we put a request on the queue.
    //
    // SAFETY: we only ever read up to `pull()`'s return value (what the device
    // reports it produced), so the uninitialised tail is never observed.
    let buffer = unsafe { Dma::<[u8]>::zeroed_slice(ENTROPY_BYTES)?.assume_init() };

    // Put one WRITE_ONLY request on the virtqueue and block until the device has
    // filled the buffer; returns how many bytes it produced.
    let pull = || -> usize {
        let chain = ChainBuilder::new()
            .chain(Buffer::new_unsized(&buffer).flags(DescriptorFlags::WRITE_ONLY))
            .build();
        (futures::executor::block_on(queue.send(chain)) as usize).min(buffer.len())
    };

    // Prove the source works end-to-end before reporting readiness.
    let n = pull();
    log::info!(
        "virtio-rng: {} ready; pulled {} bytes of entropy: {:02x?}",
        pci_config.func.display(),
        n,
        &buffer[..n],
    );

    // Open randd's scheme (write-only) so we can mix hardware entropy into its
    // pool. This MUST happen before setrens(0, 0): afterwards we are confined to
    // the null namespace and cannot open new schemes, but an already-open handle
    // keeps working.
    let rand = std::fs::OpenOptions::new().write(true).open(RAND_SCHEME);

    daemon.ready();

    libredox::call::setrens(0, 0).expect("virtio-rngd: failed to enter null namespace");

    let mut rand = match rand {
        Ok(file) => file,
        Err(err) => {
            // Not fatal: the device is still bound, we just can't feed the system
            // pool. Park so the binding persists rather than exiting.
            log::warn!("virtio-rng: could not open {RAND_SCHEME} ({err}); not feeding the entropy pool");
            loop {
                std::thread::park();
            }
        }
    };

    // Seed randd once now, then keep topping up with fresh hardware entropy.
    feed(&mut rand, &buffer[..n]);
    log::info!(
        "virtio-rng: seeded {RAND_SCHEME} with {} bytes; topping up every {}s",
        n,
        REFEED_SECS,
    );
    loop {
        std::thread::sleep(Duration::from_secs(REFEED_SECS));
        let n = pull();
        feed(&mut rand, &buffer[..n]);
    }
}

/// Write entropy into randd's pool, logging (but never failing the driver) on error.
fn feed(rand: &mut std::fs::File, entropy: &[u8]) {
    use std::io::Write;
    match rand.write_all(entropy) {
        Ok(()) => log::debug!(
            "virtio-rng: reseeded the entropy pool with {} bytes",
            entropy.len()
        ),
        Err(err) => log::warn!("virtio-rng: failed to reseed the entropy pool: {err}"),
    }
}
