use std::path::Path;
use std::{fs, io};

use serde::Deserialize;

use crate::script::Script;
use crate::service::Service;

pub struct Unit {
    pub name: String,

    pub info: UnitInfo,
    pub kind: UnitKind,
}

#[derive(Deserialize)]
pub struct UnitInfo {
    #[serde(default)]
    description: String,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    requires_weak: Vec<String>,
}

pub enum UnitKind {
    LegacyScript { script: Script },
    Service { service: Service },
}

#[derive(Deserialize)]
struct SerializedService {
    unit: UnitInfo,
    service: Service,
}

impl Unit {
    pub fn from_file(config_path: &Path) -> io::Result<(Self, Vec<String>)> {
        let name = config_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();

        let config = fs::read_to_string(config_path)?;

        let (info, kind, errors) = match config_path.extension().map(|ext| ext.to_str().unwrap()) {
            None => {
                let (script, warnings) = Script::from_str(&config)?;
                (
                    UnitInfo {
                        description: "".to_owned(),
                        requires: vec![],
                        requires_weak: vec![],
                    },
                    UnitKind::LegacyScript { script },
                    warnings,
                )
            }
            Some("service") => {
                let service: SerializedService = serde_json::from_str(&config)?;
                (
                    service.unit,
                    UnitKind::Service {
                        service: service.service,
                    },
                    vec![],
                )
            }
            Some(_) => return Err(io::Error::other("invalid file extension")),
        };

        Ok((Unit { name, info, kind }, errors))
    }
}
