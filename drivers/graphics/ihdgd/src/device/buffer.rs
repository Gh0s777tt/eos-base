use std::{ptr, slice};

use range_alloc::RangeAllocator;
use syscall::{Error, EIO};

use crate::device::MmioRegion;

#[derive(Debug)]
pub struct GpuBuffer {
    pub virt: *mut u8,
    pub gm_offset: u32,
    pub size: u32,
}

impl GpuBuffer {
    pub unsafe fn new(gm: &MmioRegion, gm_offset: u32, size: u32, clear: bool) -> Self {
        let virt = ptr::with_exposed_provenance_mut::<u8>(gm.virt + gm_offset as usize);

        if clear {
            let onscreen = slice::from_raw_parts_mut(virt, size as usize);
            onscreen.fill(0);
        }

        Self {
            virt,
            gm_offset,
            size,
        }
    }

    pub fn alloc(
        gm: &MmioRegion,
        alloc_surfaces: &mut RangeAllocator<u32>,
        size: u32,
    ) -> syscall::Result<Self> {
        let surf_size = size.next_multiple_of(4096);
        let gm_offset = alloc_surfaces
            .allocate_range(surf_size)
            .map_err(|err| {
                log::warn!("failed to allocate buffer of size {}: {:?}", surf_size, err);
                Error::new(EIO)
            })?
            .start;

        Ok(unsafe { GpuBuffer::new(gm, gm_offset, size, true) })
    }

    pub fn alloc_dumb(
        gm: &MmioRegion,
        alloc_surfaces: &mut RangeAllocator<u32>,
        width: u32,
        height: u32,
    ) -> syscall::Result<(Self, u32)> {
        //TODO: documentation on this is not great
        let stride = (width * 4).next_multiple_of(64);

        Ok((
            GpuBuffer::alloc(gm, alloc_surfaces, stride * height)?,
            stride,
        ))
    }
}
