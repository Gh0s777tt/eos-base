use std::collections::{BTreeMap, BTreeSet};
use std::{env, io, iter};

use crate::service::{Service, ServiceType};
use crate::unit::UnitId;

pub fn subst_env<'a>(arg: &str) -> String {
    if arg.starts_with('$') {
        env::var(&arg[1..]).unwrap_or(String::new())
    } else {
        arg.to_owned()
    }
}

pub struct Script(pub Vec<Command>);

impl Script {
    pub fn from_str(config: &str) -> io::Result<(Script, Vec<String>)> {
        let mut cmds = vec![];
        let mut errors = vec![];

        for line_raw in config.lines() {
            let line = line_raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let args = line.split(' ').map(subst_env);

            match Command::parse(args) {
                Ok(cmd) => cmds.push(cmd),
                Err(err) => errors.push(err),
            }
        }

        Ok((Script(cmds), errors))
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    // Dependencies
    RequiresWeak(Vec<UnitId>),

    // Service
    Service(Service),

    // Misc
    Echo(String),
    Nothing,
}

impl Command {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
        let Some(cmd) = args.next() else {
            return Ok(Command::Nothing);
        };

        match cmd.as_str() {
            "requires_weak" => Ok(Command::RequiresWeak(args.map(UnitId).collect::<Vec<_>>())),
            "echo" => Ok(Command::Echo(args.collect::<Vec<_>>().join(" "))),
            "notify" => {
                let process = Process::parse(args)?;

                Ok(Command::Service(Service {
                    cmd: process.cmd,
                    args: process.args,
                    envs: process.envs,
                    inherit_envs: BTreeSet::new(),
                    type_: ServiceType::Notify,
                }))
            }
            "scheme" => {
                let Some(scheme) = args.next() else {
                    return Err("init: failed to run scheme: no argument".to_owned());
                };

                let process = Process::parse(args)?;

                Ok(Command::Service(Service {
                    cmd: process.cmd,
                    args: process.args,
                    envs: process.envs,
                    inherit_envs: BTreeSet::new(),
                    type_: ServiceType::Scheme(scheme),
                }))
            }
            "nowait" => {
                let process = Process::parse(args)?;

                Ok(Command::Service(Service {
                    cmd: process.cmd,
                    args: process.args,
                    envs: process.envs,
                    inherit_envs: BTreeSet::new(),
                    type_: ServiceType::OneshotAsync,
                }))
            }
            _ => {
                let process = Process::parse(iter::once(cmd).chain(args))?;

                Ok(Command::Service(Service {
                    cmd: process.cmd,
                    args: process.args,
                    envs: process.envs,
                    inherit_envs: BTreeSet::new(),
                    type_: ServiceType::Oneshot,
                }))
            }
        }
    }
}

#[derive(Debug)]
pub struct Process {
    pub cmd: String,
    pub args: Vec<String>,
    pub envs: BTreeMap<String, String>,
}

impl Process {
    fn parse(parts: impl Iterator<Item = String>) -> Result<Process, String> {
        let mut cmd = None;
        let mut args = vec![];
        let mut envs = BTreeMap::new();

        for arg in parts {
            if cmd.is_none() {
                if let Some((env, value)) = arg.split_once('=') {
                    let value = if value == "$" {
                        env::var(env).unwrap_or_default()
                    } else {
                        subst_env(value)
                    };
                    if !value.is_empty() {
                        envs.insert(env.to_owned(), value);
                    }
                } else {
                    cmd = Some(arg);
                }
            } else {
                args.push(arg);
            }
        }

        if let Some(cmd) = cmd {
            Ok(Process { cmd, args, envs })
        } else {
            Err("no command given".to_owned())
        }
    }
}
