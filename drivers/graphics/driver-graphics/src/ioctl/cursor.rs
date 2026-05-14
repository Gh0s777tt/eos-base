use std::collections::HashMap;

use drm_sys::{DRM_MODE_CURSOR_BO, DRM_MODE_CURSOR_MOVE};
use syscall::{Error, ENOENT, ENXIO};

use crate::{DrmHandle, GraphicsAdapter, VtState};

pub(super) fn mode_cursor<T: GraphicsAdapter>(
    adapter: &mut T,
    vts: &mut HashMap<usize, VtState<T>>,
    handle: &mut DrmHandle<T>,
    data: redox_ioctl::drm::DrmModeCursor<'_>,
) -> Result<usize, Error> {
    let vt_state = vts.get_mut(&handle.vt).unwrap();

    let Some(cursor_plane) = &mut vt_state.cursor_plane else {
        return Err(Error::new(ENXIO));
    };

    let update_buffer = data.flags() & DRM_MODE_CURSOR_BO != 0;
    if update_buffer {
        cursor_plane.buffer = if data.handle() == 0 {
            None
        } else if let Some(buffer) = handle.buffers.get(&data.handle()) {
            Some(buffer.clone())
        } else {
            return Err(Error::new(ENOENT));
        };
    }

    if data.flags() & DRM_MODE_CURSOR_MOVE != 0 {
        cursor_plane.x = data.x();
        cursor_plane.y = data.y();
    }

    adapter.handle_cursor(cursor_plane, update_buffer);

    Ok(0)
}

pub(super) fn mode_cursor2<T: GraphicsAdapter>(
    adapter: &mut T,
    vts: &mut HashMap<usize, VtState<T>>,
    handle: &mut DrmHandle<T>,
    data: redox_ioctl::drm::DrmModeCursor2<'_>,
) -> Result<usize, Error> {
    let vt_state = vts.get_mut(&handle.vt).unwrap();

    let Some(cursor_plane) = &mut vt_state.cursor_plane else {
        return Err(Error::new(ENXIO));
    };

    let update_buffer = data.flags() & DRM_MODE_CURSOR_BO != 0;
    if update_buffer {
        cursor_plane.buffer = if data.handle() == 0 {
            None
        } else if let Some(buffer) = handle.buffers.get(&data.handle()) {
            Some(buffer.clone())
        } else {
            return Err(Error::new(ENOENT));
        };
        cursor_plane.hot_x = data.hot_x();
        cursor_plane.hot_y = data.hot_y();
    }

    if data.flags() & DRM_MODE_CURSOR_MOVE != 0 {
        cursor_plane.x = data.x();
        cursor_plane.y = data.y();
    }

    adapter.handle_cursor(cursor_plane, update_buffer);

    Ok(0)
}
