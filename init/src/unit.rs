use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{fs, io};

use serde::Deserialize;

use crate::script::{Command, Script};
use crate::service::Service;

pub struct UnitStore {
    pub config_dirs: Vec<PathBuf>,
    units: BTreeMap<UnitId, Unit>,
}

impl UnitStore {
    pub fn new() -> Self {
        UnitStore {
            config_dirs: vec![],
            units: BTreeMap::new(),
        }
    }

    fn load_single_unit(&mut self, unit_id: UnitId, errors: &mut Vec<String>) -> Option<UnitId> {
        let Some(path) = self
            .config_dirs
            .iter()
            .rev()
            .map(|dir| dir.join(&unit_id.0))
            .find(|path| path.exists())
        else {
            errors.push(format!("unit {} not found", unit_id.0));
            return None;
        };

        let unit = match Unit::from_file(&path, errors) {
            Ok(unit) => unit,
            Err(err) => {
                errors.push(format!("{}: {err}", path.display()));
                return None;
            }
        };
        assert_eq!(unit_id, unit.id);
        self.units.insert(unit_id.clone(), unit);

        Some(unit_id)
    }

    pub fn load_units(&mut self, root_unit: UnitId, errors: &mut Vec<String>) -> Vec<UnitId> {
        let mut loaded_units = vec![];
        let mut pending_units = vec![root_unit];

        while let Some(unit_id) = pending_units.pop() {
            if self.units.contains_key(&unit_id) {
                continue;
            }
            let unit = self.load_single_unit(unit_id, errors);
            if let Some(unit) = unit {
                loaded_units.push(unit.clone());
                for dep in &self.unit(&unit).info.requires_weak {
                    pending_units.push(dep.clone());
                }
            }
        }

        loaded_units
    }

    pub fn unit(&self, unit: &UnitId) -> &Unit {
        self.units.get(unit).unwrap()
    }

    pub fn unit_mut(&mut self, unit: &UnitId) -> &mut Unit {
        self.units.get_mut(unit).unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(transparent)]
pub struct UnitId(pub String);

pub struct Unit {
    pub id: UnitId,

    pub info: UnitInfo,
    pub kind: UnitKind,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitInfo {
    pub description: Option<String>,
    #[serde(default = "true_bool")]
    pub default_dependencies: bool,
    #[serde(default)]
    pub requires_weak: Vec<UnitId>,
    pub condition_architecture: Option<Vec<String>>,
    // FIXME replace this with hwd reading from the devicetree
    pub condition_board: Option<Vec<String>>,
}

fn true_bool() -> bool {
    true
}

pub enum UnitKind {
    LegacyScript { script: Script },
    Service { service: Service },
    Target {},
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SerializedService {
    unit: UnitInfo,
    service: Service,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SerializedTarget {
    unit: UnitInfo,
}

impl Unit {
    pub fn from_file(config_path: &Path, errors: &mut Vec<String>) -> io::Result<Self> {
        let name = UnitId(
            config_path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned(),
        );

        let config = fs::read_to_string(config_path)?;

        let (info, kind) = match config_path.extension().map(|ext| ext.to_str().unwrap()) {
            None => {
                let script = Script::from_str(&config, errors)?;
                let mut requires_weak = vec![];
                for command in &script.0 {
                    match command {
                        Command::RequiresWeak(deps) => {
                            requires_weak.extend(deps.into_iter().cloned())
                        }
                        _ => {}
                    }
                }
                (
                    UnitInfo {
                        description: None,
                        default_dependencies: true,
                        requires_weak,
                        condition_architecture: None,
                        condition_board: None,
                    },
                    UnitKind::LegacyScript { script },
                )
            }
            Some("service") => {
                let service: SerializedService = toml::from_str(&config)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                (
                    service.unit,
                    UnitKind::Service {
                        service: service.service,
                    },
                )
            }
            Some("target") => {
                let target: SerializedTarget = toml::from_str(&config)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                (target.unit, UnitKind::Target {})
            }
            Some(_) => return Err(io::Error::other("invalid file extension")),
        };

        Ok(Unit {
            id: name,
            info,
            kind,
        })
    }
}
