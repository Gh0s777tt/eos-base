use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use syscall::{Error, Result, EINVAL};

use crate::GraphicsAdapter;

#[derive(Debug)]
pub struct DrmObjects<T: GraphicsAdapter> {
    next_id: DrmObjectId,
    pub(crate) connectors: Vec<DrmObjectId>,
    pub(crate) encoders: Vec<DrmObjectId>,
    pub(crate) objects: HashMap<DrmObjectId, Arc<DrmObjectData>>,
    _marker: PhantomData<T>,
}

impl<T: GraphicsAdapter> DrmObjects<T> {
    pub(crate) fn new() -> Self {
        DrmObjects {
            next_id: DrmObjectId(1),
            connectors: vec![],
            encoders: vec![],
            objects: HashMap::new(),
            _marker: PhantomData,
        }
    }

    pub(crate) fn add<U: DrmObject>(&mut self, data: U) -> DrmObjectId {
        let id = self.next_id;
        self.objects.insert(
            id,
            Arc::new(DrmObjectData {
                kind: Box::new(data),
                properties: Mutex::new(vec![]),
            }),
        );
        self.next_id.0 += 1;

        id
    }

    pub(crate) fn get<U: DrmObject>(&self, id: DrmObjectId) -> Result<&U> {
        let object = self.objects.get(&id).ok_or(Error::new(EINVAL))?;
        if let Some(object) = (&*object.kind as &dyn Any).downcast_ref::<U>() {
            Ok(object)
        } else {
            Err(Error::new(EINVAL))
        }
    }

    pub fn object_type(&self, id: DrmObjectId) -> Result<u32> {
        let object = self.objects.get(&id).ok_or(Error::new(EINVAL))?;
        Ok(object.kind.object_type())
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct DrmObjectId(pub(crate) u32);

impl DrmObjectId {
    pub const INVALID: DrmObjectId = DrmObjectId(0);
}

impl From<DrmObjectId> for u64 {
    fn from(value: DrmObjectId) -> Self {
        value.0.into()
    }
}

#[derive(Debug)]
pub(crate) struct DrmObjectData {
    kind: Box<dyn DrmObject + 'static>,
    pub(crate) properties: Mutex<Vec<(DrmObjectId, u64)>>,
}

pub trait DrmObject: Any + Debug {
    fn object_type(&self) -> u32;
}
