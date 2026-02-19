use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::{env, fs, io, iter, process};

fn subst_env<'a>(arg: &str) -> String {
    if arg.starts_with('$') {
        env::var(&arg[1..]).unwrap_or(String::new())
    } else {
        arg.to_owned()
    }
}

pub struct Script(pub Vec<Command>);

impl Script {
    pub fn from_file(file: &Path) -> io::Result<(Script, Vec<String>)> {
        let mut cmds = vec![];
        let mut errors = vec![];

        for line_raw in fs::read_to_string(file)?.lines() {
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

#[derive(Debug)]
pub enum Command {
    // Service
    Nowait(Process),
    Notify(Process),
    Scheme(String, Process),
    Regular(Process),

    // Modify env
    Stdio(String),
    Export(String, String),
    Unset(Vec<String>),

    // Misc
    Echo(String),
    SwitchRoot(PathBuf, PathBuf),
    Nothing,
}

impl Command {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
        let Some(cmd) = args.next() else {
            return Ok(Command::Nothing);
        };

        match cmd.as_str() {
            "echo" => Ok(Command::Echo(args.collect::<Vec<_>>().join(" "))),
            "export" => {
                let Some(var) = args.next() else {
                    return Err("init: failed to export: no argument".to_owned());
                };
                let mut value = String::new();
                if let Some(arg) = args.next() {
                    value.push_str(&arg);
                }
                for arg in args {
                    value.push(' ');
                    value.push_str(&arg);
                }
                Ok(Command::Export(var, value))
            }
            "switchroot" => {
                let Some(prefix) = args.next() else {
                    return Err("init: failed to switchroot: no argument".to_owned());
                };
                let Some(etcdir) = args.next() else {
                    return Err("init: failed to switchroot: missing etcdir".to_owned());
                };
                Ok(Command::SwitchRoot(
                    PathBuf::from(prefix),
                    PathBuf::from(etcdir),
                ))
            }
            "stdio" => {
                let Some(stdio) = args.next() else {
                    return Err("init: failed to set stdio: no argument".to_owned());
                };
                Ok(Command::Stdio(stdio))
            }
            "unset" => Ok(Command::Unset(args.collect())),
            "nowait" => Ok(Command::Nowait(Process::parse(args)?)),
            "notify" => Ok(Command::Notify(Process::parse(args)?)),
            "scheme" => {
                let Some(scheme) = args.next() else {
                    return Err("init: failed to run scheme: no argument".to_owned());
                };

                Ok(Command::Scheme(scheme, Process::parse(args)?))
            }
            _ => Ok(Command::Regular(Process::parse(
                iter::once(cmd).chain(args),
            )?)),
        }
    }
}

#[derive(Debug)]
pub struct Process {
    pub cmd: String,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
}

impl Process {
    fn parse(parts: impl Iterator<Item = String>) -> Result<Process, String> {
        let mut cmd = None;
        let mut args = vec![];
        let mut envs = vec![];

        for arg in parts {
            if cmd.is_none() {
                if let Some((env, value)) = arg.split_once('=') {
                    let value = if value == "$" {
                        env::var(env).unwrap_or_default()
                    } else {
                        subst_env(value)
                    };
                    if !value.is_empty() {
                        envs.push((env.to_owned(), value));
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

    pub fn into_command(self, base_envs: &BTreeMap<String, OsString>) -> process::Command {
        let mut command = process::Command::new(self.cmd);
        command.args(self.args);
        command.env_clear().envs(base_envs).envs(self.envs);
        command
    }
}
