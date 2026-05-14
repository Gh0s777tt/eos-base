use std::collections::HashMap;

use drm_sys::{DRM_MODE_CURSOR_BO, DRM_MODE_CURSOR_MOVE};
use syscall::{ENOENT, ENXIO, Error};

use crate::{DrmHandle, GraphicsAdapter, VtState};

pub(super) fn mode_cursor<T: GraphicsAdapter>(
    adapter: &mut T,
    vts: &mut HashMap<usize, VtState<T>>,
    handle: &mut DrmHandle<T>,
    data: redox_ioctl::drm::DrmModeCursor<'_>,
) -> Result<usize, Error> {
    cursor_inner(
        adapter,
        vts,
        handle,
        data.flags(),
        data.crtc_id(),
        data.x(),
        data.y(),
        data.width(),
        data.height(),
        data.handle(),
        0,
        0,
    )
}

pub(super) fn mode_cursor2<T: GraphicsAdapter>(
    adapter: &mut T,
    vts: &mut HashMap<usize, VtState<T>>,
    handle: &mut DrmHandle<T>,
    data: redox_ioctl::drm::DrmModeCursor2<'_>,
) -> Result<usize, Error> {
    cursor_inner(
        adapter,
        vts,
        handle,
        data.flags(),
        data.crtc_id(),
        data.x(),
        data.y(),
        data.width(),
        data.height(),
        data.handle(),
        data.hot_x(),
        data.hot_y(),
    )
}

fn cursor_inner<T: GraphicsAdapter>(
    adapter: &mut T,
    vts: &mut HashMap<usize, VtState<T>>,
    handle: &mut DrmHandle<T>,

    flags: u32,
    _crtc_id: u32,
    x: i32,
    y: i32,
    _width: u32,
    _height: u32,
    handle_id: u32,
    hot_x: i32,
    hot_y: i32,
) -> Result<usize, Error> {
    let vt_state = vts.get_mut(&handle.vt).unwrap();

    let Some(cursor_plane) = &mut vt_state.cursor_plane else {
        return Err(Error::new(ENXIO));
    };

    let update_buffer = flags & DRM_MODE_CURSOR_BO != 0;
    if update_buffer {
        cursor_plane.buffer = if handle_id == 0 {
            None
        } else if let Some(buffer) = handle.buffers.get(&handle_id) {
            Some(buffer.clone())
        } else {
            return Err(Error::new(ENOENT));
        };
        cursor_plane.hot_x = hot_x;
        cursor_plane.hot_y = hot_y;
    }

    if flags & DRM_MODE_CURSOR_MOVE != 0 {
        cursor_plane.x = x;
        cursor_plane.y = y;
    }

    adapter.handle_cursor(cursor_plane, update_buffer);

    Ok(0)
}
