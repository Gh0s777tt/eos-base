//! raid1d — E-OS R-501: userspace RAID-1 (mirror) block daemon.
//!
//! Members are whole disks carrying an E-OS RAID superblock in their last
//! 4 KiB. The daemon scans `/scheme/disk.*`, assembles the first array it
//! finds and exposes it as the `disk.raid1` scheme (via driver-block's
//! DiskScheme), where RedoxFS mounts it like any other disk. Holding the
//! member disks open also keeps other consumers away (the disk schemes
//! hand out whole-disk access exclusively).
//!
//! Subcommands: `raid1d create <diskA> <diskB>` initializes a mirror,
//! `raid1d status` prints superblocks; no arguments = daemon mode (used by
//! the `25_raid1d.service` init service; exits cleanly when no array).
//!
//! MVP scope (R-501): 2 members, write-both/read-primary-with-fallback,
//! degraded assembly with loud logs. Resync/rebuild is R-501b.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::time::{SystemTime, UNIX_EPOCH};

use driver_block::{Disk, DiskScheme, ExecutorTrait, TrivialExecutor};
use syscall::error::{Error, EIO, EINVAL};

const MAGIC: &[u8; 8] = b"EOSRAID1";
const SB_SIZE: u64 = 4096;
const SB_VERSION: u32 = 2;

#[derive(Clone, Debug)]
struct Superblock {
    array_uuid: [u8; 16],
    member_index: u32,
    generation: u64,
    usable_bytes: u64,
    block_size: u32,
    /// The generation at which this member was last part of a *complete*,
    /// consistent assembly (all members present and in sync). A member whose
    /// `generation > last_full_sync` advanced while a peer was absent; two such
    /// members at the same generation are a split-brain, not an in-sync mirror.
    last_full_sync: u64,
    /// Total members in the array (2 for a mirror). Lets assembly tell a
    /// complete set from a degraded subset.
    member_count: u32,
}

impl Superblock {
    fn to_bytes(&self) -> [u8; 64] {
        let mut b = [0u8; 64];
        b[0..8].copy_from_slice(MAGIC);
        b[8..12].copy_from_slice(&SB_VERSION.to_le_bytes());
        b[12..28].copy_from_slice(&self.array_uuid);
        b[28..32].copy_from_slice(&self.member_index.to_le_bytes());
        b[32..40].copy_from_slice(&self.generation.to_le_bytes());
        b[40..48].copy_from_slice(&self.usable_bytes.to_le_bytes());
        b[48..52].copy_from_slice(&self.block_size.to_le_bytes());
        b[52..60].copy_from_slice(&self.last_full_sync.to_le_bytes());
        b[60..64].copy_from_slice(&self.member_count.to_le_bytes());
        b
    }

    fn from_bytes(b: &[u8]) -> Option<Superblock> {
        if b.len() < 64 || &b[0..8] != MAGIC {
            return None;
        }
        if u32::from_le_bytes(b[8..12].try_into().ok()?) != SB_VERSION {
            return None;
        }
        Some(Superblock {
            array_uuid: b[12..28].try_into().ok()?,
            member_index: u32::from_le_bytes(b[28..32].try_into().ok()?),
            generation: u64::from_le_bytes(b[32..40].try_into().ok()?),
            usable_bytes: u64::from_le_bytes(b[40..48].try_into().ok()?),
            block_size: u32::from_le_bytes(b[48..52].try_into().ok()?),
            last_full_sync: u64::from_le_bytes(b[52..60].try_into().ok()?),
            member_count: u32::from_le_bytes(b[60..64].try_into().ok()?),
        })
    }
}

/// Superblock location: the last 4 KiB-aligned block of the device.
fn sb_offset(dev_size: u64) -> u64 {
    (dev_size / SB_SIZE) * SB_SIZE - SB_SIZE
}

fn read_superblock(file: &File) -> Option<(Superblock, u64)> {
    let size = file.metadata().ok()?.len();
    if size < 2 * SB_SIZE {
        return None;
    }
    let off = sb_offset(size);
    // disk schemes only accept block-multiple I/O, so read the whole 4 KiB
    let mut buf = vec![0u8; SB_SIZE as usize];
    file.read_exact_at(&mut buf, off).ok()?;
    Superblock::from_bytes(&buf).map(|sb| (sb, size))
}

