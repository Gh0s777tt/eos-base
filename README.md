<a name="top"></a>
<p align="center">
  <img src="https://capsule-render.vercel.app/api?type=waving&color=0:E50914,100:0B0B0B&height=200&section=header&text=eos-base&fontSize=80&fontColor=ffffff&fontAlignY=38&desc=Fundamental%20system%20daemons%20for%20E-OS&descAlignY=60&descSize=18&animation=fadeIn" alt="eos-base"/>
</p>

<p align="center">
  <img src="https://readme-typing-svg.demolab.com?font=Fira+Code&weight=600&size=19&pause=900&color=E50914&center=true&vCenter=true&width=800&lines=Fundamental+system+daemons+for+E-OS;Downstream+of+redox-os%2Fbase;aarch64+nvmed+INTx+fix+%28R-401c%29" alt="tagline"/>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT-E50914?style=for-the-badge&labelColor=0B0B0B" alt="license"/>
  <img src="https://img.shields.io/badge/Rust-E50914?style=for-the-badge&logo=rust&logoColor=white&labelColor=0B0B0B" alt="Rust"/>
  <img src="https://img.shields.io/badge/part%20of-E--OS-E50914?style=for-the-badge&labelColor=0B0B0B" alt="part of E-OS"/>
</p>

<p align="center"><img src="https://raw.githubusercontent.com/Gh0s777tt/Gh0s777tt/main/assets/divider.svg" width="100%" alt=""/></p>

Repository containing various system daemons, that are considered fundamental for the OS.

You can see what each component does in the following list:

- audiod : Daemon used to process the sound drivers audio
- bootstrap : First code that the kernel executes, responsible for spawning the init daemon
- daemon : Redox daemon library
- drivers
- init : Daemon used to start most system components and programs
- initfs : Filesystem with the necessary system components to run RedoxFS
- ipcd : Daemon used for inter-process communication
- logd : Daemon used to log system components and daemons
- netstack : Daemon used for networking
- ptyd : Daemon used for pseudo-terminal
- ramfs : RAM filesystem
- randd : Daemon used for random number generation
- zerod : Daemon used to discard all writes and fill read buffers with zero

## How To Contribute

To learn how to contribute you need to read the following document:

- [CONTRIBUTING.md](https://gitlab.redox-os.org/redox-os/redox/-/blob/master/CONTRIBUTING.md)

If you want to contribute to drivers read its [README](drivers/README.md)

## Development

To learn how to do development with these system components inside the Redox build system you need to read the [Build System](https://doc.redox-os.org/book/build-system-reference.html) and [Coding and Building](https://doc.redox-os.org/book/coding-and-building.html) pages.

### How To Build

It is recommended to build this system component via the Redox build system, you can learn how to do it on the [Building Redox](https://doc.redox-os.org/book/podman-build.html) page.

To build and test outside the build system, [install redoxer](https://doc.redox-os.org/book/ci.html) then use `check.sh` script to build or test:
- `./check.sh` - Check build for x86_64
- `./check.sh --arch=ARCH` - Check build for specific ARCH (`aarch64`, `i586`, `riscv64gc`)
- `./check.sh --all` - Check build for all ARCH
- `./check.sh --test` - Check the base system boots up on x86_64

You can also use `make install` to inspect the content on `./sysroot`, or `make test-gui` to test booting with orbital interactively.

<p align="center"><img src="https://raw.githubusercontent.com/Gh0s777tt/Gh0s777tt/main/assets/divider.svg" width="100%" alt=""/></p>

<div align="center">

### 🩸 Part of the **GHOST EMPIRE** ecosystem

[**E-OS**](https://github.com/Gh0s777tt/E-OS) · Rust microkernel OS &nbsp;·&nbsp; Minecraft infrastructure suite &nbsp;·&nbsp; Discord &amp; streaming platforms — forged under **Empire Forge**.

<a href="https://discord.gg/Egf88V9UdH"><img src="https://img.shields.io/badge/Discord-Join%20the%20Empire-5865F2?style=for-the-badge&logo=discord&logoColor=white&labelColor=0B0B0B" alt="discord"/></a>
<a href="mailto:ghostt77@empire-forge.com"><img src="https://img.shields.io/badge/Email-Empire%20Forge-E50914?style=for-the-badge&logo=maildotru&logoColor=white&labelColor=0B0B0B" alt="email"/></a>
<a href="https://donatr.ee/ghost77/"><img src="https://img.shields.io/badge/%E2%9D%A4%20Support-Donate-E50914?style=for-the-badge&labelColor=0B0B0B" alt="donate"/></a>

<sub><i>Black. Red. Production-grade. — © GHOST EMPIRE · Empire Forge</i></sub>

</div>
