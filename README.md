# eos-base

**E-OS fork of [`redox-os/base`](https://gitlab.redox-os.org/redox-os/base).** Part of the [**E-OS**](https://github.com/Gh0s777tt/E-OS) ecosystem — a hardened, Crimson-branded downstream of [Redox OS](https://www.redox-os.org).

This repository is the Redox **base system** — device drivers, `init`, and the core userspace daemons.

## E-OS changes vs upstream

- **R-501 `raid1d`** — a userspace RAID-1 mirror block daemon (write-both / read-fallback / resync), with untrusted-superblock geometry validation (K-01).
- **`usbnetd`** — a USB-RNDIS (USB-Ethernet) network driver; re-enabled **`usbscsid`** (USB mass storage).
- **aarch64 platform** — PCIe **INTx** routing in `pcid` / `virtio-core`, `xhcid` non-blocking bulk-IN, `randd` RNDRRS entropy.
- **Hardening** — release overflow-checks across drivers/daemons; `vesad` env-parse fix (R-F09).

## How it's pinned

The E-OS build pins this fork in [`recipes/core/base/recipe.toml`](https://github.com/Gh0s777tt/E-OS/blob/main/recipes/core/base/recipe.toml):

- branch **`eos-july`** · rev **`98f22879f808`**
- **5 commit(s) behind** upstream master

## Build standalone

This fork is normally built by the E-OS cookbook (`make CI=1 …` in the [main repo](https://github.com/Gh0s777tt/E-OS)). To build it on its own you need the Redox toolchain; see the main repo's [build guide](https://github.com/Gh0s777tt/E-OS/blob/main/docs/building.md).

## Hosting

**GitLab (source of truth):** https://gitlab.com/e-os/eos-base  
**GitHub (read-only mirror):** https://github.com/Gh0s777tt/eos-base

## License

MIT (inherited from upstream Redox). The E-OS project as a whole is AGPL-3.0; see the [main repo](https://github.com/Gh0s777tt/E-OS/blob/main/LICENSE).

---
[E-OS main repo](https://github.com/Gh0s777tt/E-OS) · [Docs](https://github.com/Gh0s777tt/E-OS/tree/main/docs) · [Upstream](https://gitlab.redox-os.org/redox-os/base)
