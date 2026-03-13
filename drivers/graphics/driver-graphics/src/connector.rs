use std::ffi::c_char;
use std::fmt::Debug;
use std::sync::Mutex;

use drm_sys::{
    drm_mode_modeinfo, DRM_MODE_CONNECTOR_Unknown, DRM_MODE_OBJECT_CONNECTOR,
    DRM_MODE_OBJECT_ENCODER, DRM_MODE_TYPE_PREFERRED,
};
use syscall::Result;

use crate::objects::{DrmObject, DrmObjectId, DrmObjects};
use crate::GraphicsAdapter;

impl<T: GraphicsAdapter> DrmObjects<T> {
    pub fn add_connector(&mut self, driver_data: T::Connector) -> DrmObjectId {
        let encoder_id = self.add(DrmEncoder {
            crtc_id: DrmObjectId::INVALID,
            possible_crtcs: 0,
            possible_clones: 1 << self.encoders.len(),
        });
        self.encoders.push(encoder_id);

        let connector_id = self.add(Mutex::new(DrmConnector {
            encoder_id,
            modes: vec![],
            connector_type: DRM_MODE_CONNECTOR_Unknown,
            connector_type_id: self.connectors.len() as u32, // FIXME maybe pick unique id within connector type?
            connection: DrmConnectorStatus::Unknown,
            mm_width: 0,
            mm_height: 0,
            subpixel: DrmSubpixelOrder::Unknown,
            driver_data,
        }));
        self.connectors.push(connector_id);

        connector_id
    }

    pub fn connector_ids(&self) -> &[DrmObjectId] {
        &self.connectors
    }

    pub fn connectors(
        &self,
    ) -> impl Iterator<Item = &Mutex<DrmConnector<T::Connector>>> + use<'_, T> {
        self.connectors
            .iter()
            .map(|&id| self.get_connector(id).unwrap())
    }

    pub fn get_connector(&self, id: DrmObjectId) -> Result<&Mutex<DrmConnector<T::Connector>>> {
        self.get(id)
    }

    pub fn encoder_ids(&self) -> &[DrmObjectId] {
        &self.encoders
    }

    pub fn get_encoder(&self, id: DrmObjectId) -> Result<&DrmEncoder> {
        self.get(id)
    }
}

#[derive(Debug)]
pub struct DrmConnector<T: Debug + 'static> {
    pub encoder_id: DrmObjectId,
    pub modes: Vec<drm_mode_modeinfo>,
    pub connector_type: u32,
    pub connector_type_id: u32,
    pub connection: DrmConnectorStatus,
    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: DrmSubpixelOrder,
    pub driver_data: T,
}

impl<T: Debug + 'static> DrmConnector<T> {
    pub fn update_from_size(&mut self, width: u32, height: u32) {
        self.modes = vec![Self::modeinfo_for_size(width, height)];
    }

    pub fn update_from_edid(&mut self, edid: &[u8]) {
        let edid = edid::parse(edid).unwrap().1;

        if let Some(first_detailed_timing) =
            edid.descriptors
                .iter()
                .find_map(|descriptor| match descriptor {
                    edid::Descriptor::DetailedTiming(detailed_timing) => Some(detailed_timing),
                    _ => None,
                })
        {
            self.mm_width = first_detailed_timing.horizontal_size.into();
            self.mm_height = first_detailed_timing.vertical_size.into();
        } else {
            log::error!("No edid timing descriptor detected");
        }

        self.modes = edid
            .descriptors
            .iter()
            .filter_map(|descriptor| {
                match descriptor {
                    edid::Descriptor::DetailedTiming(detailed_timing) => {
                        // FIXME extract full information
                        Some(Self::modeinfo_for_size(
                            u32::from(detailed_timing.horizontal_active_pixels),
                            u32::from(detailed_timing.vertical_active_lines),
                        ))
                    }
                    _ => None,
                }
            })
            .collect::<Vec<_>>();

        // First detailed timing descriptor indicates preferred mode.
        for mode in self.modes.iter_mut().skip(1) {
            mode.flags &= !DRM_MODE_TYPE_PREFERRED;
        }

        // FIXME update the EDID property
    }

    fn modeinfo_for_size(width: u32, height: u32) -> drm_mode_modeinfo {
        let mut modeinfo = drm_mode_modeinfo {
            // The actual visible display size
            hdisplay: width as u16,
            vdisplay: height as u16,

            // These are used to calculate the refresh rate
            clock: 60 * width * height / 1000,
            htotal: width as u16,
            vtotal: height as u16,
            vscan: 0,
            vrefresh: 60,

            type_: drm_sys::DRM_MODE_TYPE_PREFERRED | drm_sys::DRM_MODE_TYPE_DRIVER,
            name: [0; 32],

            // These only matter when modesetting physical display adapters. For
            // those we should be able to parse the EDID blob.
            hsync_start: width as u16,
            hsync_end: width as u16,
            hskew: 0,
            vsync_start: height as u16,
            vsync_end: height as u16,
            flags: 0,
        };

        let name = format!("{width}x{height}").into_bytes();
        for (to, from) in modeinfo.name.iter_mut().zip(name) {
            *to = from as c_char;
        }

        modeinfo
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(u32)]
pub enum DrmConnectorStatus {
    Disconnected = 0,
    Connected = 1,
    Unknown = 2,
}

#[derive(Debug, Copy, Clone)]
#[repr(u32)]
pub enum DrmSubpixelOrder {
    Unknown = 0,
    HorizontalRGB,
    HorizontalBGR,
    VerticalRGB,
    VerticalBGR,
    None,
}

impl<T: Debug + 'static> DrmObject for Mutex<DrmConnector<T>> {
    fn object_type(&self) -> u32 {
        DRM_MODE_OBJECT_CONNECTOR
    }
}

// FIXME can we represent connector and encoder using a single struct?
#[derive(Debug)]
pub struct DrmEncoder {
    pub crtc_id: DrmObjectId,
    pub possible_crtcs: u32,
    pub possible_clones: u32,
}

impl DrmObject for DrmEncoder {
    fn object_type(&self) -> u32 {
        DRM_MODE_OBJECT_ENCODER
    }
}
