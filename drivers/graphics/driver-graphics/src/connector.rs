use std::fmt::Debug;

use drm_sys::{
    drm_mode_modeinfo, DRM_MODE_CONNECTOR_Unknown, DRM_MODE_OBJECT_CONNECTOR,
    DRM_MODE_OBJECT_ENCODER,
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

        let connector_id = self.add(DrmConnector {
            encoder_id,
            modes: vec![],
            connector_type: DRM_MODE_CONNECTOR_Unknown,
            connector_type_id: self.connectors.len() as u32, // FIXME maybe pick unique id within connector type?
            connection: DrmConnectorStatus::Unknown,
            mm_width: 0,
            mm_height: 0,
            subpixel: DrmSubpixelOrder::Unknown,
            driver_data,
        });
        self.connectors.push(connector_id);

        connector_id
    }

    pub fn connector_ids(&self) -> &[DrmObjectId] {
        &self.connectors
    }

    pub fn connectors(&self) -> impl Iterator<Item = &DrmConnector<T::Connector>> + use<'_, T> {
        self.connectors.iter().map(|&id| self.get(id).unwrap())
    }

    pub fn get_connector(&self, id: DrmObjectId) -> Result<&DrmConnector<T::Connector>> {
        self.get(id)
    }

    pub fn get_connector_mut(
        &mut self,
        id: DrmObjectId,
    ) -> Result<&mut DrmConnector<T::Connector>> {
        self.get_mut(id)
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

impl<T: Debug + 'static> DrmObject for DrmConnector<T> {
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
