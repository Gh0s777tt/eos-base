#![feature(never_type)]

use std::io::{self, PipeWriter, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::process::Command;

use libredox::Fd;
use redox_scheme::Socket;
use redox_scheme::scheme::{SchemeAsync, SchemeSync};

unsafe fn get_fd(var: &str) -> RawFd {
    let fd: RawFd = std::env::var(var).unwrap().parse().unwrap();
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        panic!(
            "daemon: failed to set CLOEXEC flag for {var} fd: {}",
            io::Error::last_os_error()
        );
    }
    fd
}

#[must_use = "Daemon::ready must be called"]
pub struct Daemon {
    write_pipe: PipeWriter,
}

impl Daemon {
    pub fn new(f: impl FnOnce(Daemon) -> !) -> ! {
        let write_pipe = unsafe { io::PipeWriter::from_raw_fd(get_fd("INIT_NOTIFY")) };

        f(Daemon { write_pipe })
    }

    pub fn ready(mut self) {
        self.write_pipe.write_all(&[0]).unwrap();
    }

    pub fn spawn(mut cmd: Command) {
        let (mut read_pipe, write_pipe) = io::pipe().unwrap();

        // Pass notify pipe to child
        if unsafe { libc::fcntl(write_pipe.as_raw_fd(), libc::F_SETFD, 0) } == -1 {
            eprintln!(
                "daemon: failed to unset CLOEXEC flag for notify pipe: {}",
                io::Error::last_os_error()
            );
            return;
        }
        cmd.env("INIT_NOTIFY", format!("{}", write_pipe.as_raw_fd()));

        if let Err(err) = cmd.spawn() {
            eprintln!("daemon: failed to execute {cmd:?}: {err}");
            return;
        }
        drop(write_pipe);

        let mut data = [0];
        match read_pipe.read_exact(&mut data) {
            Ok(()) => {
                if data[0] != 0 {
                    eprintln!("daemon: {cmd:?} failed with {}", data[0]);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => {
                eprintln!("daemon: {cmd:?} exited without notifying readiness");
            }
            Err(err) => {
                eprintln!("daemon: failed to wait for {cmd:?}: {err}");
            }
        }
    }
}

#[must_use = "SchemeDaemon::ready must be called"]
pub struct SchemeDaemon {
    write_pipe: PipeWriter,
}

impl SchemeDaemon {
    pub fn new(f: impl FnOnce(SchemeDaemon) -> !) -> ! {
        let write_pipe = unsafe { io::PipeWriter::from_raw_fd(get_fd("INIT_NOTIFY")) };

        f(SchemeDaemon { write_pipe })
    }

    pub fn ready_with_fd(self, cap_fd: Fd) -> syscall::Result<()> {
        syscall::call_wo(
            self.write_pipe.as_raw_fd() as usize,
            &cap_fd.into_raw().to_ne_bytes(),
            syscall::CallFlags::FD,
            &[],
        )?;
        Ok(())
    }

    pub fn ready_sync_scheme<S: SchemeSync>(
        self,
        socket: &Socket,
        scheme: &mut S,
    ) -> syscall::Result<()> {
        let cap_id = scheme.scheme_root()?;
        let cap_fd = socket.create_this_scheme_fd(0, cap_id, 0, 0)?;
        self.ready_with_fd(Fd::new(cap_fd))
    }

    pub fn ready_async_scheme<S: SchemeAsync>(
        self,
        socket: &Socket,
        scheme: &mut S,
    ) -> syscall::Result<()> {
        let cap_id = scheme.scheme_root()?;
        let cap_fd = socket.create_this_scheme_fd(0, cap_id, 0, 0)?;
        self.ready_with_fd(Fd::new(cap_fd))
    }

    pub fn spawn(mut cmd: Command, scheme_name: &str) {
        let (read_pipe, write_pipe) = io::pipe().unwrap();

        // Pass notify pipe to child
        if unsafe { libc::fcntl(write_pipe.as_raw_fd(), libc::F_SETFD, 0) } == -1 {
            eprintln!(
                "daemon: failed to unset CLOEXEC flag for notify pipe: {}",
                io::Error::last_os_error()
            );
            return;
        }
        cmd.env("INIT_NOTIFY", format!("{}", write_pipe.as_raw_fd()));

        if let Err(err) = cmd.spawn() {
            eprintln!("daemon: failed to execute {cmd:?}: {err}");
            return;
        }
        drop(write_pipe);

        let mut new_fd = usize::MAX;
        let fd_bytes = unsafe {
            core::slice::from_raw_parts_mut(
                core::slice::from_mut(&mut new_fd).as_mut_ptr() as *mut u8,
                core::mem::size_of::<usize>(),
            )
        };
        loop {
            match syscall::call_ro(
                read_pipe.as_raw_fd() as usize,
                fd_bytes,
                syscall::CallFlags::FD | syscall::CallFlags::FD_UPPER,
                &[],
            ) {
                Err(syscall::Error {
                    errno: syscall::EINTR,
                }) => continue,
                Ok(0) => {
                    eprintln!("daemon: {cmd:?} exited without notifying readiness");
                    return;
                }
                Ok(1) => break,
                Ok(n) => {
                    eprintln!("daemon: incorrect amount of fds {n} returned");
                    return;
                }
                Err(err) => {
                    eprintln!("daemon: failed to wait for {cmd:?}: {err}");
                    return;
                }
            }
        }

        let current_namespace_fd = libredox::call::getns().expect("TODO");
        libredox::call::register_scheme_to_ns(current_namespace_fd, scheme_name, new_fd)
            .expect("TODO");
    }
}
