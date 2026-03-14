use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::io::AsRawFd;

use drm::control::connector::{self, State};
use drm::control::Device as _;
use drm::{Device as _, DriverCapability};
use syscall::CallFlags;

pub use crate::common::Damage;

/// A graphics handle using the v2 graphics API.
///
/// The v2 graphics API allows creating framebuffers on the fly, using them for page flipping and
/// handles all displays using a single fd. This is basically a subset of the Linux DRM interface
/// with a couple of custom ioctls in the place of the KMS ioctls that are missing.
pub struct V2GraphicsHandle {
    file: File,
}

impl AsFd for V2GraphicsHandle {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl drm::Device for V2GraphicsHandle {}
impl drm::control::Device for V2GraphicsHandle {}

impl V2GraphicsHandle {
    pub fn from_file(file: File) -> io::Result<Self> {
        let handle = V2GraphicsHandle { file };
        assert!(handle.get_driver_capability(DriverCapability::DumbBuffer)? == 1);
        Ok(handle)
    }

    pub fn first_display(&self) -> io::Result<connector::Handle> {
        for &connector in self.resource_handles().unwrap().connectors() {
            if self.get_connector(connector, true)?.state() == State::Connected {
                return Ok(connector);
            }
        }
        Err(io::Error::other("no connected display"))
    }

    pub fn update_plane(&self, display_id: usize, fb_id: u32, damage: Damage) -> io::Result<()> {
        let cmd = ipc::UpdatePlane {
            display_id,
            fb_id,
            damage,
        };
        libredox::call::call_wo(
            self.file.as_raw_fd() as usize,
            unsafe { plain::as_bytes(&cmd) },
            CallFlags::empty(),
            &[ipc::UPDATE_PLANE, 0, 0],
        )?;
        Ok(())
    }
}

pub mod ipc {
    use crate::common::Damage;

    pub use redox_ioctl::drm::*;

    // FIXME replace these with proper drm interfaces and update orbital
    pub const UPDATE_PLANE: u64 = 0x12345670;
    #[repr(C, packed)]
    pub struct UpdatePlane {
        pub display_id: usize,
        pub fb_id: u32,
        pub damage: Damage,
    }
}
