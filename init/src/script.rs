use std::path::{Path, PathBuf};
use std::{env, fs, io};

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

            let args = line.split(' ').map(|arg| {
                if arg.starts_with('$') {
                    env::var(&arg[1..]).unwrap_or(String::new())
                } else {
                    arg.to_string()
                }
            });

            match Command::from_arg_iter(args) {
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
    Nowait(String, Vec<String>),
    Notify(String, Vec<String>),
    Scheme(String, String, Vec<String>),
    Regular(String, Vec<String>),

    // Modify env
    Cd(PathBuf),
    Stdio(String),
    Export(String, String),
    Unset(Vec<String>),

    // Misc
    Echo(String),
    SwitchRoot(PathBuf, PathBuf),
    Nothing,
}

impl Command {
    fn from_arg_iter(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
        let Some(cmd) = args.next() else {
            return Ok(Command::Nothing);
        };

        match cmd.as_str() {
            "cd" => {
                let Some(dir) = args.next() else {
                    return Err("init: failed to cd: no argument".to_owned());
                };
                Ok(Command::Cd(PathBuf::from(dir)))
            }
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
            "nowait" => {
                let Some(cmd) = args.next() else {
                    return Err("init: failed to run nowait: no argument".to_owned());
                };

                Ok(Command::Nowait(cmd, args.collect()))
            }
            "notify" => {
                let Some(cmd) = args.next() else {
                    return Err("init: failed to run notify: no argument".to_owned());
                };

                Ok(Command::Notify(cmd, args.collect()))
            }
            "scheme" => {
                let Some(scheme) = args.next() else {
                    return Err("init: failed to run scheme: no argument".to_owned());
                };
                let Some(cmd) = args.next() else {
                    return Err("init: failed to run scheme: missing command".to_owned());
                };

                Ok(Command::Scheme(scheme, cmd, args.collect()))
            }
            _ => Ok(Command::Regular(cmd, args.collect())),
        }
    }
}
