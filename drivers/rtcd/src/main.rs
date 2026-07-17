use anyhow::{Context, Result};

// TODO: Do not use target architecture to distinguish these.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;

/// The rtc driver runs only once, being perhaps the first of all processes that init starts (since
/// early logging benefits from knowing the time, even though this can be adjusted later once the
/// time is known). The sole job of `rtcd` is to read from the hardware real-time clock, and then
/// write the offset to the kernel.

fn main() -> Result<()> {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        common::acquire_port_io_rights().context("failed to set iopl")?;

        let time_s = self::x86::get_time();
        let time_ns = u128::from(time_s) * 1_000_000_000;

        std::fs::write("/scheme/sys/update_time_offset", &time_ns.to_ne_bytes())
            .context("failed to write to time offset")?;
    }
    // On aarch64 the kernel only programs the RTC on a Device-Tree boot (rtc::init,
    // via init_devicetree); on an ACPI/UEFI boot the clock stays at the Unix epoch
    // (1970), which breaks TLS certificate-validity checks. The bootloader reads the
    // firmware clock (UEFI GetTime) and passes it as BOOT_TIME=<unix_secs> in the
    // kernel env, so apply the same offset x86 derives from the CMOS RTC.
    #[cfg(target_arch = "aarch64")]
    {
        let env =
            std::fs::read_to_string("/scheme/sys/env").context("failed to read /scheme/sys/env")?;
        if let Some(time_s) = env.lines().find_map(|line| {
            line.strip_prefix("BOOT_TIME=")
                .and_then(|v| v.trim().parse::<u64>().ok())
        }) {
            let time_ns = u128::from(time_s) * 1_000_000_000;
            std::fs::write("/scheme/sys/update_time_offset", &time_ns.to_ne_bytes())
                .context("failed to write to time offset")?;
        } else {
            eprintln!("rtcd: no BOOT_TIME in kernel env; clock left at boot default");
        }
    }

    Ok(())
}
