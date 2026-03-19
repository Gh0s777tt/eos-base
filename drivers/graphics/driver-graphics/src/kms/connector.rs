use std::ffi::c_char;
use std::fmt::Debug;
use std::sync::Mutex;

use drm_sys::{
    drm_mode_modeinfo, DRM_MODE_CONNECTOR_Unknown, DRM_MODE_DPMS_OFF, DRM_MODE_DPMS_ON,
    DRM_MODE_DPMS_STANDBY, DRM_MODE_DPMS_SUSPEND, DRM_MODE_TYPE_PREFERRED,
};
use syscall::Result;

use crate::kms::objects::{KmsObjectId, KmsObjects};
use crate::kms::properties::{KmsPropertyData, DPMS, EDID};
use crate::GraphicsAdapter;

impl<T: GraphicsAdapter> KmsObjects<T> {
    pub fn add_connector(
        &mut self,
        driver_data: T::Connector,
        crtcs: &[KmsObjectId],
    ) -> KmsObjectId {
        let mut possible_crtcs = 0;
        for &crtc in crtcs {
            possible_crtcs = 1 << self.get_crtc(crtc).unwrap().lock().unwrap().crtc_index;
        }

        let encoder_id = self.add(KmsEncoder {
            crtc_id: KmsObjectId::INVALID,
            possible_crtcs: possible_crtcs,
            possible_clones: 1 << self.encoders.len(),
        });
        self.encoders.push(encoder_id);

        let connector_id = self.add(Mutex::new(KmsConnector {
            encoder_id,
            modes: vec![],
            connector_type: DRM_MODE_CONNECTOR_Unknown,
            connector_type_id: self.connectors.len() as u32, // FIXME maybe pick unique id within connector type?
            connection: KmsConnectorStatus::Unknown,
            mm_width: 0,
            mm_height: 0,
            subpixel: DrmSubpixelOrder::Unknown,
            properties: vec![],
            dpms: KmsDpms::Off,
            edid: KmsObjectId::INVALID,
            driver_data,
        }));
        self.connectors.push(connector_id);

        let mut connector = self.get_connector(connector_id).unwrap().lock().unwrap();
        connector.properties.push(KmsPropertyData {
            id: DPMS,
            getter: |connector| connector.dpms as u64,
        });
        connector.properties.push(KmsPropertyData {
            id: EDID,
            getter: |connector| connector.edid.0.into(),
        });

        connector_id
    }

    pub fn connector_ids(&self) -> &[KmsObjectId] {
        &self.connectors
    }

    pub fn connectors(
        &self,
    ) -> impl Iterator<Item = &Mutex<KmsConnector<T::Connector>>> + use<'_, T> {
        self.connectors
            .iter()
            .map(|&id| self.get_connector(id).unwrap())
    }

    pub fn get_connector(&self, id: KmsObjectId) -> Result<&Mutex<KmsConnector<T::Connector>>> {
        self.get(id)
    }

    pub fn encoder_ids(&self) -> &[KmsObjectId] {
        &self.encoders
    }

    pub fn get_encoder(&self, id: KmsObjectId) -> Result<&KmsEncoder> {
        self.get(id)
    }
}

#[derive(Debug)]
pub struct KmsConnector<T> {
    pub encoder_id: KmsObjectId,
    pub modes: Vec<drm_mode_modeinfo>,
    pub connector_type: u32,
    pub connector_type_id: u32,
    pub connection: KmsConnectorStatus,
    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: DrmSubpixelOrder,
    pub properties: Vec<KmsPropertyData<Self>>,
    pub dpms: KmsDpms,
    pub edid: KmsObjectId,
    pub driver_data: T,
}

impl<T> KmsConnector<T> {
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

    pub(crate) fn modeinfo_for_size(width: u32, height: u32) -> drm_mode_modeinfo {
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
pub enum KmsConnectorStatus {
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

#[derive(Debug, Copy, Clone)]
#[repr(u64)]
pub enum KmsDpms {
    On = DRM_MODE_DPMS_ON as u64,
    Standby = DRM_MODE_DPMS_STANDBY as u64,
    Suspend = DRM_MODE_DPMS_SUSPEND as u64,
    Off = DRM_MODE_DPMS_OFF as u64,
}

// FIXME can we represent connector and encoder using a single struct?
#[derive(Debug)]
pub struct KmsEncoder {
    pub crtc_id: KmsObjectId,
    pub possible_crtcs: u32,
    pub possible_clones: u32,
}
