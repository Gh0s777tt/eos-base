use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use drm_sys::{
    drm_mode_modeinfo, DRM_MODE_OBJECT_BLOB, DRM_MODE_OBJECT_CONNECTOR, DRM_MODE_OBJECT_CRTC,
    DRM_MODE_OBJECT_ENCODER, DRM_MODE_OBJECT_PROPERTY,
};
use syscall::{Error, Result, EINVAL};

use crate::kms::connector::{KmsConnector, KmsEncoder};
use crate::kms::properties::{KmsBlob, KmsProperty};
use crate::GraphicsAdapter;

#[derive(Debug)]
pub struct KmsObjects<T: GraphicsAdapter> {
    next_id: KmsObjectId,
    pub(crate) connectors: Vec<KmsObjectId>,
    pub(crate) encoders: Vec<KmsObjectId>,
    crtcs: Vec<KmsObjectId>,
    pub(crate) objects: HashMap<KmsObjectId, Arc<KmsObjectData<T>>>,
    _marker: PhantomData<T>,
}

impl<T: GraphicsAdapter> KmsObjects<T> {
    pub(crate) fn new() -> Self {
        KmsObjects {
            next_id: KmsObjectId(1),
            connectors: vec![],
            encoders: vec![],
            crtcs: vec![],
            objects: HashMap::new(),
            _marker: PhantomData,
        }
    }

    pub(crate) fn add<U: Into<KmsObjectKind<T>>>(&mut self, data: U) -> KmsObjectId {
        let id = self.next_id;
        self.objects.insert(
            id,
            Arc::new(KmsObjectData {
                kind: Box::new(data.into()),
                properties: Mutex::new(vec![]),
            }),
        );
        self.next_id.0 += 1;

        id
    }

    pub(crate) fn get<'a, U: 'a>(&'a self, id: KmsObjectId) -> Result<&'a U>
    where
        &'a U: TryFrom<&'a KmsObjectKind<T>>,
    {
        let object = self.objects.get(&id).ok_or(Error::new(EINVAL))?;
        if let Ok(object) = (&*object.kind).try_into() {
            Ok(object)
        } else {
            Err(Error::new(EINVAL))
        }
    }

    pub fn object_type(&self, id: KmsObjectId) -> Result<u32> {
        let object = self.objects.get(&id).ok_or(Error::new(EINVAL))?;
        Ok(object.kind.object_type())
    }

    pub fn add_crtc(&mut self, driver_data: T::Crtc) -> KmsObjectId {
        let crtc_index = self.crtcs.len() as u32;
        let id = self.add(Mutex::new(KmsCrtc {
            crtc_index,
            fb_id: KmsObjectId::INVALID,
            gamma_size: 0,
            mode: None,
            driver_data,
        }));
        self.crtcs.push(id);

        id
    }

    pub fn crtc_ids(&self) -> &[KmsObjectId] {
        &self.crtcs
    }

    pub fn crtcs(&self) -> impl Iterator<Item = &Mutex<KmsCrtc<T::Crtc>>> + use<'_, T> {
        self.crtcs
            .iter()
            .map(|&id| self.get::<Mutex<KmsCrtc<T::Crtc>>>(id).unwrap())
    }

    pub fn get_crtc(&self, id: KmsObjectId) -> Result<&Mutex<KmsCrtc<T::Crtc>>> {
        self.get(id)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct KmsObjectId(pub(crate) u32);

impl KmsObjectId {
    pub const INVALID: KmsObjectId = KmsObjectId(0);
}

impl From<KmsObjectId> for u64 {
    fn from(value: KmsObjectId) -> Self {
        value.0.into()
    }
}

#[derive(Debug)]
pub(crate) struct KmsObjectData<T: GraphicsAdapter> {
    kind: Box<KmsObjectKind<T>>,
    pub(crate) properties: Mutex<Vec<(KmsObjectId, u64)>>,
}

macro_rules! define_object_kinds {
    (<$T:ident> $(
        $variant:ident($data:ty) = $type:ident,
    )*) => {
        #[derive(Debug)]
        pub(crate) enum KmsObjectKind<$T: GraphicsAdapter> {
            $($variant($data),)*
        }

        impl<$T: GraphicsAdapter> KmsObjectKind<$T> {
            fn object_type(&self) -> u32 {
                match self {
                    $(Self::$variant(_) => $type,)*
                }
            }
        }


        $(
            impl<$T: GraphicsAdapter> From<$data> for KmsObjectKind<$T> {
                fn from(value: $data) -> Self {
                    Self::$variant(value)
                }
            }

            impl<'a, $T: GraphicsAdapter> TryFrom<&'a KmsObjectKind<$T>> for &'a $data {
                type Error = ();

                fn try_from(value: &'a KmsObjectKind<T>) -> Result<Self, Self::Error> {
                    match value {
                        KmsObjectKind::$variant(data) => Ok(data),
                        _ => Err(()),
                    }
                }
            }
        )*
    };
}

define_object_kinds! { <T>
    Crtc(Mutex<KmsCrtc<T::Crtc>>) = DRM_MODE_OBJECT_CRTC,
    Connector(Mutex<KmsConnector<T::Connector>>) = DRM_MODE_OBJECT_CONNECTOR,
    Encoder(KmsEncoder) = DRM_MODE_OBJECT_ENCODER,
    Property(KmsProperty) = DRM_MODE_OBJECT_PROPERTY,
    Blob(KmsBlob) = DRM_MODE_OBJECT_BLOB,
}

#[derive(Debug)]
pub struct KmsCrtc<T> {
    pub crtc_index: u32,
    pub fb_id: KmsObjectId,
    pub gamma_size: u32,
    pub mode: Option<drm_mode_modeinfo>,
    pub driver_data: T,
}
