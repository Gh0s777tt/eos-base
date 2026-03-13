use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use syscall::{Error, Result, EINVAL};

use crate::GraphicsAdapter;

#[derive(Debug)]
pub struct KmsObjects<T: GraphicsAdapter> {
    next_id: KmsObjectId,
    pub(crate) connectors: Vec<KmsObjectId>,
    pub(crate) encoders: Vec<KmsObjectId>,
    pub(crate) objects: HashMap<KmsObjectId, Arc<KmsObjectData>>,
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

    pub(crate) fn add<U: KmsObject>(&mut self, data: U) -> KmsObjectId {
        let id = self.next_id;
        self.objects.insert(
            id,
            Arc::new(KmsObjectData {
                kind: Box::new(data),
                properties: Mutex::new(vec![]),
            }),
        );
        self.next_id.0 += 1;

        id
    }

    pub(crate) fn get<U: KmsObject>(&self, id: KmsObjectId) -> Result<&U> {
        let object = self.objects.get(&id).ok_or(Error::new(EINVAL))?;
        if let Some(object) = (&*object.kind as &dyn Any).downcast_ref::<U>() {
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
pub(crate) struct KmsObjectData {
    kind: Box<dyn KmsObject + 'static>,
    pub(crate) properties: Mutex<Vec<(KmsObjectId, u64)>>,
}

pub trait KmsObject: Any + Debug {
    fn object_type(&self) -> u32;
}