fn write_superblock(file: &File, dev_size: u64, sb: &Superblock) -> std::io::Result<()> {
    let mut block = vec![0u8; SB_SIZE as usize];
    block[..64].copy_from_slice(&sb.to_bytes());
    file.write_all_at(&block, sb_offset(dev_size))
}

/// Whole-disk entries of every `disk.*` scheme except our own output.
fn scan_disk_paths() -> Vec<String> {
    let mut out = Vec::new();
    let Ok(schemes) = std::fs::read_dir("/scheme") else {
        return out;
    };
    for scheme in schemes.flatten() {
        let raw_name = scheme.file_name().to_string_lossy().to_string();
        let name = raw_name.trim();
        if !name.starts_with("disk.") || name == "disk.raid1" || name == "disk.live" {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(format!("/scheme/{}", name)) else {
            continue;
        };
        for entry in entries.flatten() {
            // scheme getdents may hand back names with a trailing newline
            let raw = entry.file_name().to_string_lossy().to_string();
            let e = raw.trim();
            if !e.is_empty() && !e.contains('p') {
                out.push(format!("/scheme/{}/{}", name, e));
            }
        }
    }
    out.sort();
    out
}

struct Member {
    path: String,
    file: File,
    sb: Superblock,
    dev_size: u64,
    active: bool,
}

struct Raid1Disk {
    members: Vec<Member>,
    primary: usize,
    usable_bytes: u64,
    block_size: u32,
}

impl Raid1Disk {
    fn byte_off(&self, block: u64, len: usize) -> syscall::Result<u64> {
        let off = block
            .checked_mul(self.block_size as u64)
            .ok_or(Error::new(EINVAL))?;
        if off + len as u64 > self.usable_bytes {
            return Err(Error::new(EINVAL));
        }
        Ok(off)
    }
}

impl Disk for Raid1Disk {
    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn size(&self) -> u64 {
        self.usable_bytes
    }

    async fn read(&mut self, block: u64, buffer: &mut [u8]) -> syscall::Result<usize> {
        let off = self.byte_off(block, buffer.len())?;
        let order = [self.primary, 1 - self.primary];
        for &i in &order {
            let Some(m) = self.members.get(i) else {
                continue;
            };
            if !m.active {
                continue;
            }
            match m.file.read_exact_at(buffer, off) {
                Ok(()) => return Ok(buffer.len()),
                Err(err) => {
                    eprintln!("raid1d: read error on {} block {}: {}", m.path, block, err);
                }
            }
        }
        Err(Error::new(EIO))
    }

    async fn write(&mut self, block: u64, buffer: &[u8]) -> syscall::Result<usize> {
        let off = self.byte_off(block, buffer.len())?;
        let before = self.members.iter().filter(|m| m.active).count();
        let mut ok = 0;
        for m in self.members.iter_mut() {
            if !m.active {
                continue;
            }
            match m.file.write_all_at(buffer, off) {
                Ok(()) => ok += 1,
                Err(err) => {
                    eprintln!(
                        "raid1d: WRITE FAILED on {} block {}: {} — dropping member, array DEGRADED",
                        m.path, block, err
                    );
                    m.active = false;
                }
            }
        }
        if ok == 0 {
            return Err(Error::new(EIO));
        }
        // R-501b: a member that drops out during operation must leave a
        // generation gap on the survivors, otherwise the next assembly sees
        // equal generations, skips the resync, and the stale disk silently
        // diverges. Bump the still-active members' on-disk generation once, the
        // moment the active set shrinks.
        if ok < before {
            let new_gen = self
                .members
                .iter()
                .filter(|m| m.active)
                .map(|m| m.sb.generation)
                .max()
                .unwrap_or(0)
                + 1;
            for m in self.members.iter_mut() {
                if m.active {
                    m.sb.generation = new_gen;
                    if let Err(err) = write_superblock(&m.file, m.dev_size, &m.sb) {
                        eprintln!("raid1d: degrade-bump superblock write failed on {}: {}", m.path, err);
                    }
                }
            }
        }
        Ok(buffer.len())
    }
}

fn make_uuid() -> [u8; 16] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9);
    let mut z = nanos ^ ((std::process::id() as u64) << 32) ^ 0xA076_1D64_78BD_642F;
    let mut out = [0u8; 16];
    for chunk in out.chunks_mut(8) {
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        chunk.copy_from_slice(&z.to_le_bytes());
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn cmd_create(a_path: &str, b_path: &str) -> Result<(), String> {
    let open = |p: &str| {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(p)
            .map_err(|e| format!("cannot open {}: {}", p, e))
    };
    let a = open(a_path)?;
    let b = open(b_path)?;
    let a_size = a.metadata().map_err(|e| e.to_string())?.len();
    let b_size = b.metadata().map_err(|e| e.to_string())?.len();
    if a_size < 2 * SB_SIZE || b_size < 2 * SB_SIZE {
        return Err("member too small".into());
    }
    let usable = sb_offset(a_size).min(sb_offset(b_size));
    let sb = Superblock {
        array_uuid: make_uuid(),
        member_index: 0,
        generation: 1,
        usable_bytes: usable,
        block_size: 512,
        last_full_sync: 1,
        member_count: 2,
    };
    // wipe stale filesystem/partition signatures at the start of the array
    let zero = vec![0u8; SB_SIZE as usize];
    for (file, size, index, path) in [(&a, a_size, 0u32, a_path), (&b, b_size, 1u32, b_path)] {
        file.write_all_at(&zero, 0)
            .map_err(|e| format!("wipe {}: {}", path, e))?;
        let mut msb = sb.clone();
        msb.member_index = index;
        write_superblock(file, size, &msb).map_err(|e| format!("superblock {}: {}", path, e))?;
    }
    println!(
        "raid1d: created mirror {} on {} + {} ({} MiB usable)",
        hex(&sb.array_uuid),
        a_path,
        b_path,
        usable / (1024 * 1024)
    );
    println!("raid1d: start the daemon (or reboot) to assemble disk.raid1");
    Ok(())
}

const STATE_PATH: &str = "/tmp/raid1d.state";

fn cmd_status() {
    // The running daemon holds its members open exclusively, so a bare disk
    // scan would hit ENOLCK. Prefer the state file the daemon writes at
    // assembly; fall back to scanning when no daemon is running.
    if let Ok(state) = std::fs::read_to_string(STATE_PATH) {
        print!("{}", state);
        return;
    }
    let mut found = false;
    for path in scan_disk_paths() {
        let Ok(file) = File::open(&path) else {
            continue;
        };
        if let Some((sb, _)) = read_superblock(&file) {
            found = true;
            println!(
                "{}: array {} member {} generation {} usable {} MiB",
                path,
                hex(&sb.array_uuid),
                sb.member_index,
                sb.generation,
                sb.usable_bytes / (1024 * 1024)
            );
        }
    }
    if !found {
        println!("raid1d: no RAID members found (no daemon, no members); scanned:");
        for path in scan_disk_paths() {
            println!("  {}", path);
        }
    }
}

/// Copy the whole data region [0, usable_bytes) from `src` to `dst`. The RAID
/// superblock lives past `usable_bytes` (last 4 KiB of the device) and is left
/// untouched, so each member keeps its own member index.
fn resync_copy(src: &File, dst: &File, usable_bytes: u64) -> std::io::Result<()> {
    const CHUNK: usize = 1024 * 1024;
    let mut buf = vec![0u8; CHUNK];
    let mut off = 0u64;
    let mut last_pct = 0u64;
    while off < usable_bytes {
        let n = ((usable_bytes - off) as usize).min(CHUNK);
        src.read_exact_at(&mut buf[..n], off)?;
        dst.write_all_at(&buf[..n], off)?;
        off += n as u64;
        let pct = off * 100 / usable_bytes;
        if pct >= last_pct + 25 {
            last_pct = pct;
            eprintln!("raid1d:   resync {}%", pct);
        }
    }
    Ok(())
}

/// Write a human-readable state file so `raid1d status` can report the array
/// without opening the (exclusively held) member disks.
fn write_state(members: &[Member], usable_bytes: u64, block_size: u32) {
    let active = members.iter().filter(|m| m.active).count();
    let member_count = members.iter().map(|m| m.sb.member_count).max().unwrap_or(0) as usize;
    // NOTE: this is an assembly-time snapshot. The daemon enters a null namespace
    // after startup and can no longer touch the filesystem, so a member dropping
    // out during operation is not reflected here until the next assembly; the
    // on-disk generation bump in write() is what preserves data integrity.
    let status = if active == member_count && active >= 2 {
        "optimal"
    } else {
        "degraded"
    };
    let mut s = format!(
        "array = {}\nusable_mib = {}\nblock_size = {}\nstatus = {}\nmembers = {}/{}\n",
        hex(&members[0].sb.array_uuid),
        usable_bytes / (1024 * 1024),
        block_size,
        status,
        active,
        member_count,
    );
    for m in members {
        s.push_str(&format!(
            "member {} = {} (generation {}, {})\n",
            m.sb.member_index,
            if m.active { "active" } else { "excluded" },
            m.sb.generation,
            m.path,
        ));
    }
    let _ = std::fs::write(STATE_PATH, s);
}

fn assemble() -> Option<Raid1Disk> {
    let mut members: Vec<Member> = Vec::new();
    for path in scan_disk_paths() {
        let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
            continue;
        };
        let Some((sb, dev_size)) = read_superblock(&file) else {
            continue;
        };
        if let Some(first) = members.first() {
            if first.sb.array_uuid != sb.array_uuid {
                eprintln!(
                    "raid1d: ignoring {} (different array {})",
                    path,
                    hex(&sb.array_uuid)
                );
                continue;
            }
        }
        members.push(Member {
            path,
            file,
            sb,
            dev_size,
            active: true,
        });
    }
    if members.is_empty() {
        return None;
    }

    // Pick the authoritative source and the set of members to rebuild.
    //
    // `generation` alone can't tell "in sync at gen N" from "two members each
    // advanced to gen N while the other was absent" (split-brain). We use
    // `last_full_sync`: a member with generation > last_full_sync advanced while
    // degraded. If two members share the newest generation and at least one
    // advanced past its last full sync, they diverged independently — that's a
    // split-brain, not a healthy mirror.
    let newest = members.iter().map(|m| m.sb.generation).max().unwrap();
    let member_count = members.iter().map(|m| m.sb.member_count).max().unwrap();
    let newest_members: Vec<usize> = members
        .iter()
        .enumerate()
        .filter(|(_, m)| m.sb.generation == newest)
        .map(|(i, _)| i)
        .collect();
    let diverged = newest_members
        .iter()
        .any(|&i| members[i].sb.generation > members[i].sb.last_full_sync);
    let split_brain = newest_members.len() >= 2 && diverged;

    // Authoritative source = lowest member_index among the newest members.
    // Deterministic; in a split-brain this discards the losers' unsynced writes
    // to restore a consistent mirror (a production build would refuse and expose
    // `raid1d resolve`). All other members are rebuilt from it.
    let src_i = *newest_members
        .iter()
        .min_by_key(|&&i| members[i].sb.member_index)
        .unwrap();
    let usable = members[src_i].sb.usable_bytes;
    let src_block_size = members[src_i].sb.block_size;

    if split_brain {
        eprintln!(
            "raid1d: !!! SPLIT-BRAIN in array {} — {} members advanced independently to \
             generation {}; forcing {} authoritative and rebuilding the rest (unsynced \
             writes on the others are LOST)",
            hex(&members[0].sb.array_uuid),
            newest_members.len(),
            newest,
            members[src_i].path
        );
    }

    let rebuild: Vec<usize> = (0..members.len())
        .filter(|&i| i != src_i)
        .filter(|&i| members[i].sb.generation < newest || split_brain)
        .collect();
    for i in rebuild {
        let reason = if members[i].sb.generation < newest {
            format!("STALE (generation {} < {})", members[i].sb.generation, newest)
        } else {
            "SPLIT-BRAIN loser".to_string()
        };
        eprintln!(
            "raid1d: member {} is {} — rebuilding from {}",
            members[i].path, reason, members[src_i].path
        );
        match resync_copy(&members[src_i].file, &members[i].file, usable) {
            Ok(()) => {
                members[i].sb.generation = newest;
                members[i].sb.usable_bytes = usable;
                members[i].sb.block_size = src_block_size;
                members[i].sb.last_full_sync = members[src_i].sb.last_full_sync;
                if let Err(err) = write_superblock(&members[i].file, members[i].dev_size, &members[i].sb) {
                    eprintln!(
                        "raid1d: rebuild superblock write failed on {}: {} — excluded",
                        members[i].path, err
                    );
                    members[i].active = false;
                } else {
                    eprintln!("raid1d: rebuild of {} complete", members[i].path);
                    members[i].active = true;
                }
            }
            Err(err) => {
                eprintln!(
                    "raid1d: rebuild FAILED on {}: {} — staying degraded",
                    members[i].path, err
                );
                members[i].active = false;
            }
        }
    }

    let active = members.iter().filter(|m| m.active).count();
    // A "complete" assembly has every array member present and consistent; only
    // then can we advance last_full_sync (the clean-sync marker).
    let complete = members.len() as u32 == member_count && active == members.len();
    if !complete {
        eprintln!(
            "raid1d: array {} running DEGRADED ({} of {} member(s) active)",
            hex(&members[0].sb.array_uuid),
            active,
            member_count
        );
    }

    // Bump generation on active members so a member dropping out next is
    // detectable as stale. On a complete assembly also advance last_full_sync,
    // marking this generation as a clean common sync point.
    let new_gen = newest + 1;
    for m in members.iter_mut() {
        if m.active {
            m.sb.generation = new_gen;
            if complete {
                m.sb.last_full_sync = new_gen;
            }
            if let Err(err) = write_superblock(&m.file, m.dev_size, &m.sb) {
                eprintln!("raid1d: generation bump failed on {}: {}", m.path, err);
            }
        }
    }

    members.sort_by_key(|m| m.sb.member_index);
    let primary = members.iter().position(|m| m.active).unwrap_or(0);
    // authoritative geometry comes from the newest member, not member[0] (which
    // may be an excluded stale disk still carrying its old usable_bytes)
    let usable_bytes = usable;
    let block_size = src_block_size;
    println!(
        "raid1d: assembled array {} ({} MiB, {} active member(s), primary {})",
        hex(&members[0].sb.array_uuid),
        usable_bytes / (1024 * 1024),
        active,
        members[primary].path
    );
    write_state(&members, usable_bytes, block_size);
    Some(Raid1Disk {
        members,
        primary,
        usable_bytes,
        block_size,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("create") => {
            if args.len() != 3 {
                eprintln!("usage: raid1d create <diskA> <diskB>");
                std::process::exit(1);
            }
            if let Err(err) = cmd_create(&args[1], &args[2]) {
                eprintln!("raid1d: create failed: {}", err);
                std::process::exit(1);
            }
        }
        Some("status") => cmd_status(),
        Some(other) => {
            eprintln!("raid1d: unknown command {:?} (create|status|<none>)", other);
            std::process::exit(1);
        }
        None => {
            daemon::Daemon::new(daemon_main);
        }
    }
}

fn daemon_main(daemon: daemon::Daemon) -> ! {
    let Some(disk) = assemble() else {
        // No array on this machine: report readiness and bow out quietly,
        // exactly like lived does without a live disk.
        daemon.ready();
        std::process::exit(0);
    };

    let event_queue = event::EventQueue::new().unwrap();

    event::user_data! {
        enum Event {
            Scheme,
        }
    };

    let mut scheme = DiskScheme::new(
        Some(daemon),
        "disk.raid1".to_owned(),
        BTreeMap::from([(0, disk)]),
        &TrivialExecutor,
    );

    libredox::call::setrens(0, 0).expect("raid1d: failed to enter null namespace");

    event_queue
        .subscribe(
            scheme.event_handle().raw(),
            Event::Scheme,
            event::EventFlags::READ,
        )
        .unwrap();

    for event in event_queue {
        match event.unwrap().user_data {
            Event::Scheme => TrivialExecutor.block_on(scheme.tick()).unwrap(),
        }
    }

    std::process::exit(0);
}
