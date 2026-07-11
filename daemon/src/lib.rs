//! A library for creating and managing daemons for RedoxOS.
#![feature(never_type)]

use std::io::{self, PipeWriter, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::Command;

use libredox::Fd;
use redox_scheme::Socket;
use redox_scheme::scheme::{SchemeAsync, SchemeSync};

unsafe fn get_fd(var: &str) -> Option<RawFd> {
    let fd: RawFd = std::env::var(var)
        .unwrap_or_else(|_| {
            panic!(
                "Daemons can't be started manually. \
            Add a service config to make init start this daemon instead."
            )
        })
        .parse()
        .unwrap();
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        // E-OS: a daemon spawned by *another* daemon (e.g. a USB class driver spawned by
        // xhcid) rather than by init inherits the parent's INIT_NOTIFY env var but not a
        // valid notify-pipe fd -- the parent's fd is CLOEXEC / already consumed -- so
        // fcntl fails with EBADF. Treat that as "no readiness pipe" (skip the notification)
        // instead of aborting the whole driver, which is why usbscsid used to crash.
        if io::Error::last_os_error().raw_os_error() == Some(libc::EBADF) {
            return None;
        }
        panic!(
            "daemon: failed to set CLOEXEC flag for {var} fd: {}",
            io::Error::last_os_error()
        );
    }
    Some(fd)
}

unsafe fn pass_fd(cmd: &mut Command, env: &str, fd: RawFd) {
    cmd.env(env, format!("{}", fd));
    unsafe {
        cmd.pre_exec(move || {
            // Pass notify pipe to child
            if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

/// A long running background process that handles requests.
#[must_use = "Daemon::ready must be called"]
pub struct Daemon {
    write_pipe: Option<PipeWriter>,
}

impl Daemon {
    /// Create a new daemon.
    pub fn new(f: impl FnOnce(Self) -> !) -> ! {
        let write_pipe =
            unsafe { get_fd("INIT_NOTIFY").map(|fd| io::PipeWriter::from_raw_fd(fd)) };

        f(Daemon { write_pipe })
    }

    /// Notify the process that the daemon is ready to accept requests.
    pub fn ready(mut self) {
        if let Some(pipe) = self.write_pipe.as_mut() {
            let _ = pipe.write_all(&[0]);
        }
    }

    /// Executes `Command` as a child process.
    // FIXME remove once the service spawning of hwd and pcid-spawner is moved to init
    #[deprecated]
    pub fn spawn(mut cmd: Command) {
        let (mut read_pipe, write_pipe) = io::pipe().unwrap();

        unsafe { pass_fd(&mut cmd, "INIT_NOTIFY", write_pipe.as_raw_fd()) };

        if let Err(err) = cmd.spawn() {
            eprintln!("daemon: failed to execute {cmd:?}: {err}");
            return;
        }

        drop(write_pipe);
        let mut data = [0];
        match read_pipe.read_exact(&mut data) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => {
                eprintln!("daemon: {cmd:?} exited without notifying readiness");
            }
            Err(err) => {
                eprintln!("daemon: failed to wait for {cmd:?}: {err}");
            }
        }
    }
}

/// A long running background process that handles requests using schemes.
#[must_use = "SchemeDaemon::ready must be called"]
pub struct SchemeDaemon {
    write_pipe: Option<PipeWriter>,
}

impl SchemeDaemon {
    /// Create a new daemon for use with schemes.
    #[expect(clippy::new_ret_no_self)]
    pub fn new(f: impl FnOnce(SchemeDaemon) -> !) -> ! {
        let write_pipe =
            unsafe { get_fd("INIT_NOTIFY").map(|fd| io::PipeWriter::from_raw_fd(fd)) };

        f(SchemeDaemon { write_pipe })
    }

    /// Notify the process that the scheme daemon is ready to accept requests.
    pub fn ready_with_fd(self, cap_fd: Fd) -> syscall::Result<()> {
        let Some(write_pipe) = self.write_pipe.as_ref() else {
            // No init notify pipe (spawned as a subdriver) -- nothing to notify.
            return Ok(());
        };
        libredox::call::call_wo(
            write_pipe.as_raw_fd() as usize,
            &cap_fd.into_raw().to_ne_bytes(),
            syscall::CallFlags::FD,
            &[],
        )?;
        Ok(())
    }

    /// Notify the process that the synchronous scheme daemon is ready to accept requests.
    pub fn ready_sync_scheme<S: SchemeSync>(
        self,
        socket: &Socket,
        scheme: &mut S,
    ) -> syscall::Result<()> {
        let cap_id = scheme.scheme_root()?;
        let cap_fd = socket.create_this_scheme_fd(0, cap_id, 0, 0)?;
        self.ready_with_fd(Fd::new(cap_fd))
    }

    /// Notify the process that the asynchronous scheme daemon is ready to accept requests.
    pub fn ready_async_scheme<S: SchemeAsync>(
        self,
        socket: &Socket,
        scheme: &mut S,
    ) -> syscall::Result<()> {
        let cap_id = scheme.scheme_root()?;
        let cap_fd = socket.create_this_scheme_fd(0, cap_id, 0, 0)?;
        self.ready_with_fd(Fd::new(cap_fd))
    }
}
