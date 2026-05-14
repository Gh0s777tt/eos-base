use std::sync::Mutex;

use driver_graphics::kms::connector::{KmsConnectorDriver, KmsConnectorStatus};
use driver_graphics::kms::objects::{
    KmsCrtc, KmsCrtcDriver, KmsCrtcState, KmsObjectId, KmsObjects, KmsPlane, KmsPlaneState,
};
use driver_graphics::{Buffer, CursorPlane, Damage, GraphicsAdapter};
use drm_sys::{
    DRM_CAP_DUMB_BUFFER, DRM_CAP_DUMB_PREFERRED_DEPTH, DRM_CAP_DUMB_PREFER_SHADOW,
    DRM_CLIENT_CAP_CURSOR_PLANE_HOTSPOT,
};
use syscall::error::EINVAL;

use super::buffer::GpuBuffer;
use super::Device;

#[derive(Debug)]
pub struct Crtc {
    pub pipe_idx: usize,
}

#[derive(Debug)]
pub struct Connector {
    pub fb_id: Option<KmsObjectId>,
}

impl KmsCrtcDriver for Crtc {
    type State = ();
}

impl KmsConnectorDriver for Connector {
    type State = ();
}

impl Buffer for GpuBuffer {
    fn size(&self) -> usize {
        self.size as usize
    }
}

impl GraphicsAdapter for Device {
    type Connector = Connector;
    type Crtc = Crtc;

    type Buffer = GpuBuffer;
    type Framebuffer = ();

    type Plane = ();

    fn name(&self) -> &'static [u8] {
        b"ihdgd"
    }

    fn desc(&self) -> &'static [u8] {
        b"Intel HD Graphics"
    }

    fn init(&mut self, objects: &mut KmsObjects<Self>) {
        self.init_inner(objects);
    }

    fn get_unique(&self) -> String {
        self.unique.clone()
    }

    fn get_cap(&self, cap: u32) -> syscall::Result<u64> {
        match cap {
            DRM_CAP_DUMB_BUFFER => Ok(1),
            DRM_CAP_DUMB_PREFERRED_DEPTH => Ok(24),
            DRM_CAP_DUMB_PREFER_SHADOW => Ok(1),
            _ => Err(syscall::Error::new(EINVAL)),
        }
    }

    fn set_client_cap(&self, cap: u32, _value: u64) -> syscall::Result<()> {
        match cap {
            // FIXME hide cursor plane unless this client cap is set
            DRM_CLIENT_CAP_CURSOR_PLANE_HOTSPOT => Ok(()),
            _ => Err(syscall::Error::new(EINVAL)),
        }
    }

    fn probe_connector(&mut self, objects: &mut KmsObjects<Self>, id: KmsObjectId) {
        let mut connector = objects.get_connector(id).unwrap().lock().unwrap();
        connector.connection = KmsConnectorStatus::Connected;
        // FIXME fetch EDID
    }

    fn create_dumb_buffer(&mut self, width: u32, height: u32) -> (Self::Buffer, u32) {
        GpuBuffer::alloc_dumb(&self.gm, &mut self.ggtt, width, height).unwrap()
    }

    fn map_dumb_buffer(&mut self, buffer: &Self::Buffer) -> *mut u8 {
        buffer.virt
    }

    fn create_framebuffer(&mut self, _buffer: &Self::Buffer) -> Self::Framebuffer {
        ()
    }

    fn set_crtc(
        &mut self,
        objects: &KmsObjects<Self>,
        crtc: &Mutex<KmsCrtc<Self>>,
        state: KmsCrtcState<Self>,
        _damage: Damage,
    ) -> syscall::Result<()> {
        let mut crtc = crtc.lock().unwrap();
        crtc.state = state;
        Ok(())
    }

    fn set_plane(
        &mut self,
        objects: &KmsObjects<Self>,
        crtc: &Mutex<KmsCrtc<Self>>,
        plane: &Mutex<KmsPlane<Self>>,
        new_plane_state: KmsPlaneState<Self>,
        damage: Damage,
    ) -> syscall::Result<()> {
        let crtc = crtc.lock().unwrap();
        let mut plane = plane.lock().unwrap();

        let buffer = new_plane_state
            .fb_id
            .map(|fb_id| objects.get_framebuffer(fb_id))
            .transpose()?;

        plane.state = new_plane_state;

        let pipe_idx = crtc.driver_data.pipe_idx;
        if let Some(plane_hw) = self.pipes[pipe_idx].planes.first_mut() {
            plane_hw.set_framebuffer(buffer);
        }

        Ok(())
    }

    fn has_cursor_plane(&self) -> bool {
        false
    }

    fn handle_cursor(&mut self, _cursor: &CursorPlane<Self::Buffer>, _dirty_fb: bool) {
        unimplemented!("ihdgd does not support this function");
    }
}
