use std::fs::File;
use std::ops::Range;
use std::os::fd::{AsFd, BorrowedFd};
use std::{io, mem, ptr};

use drm::control::connector::{self, State};
use drm::control::dumbbuffer::{DumbBuffer, DumbMapping};
use drm::control::Device as _;
use drm::{Device as _, DriverCapability};

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
}

pub struct CpuBackedBuffer {
    buffer: DumbBuffer,
    map: DumbMapping<'static>,
    shadow: Option<Box<[u8]>>,
}

impl CpuBackedBuffer {
    pub fn new(
        display_handle: &V2GraphicsHandle,
        size: (u32, u32),
        format: drm::buffer::DrmFourcc,
        bpp: u32,
    ) -> io::Result<CpuBackedBuffer> {
        let mut buffer = display_handle.create_dumb_buffer(size, format, bpp)?;

        let map = display_handle.map_dumb_buffer(&mut buffer)?;
        let map = unsafe { mem::transmute::<DumbMapping<'_>, DumbMapping<'static>>(map) };

        let shadow = if display_handle
            .get_driver_capability(DriverCapability::DumbPreferShadow)
            .unwrap_or(1)
            == 0
        {
            None
        } else {
            Some(vec![0; map.len()].into_boxed_slice())
        };

        Ok(CpuBackedBuffer {
            buffer,
            map,
            shadow,
        })
    }

    pub fn buffer(&self) -> &DumbBuffer {
        &self.buffer
    }

    pub fn shadow_buf(&mut self) -> &mut [u8] {
        self.shadow.as_deref_mut().unwrap_or(&mut *self.map)
    }

    pub fn sync_range(&mut self, ranges: impl Iterator<Item = Range<usize>>) {
        let Some(shadow) = &self.shadow else {
            return; // No shadow buffer; all writes are already propagated to the GPU.
        };

        for range in ranges {
            assert!(range.start <= range.end);
            assert!(range.end <= self.map.len());

            unsafe {
                ptr::copy_nonoverlapping(
                    shadow.as_ptr().add(range.start),
                    self.map.as_mut_ptr().add(range.start),
                    range.end - range.start,
                );
            }
        }

        // No need for a wbinvd to flush the write combining writes as they are
        // already flushed on the next syscall anyway. And the user will need
        // to do a DRM ioctl to actually present the changes on the display.
    }

    pub fn destroy(self, display_handle: &V2GraphicsHandle) -> io::Result<()> {
        display_handle.destroy_dumb_buffer(self.buffer)
    }
}
