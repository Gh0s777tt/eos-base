use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::process::Command;

use serde::Deserialize;

use crate::script::subst_env;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub envs: BTreeMap<String, String>,
    #[serde(default)]
    pub inherit_envs: BTreeSet<String>,
    #[serde(rename = "type")]
    pub type_: ServiceType,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceType {
    #[default]
    Notify,
    Scheme(String),
    Oneshot,
    OneshotAsync,
}

impl Service {
    pub fn spawn(&self, base_envs: &BTreeMap<String, OsString>) {
        let mut command = Command::new(&self.cmd);
        command.args(self.args.iter().map(|arg| subst_env(arg)));
        command.env_clear();
        for env in &self.inherit_envs {
            if let Some(value) = env::var_os(env) {
                command.env(env, value);
            }
        }
        command.envs(base_envs).envs(&self.envs);

        match &self.type_ {
            ServiceType::Notify => {
                daemon::Daemon::spawn(command);
            }
            ServiceType::Scheme(scheme) => {
                daemon::SchemeDaemon::spawn(command, &scheme);
            }
            ServiceType::Oneshot => {
                let mut child = match command.spawn() {
                    Ok(child) => child,
                    Err(err) => {
                        eprintln!("init: failed to execute {:?}: {}", command, err);
                        return;
                    }
                };
                match child.wait() {
                    Ok(exit_status) => {
                        if !exit_status.success() {
                            eprintln!("{command:?} failed with {exit_status}");
                        }
                    }
                    Err(err) => {
                        eprintln!("init: failed to wait for {:?}: {}", command, err)
                    }
                }
            }
            ServiceType::OneshotAsync => match command.spawn() {
                Ok(_child) => {}
                Err(err) => eprintln!("init: failed to execute '{:?}': {}", command, err),
            },
        }
    }
}
