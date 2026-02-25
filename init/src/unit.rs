use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{fs, io};

use serde::Deserialize;

use crate::SwitchRoot;
use crate::script::Script;
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

    pub fn load_units(&mut self) -> (Vec<UnitId>, Vec<String>) {
        let mut loaded_units = vec![];
        let mut errors = vec![];

        let entries = match config::config_for_dirs(&self.config_dirs) {
            Ok(entries) => entries,
            Err(err) => {
                errors.push(format!(
                    "failed to read configs from {}: {err}",
                    self.config_dirs
                        .iter()
                        .map(|dir| dir.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                return (loaded_units, errors);
            }
        };

        for entry in entries {
            let (unit, new_errors) = match Unit::from_file(&entry) {
                Ok(unit) => unit,
                Err(err) => {
                    errors.push(format!("{}: {err}", entry.display()));
                    continue;
                }
            };
            errors.extend(new_errors);
            loaded_units.push(unit.id.clone());
            self.units.insert(unit.id.clone(), unit);
        }

        (loaded_units, errors)
    }

    pub fn unit_mut(&mut self, unit: &UnitId) -> &mut Unit {
        self.units.get_mut(unit).unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnitId(pub String);

pub struct Unit {
    pub id: UnitId,

    pub info: UnitInfo,
    pub kind: UnitKind,
}

#[derive(Deserialize)]
pub struct UnitInfo {
    pub description: Option<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub requires_weak: Vec<String>,
}

pub enum UnitKind {
    LegacyScript { script: Script },
    Service { service: Service },
    SwitchRoot { switchroot: SwitchRoot },
}

#[derive(Deserialize)]
struct SerializedService {
    unit: UnitInfo,
    service: Service,
}

#[derive(Deserialize)]
struct SerializedSwitchRoot {
    unit: UnitInfo,
    switchroot: SwitchRoot,
}

impl Unit {
    pub fn from_file(config_path: &Path) -> io::Result<(Self, Vec<String>)> {
        let name = UnitId(
            config_path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned(),
        );

        let config = fs::read_to_string(config_path)?;

        let (info, kind, errors) = match config_path.extension().map(|ext| ext.to_str().unwrap()) {
            None => {
                let (script, warnings) = Script::from_str(&config)?;
                (
                    UnitInfo {
                        description: None,
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
            Some("switchroot") => {
                let switchroot: SerializedSwitchRoot = serde_json::from_str(&config)?;
                (
                    switchroot.unit,
                    UnitKind::SwitchRoot {
                        switchroot: switchroot.switchroot,
                    },
                    vec![],
                )
            }
            Some(_) => return Err(io::Error::other("invalid file extension")),
        };

        Ok((
            Unit {
                id: name,
                info,
                kind,
            },
            errors,
        ))
    }
}
