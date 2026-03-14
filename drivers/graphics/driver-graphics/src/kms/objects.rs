use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use drm_sys::{
    DRM_MODE_OBJECT_BLOB, DRM_MODE_OBJECT_CONNECTOR, DRM_MODE_OBJECT_ENCODER,
    DRM_MODE_OBJECT_PROPERTY,
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
    pub(crate) objects: HashMap<KmsObjectId, Arc<KmsObjectData<T>>>,
    _marker: PhantomData<T>,
}

impl<T: GraphicsAdapter> KmsObjects<T> {
    pub(crate) fn new() -> Self {
        KmsObjects {
            next_id: KmsObjectId(1),
            connectors: vec![],
            encoders: vec![],
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
    Connector(Mutex<KmsConnector<T::Connector>>) = DRM_MODE_OBJECT_CONNECTOR,
    Encoder(KmsEncoder) = DRM_MODE_OBJECT_ENCODER,
    Property(KmsProperty) = DRM_MODE_OBJECT_PROPERTY,
    Blob(KmsBlob) = DRM_MODE_OBJECT_BLOB,
}
