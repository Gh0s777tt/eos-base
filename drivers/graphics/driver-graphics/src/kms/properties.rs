use std::ffi::c_char;
use std::fmt::Debug;
use std::sync::Mutex;

use drm_sys::DRM_PROP_NAME_LEN;
use syscall::{Error, Result, EINVAL};

use crate::kms::objects::{KmsObjectId, KmsObjects};
use crate::GraphicsAdapter;

impl<T: GraphicsAdapter> KmsObjects<T> {
    pub fn add_property(
        &mut self,
        name: &str,
        immutable: bool,
        atomic: bool,
        kind: KmsPropertyKind,
    ) -> KmsObjectId {
        if name.len() > DRM_PROP_NAME_LEN as usize {
            panic!("Property name {name} is too long");
        }

        match &kind {
            KmsPropertyKind::Range(start, end) => assert!(start < end),
            KmsPropertyKind::Enum(variants) => {
                // FIXME check duplicate variant numbers
                for (variant_name, _) in variants {
                    if variant_name.len() > DRM_PROP_NAME_LEN as usize {
                        panic!("Property variant name {variant_name} is too long");
                    }
                }
            }
            KmsPropertyKind::Blob => {}
            KmsPropertyKind::Bitmask(bitmask_flags) => {
                // FIXME check overlapping flag numbers
                for (flag_name, _) in bitmask_flags {
                    if flag_name.len() > DRM_PROP_NAME_LEN as usize {
                        panic!("Property bitflag name {flag_name} is too long");
                    }
                }
            }
            KmsPropertyKind::Object => {}
            KmsPropertyKind::SignedRange(start, end) => assert!(start < end),
        }

        let mut name_bytes = [0; DRM_PROP_NAME_LEN as usize];
        for (to, &from) in name_bytes.iter_mut().zip(name.as_bytes()) {
            *to = from as c_char;
        }

        self.add(KmsProperty {
            name: name_bytes,
            immutable,
            atomic,
            kind,
        })
    }

    pub fn get_property(&self, id: KmsObjectId) -> Result<&KmsProperty> {
        self.get(id)
    }

    pub fn add_object_property(&mut self, object: KmsObjectId, property: KmsObjectId, value: u64) {
        let object = self.objects.get_mut(&object).unwrap();
        // FIXME validate property uniqueness and value
        object.properties.lock().unwrap().push((property, value));
    }

    pub fn set_object_property(&mut self, object: KmsObjectId, property: KmsObjectId, value: u64) {
        let object = self.objects.get_mut(&object).unwrap();
        // FIXME validate property existence and value
        for (prop, val) in object.properties.lock().unwrap().iter_mut() {
            if *prop == property {
                *val = value;
            }
        }
    }

    pub fn get_object_properties(
        &self,
        id: KmsObjectId,
    ) -> Result<&Mutex<Vec<(KmsObjectId, u64)>>> {
        let object = self.objects.get(&id).ok_or(Error::new(EINVAL))?;
        Ok(&object.properties)
    }

    pub fn add_blob(&mut self, data: Vec<u8>) -> KmsObjectId {
        self.add(KmsBlob { data })
    }

    pub fn get_blob(&self, id: KmsObjectId) -> Result<&[u8]> {
        Ok(&self.get::<KmsBlob>(id)?.data)
    }
}

#[derive(Debug)]
pub struct KmsProperty {
    pub name: [c_char; DRM_PROP_NAME_LEN as usize],
    pub immutable: bool,
    pub atomic: bool,
    pub kind: KmsPropertyKind,
}

#[derive(Debug)]
pub enum KmsPropertyKind {
    Range(u64, u64),
    Enum(Vec<(&'static str, u64)>),
    Blob,
    Bitmask(Vec<(&'static str, u64)>),
    Object,
    SignedRange(i64, i64),
}

#[derive(Debug)]
pub struct KmsBlob {
    data: Vec<u8>,
}
