use std::env;
use std::io::Result;
use std::path::Path;

use libredox::flag::{O_RDONLY, O_WRONLY};

use crate::script::Command;

mod script;

fn switch_stdio(stdio: &str) -> Result<()> {
    let stdin = libredox::Fd::open(stdio, O_RDONLY, 0)?;
    let stdout = libredox::Fd::open(stdio, O_WRONLY, 0)?;
    let stderr = libredox::Fd::open(stdio, O_WRONLY, 0)?;

    stdin.dup2(0, &[])?;
    stdout.dup2(1, &[])?;
    stderr.dup2(2, &[])?;

    Ok(())
}

struct InitConfig {
    log_debug: bool,
    skip_cmd: Vec<String>,
}

impl InitConfig {
    fn new() -> Self {
        let log_level = env::var("INIT_LOG_LEVEL").unwrap_or("INFO".into());
        let log_debug = matches!(log_level.as_str(), "DEBUG" | "TRACE");
        let skip_cmd: Vec<String> = match env::var("INIT_SKIP") {
            Ok(v) if v.len() > 0 => v.split(',').map(|s| s.to_string()).collect(),
            _ => Vec::new(),
        };

        Self {
            log_debug,
            skip_cmd,
        }
    }
}

fn switch_root(prefix: &Path, etcdir: &Path, config: &InitConfig) {
    unsafe {
        env::set_var("PATH", prefix.join("bin"));
        env::set_var("LD_LIBRARY_PATH", prefix.join("lib"));
    }
    let entries = match config::config_for_dirs(&[
        prefix.join("lib").join("init.d"),
        etcdir.join("init.d"),
    ]) {
        Ok(list) => list,
        Err(err) => {
            eprintln!("init: failed to switchroot: '{prefix:?}', '{etcdir:?}': {err}");
            return;
        }
    };

    for entry_path in entries {
        if let Err(err) = run(&entry_path, config) {
            eprintln!("init: failed to run '{}': {}", entry_path.display(), err);
        }
    }
}

fn run(file: &Path, config: &InitConfig) -> Result<()> {
    let (script, errors) = script::Script::from_file(file)?;

    for error in errors {
        eprintln!("init: {}: {error}", file.display());
    }

    for cmd in script.0 {
        if config.log_debug {
            eprintln!("init: running: {cmd:?}");
        }
        run_command(cmd, config);
    }

    Ok(())
}

fn run_command(cmd: Command, config: &InitConfig) {
    match cmd {
        Command::Nothing => {}
        Command::Echo(text) => println!("{text}"),
        Command::Export(var, value) => unsafe { env::set_var(var, value) },
        Command::SwitchRoot(prefix, etcdir) => {
            switch_root(&prefix, &etcdir, config);
        }
        Command::Stdio(stdio) => {
            if let Err(err) = switch_stdio(&stdio) {
                eprintln!("init: failed to switch stdio to '{}': {}", stdio, err);
            }
        }
        Command::Unset(envs) => {
            for env in envs {
                unsafe { env::remove_var(&env) };
            }
        }
        Command::Nowait(cmd) => {
            if config.skip_cmd.contains(&cmd.cmd) {
                eprintln!("init: skipping '{} {}'", cmd.cmd, cmd.args.join(" "));
                return;
            }

            let mut command = cmd.into_command();

            match command.spawn() {
                Ok(_child) => {}
                Err(err) => eprintln!("init: failed to execute '{:?}': {}", command, err),
            }
        }
        Command::Notify(cmd) => {
            if config.skip_cmd.contains(&cmd.cmd) {
                eprintln!("init: skipping '{} {}'", cmd.cmd, cmd.args.join(" "));
                return;
            }

            let command = cmd.into_command();

            daemon::Daemon::spawn(command);
        }
        Command::Scheme(scheme, cmd) => {
            if config.skip_cmd.contains(&cmd.cmd) {
                eprintln!("init: skipping '{} {}'", cmd.cmd, cmd.args.join(" "));
                return;
            }

            let command = cmd.into_command();

            daemon::SchemeDaemon::spawn(command, &scheme);
        }
        Command::Regular(cmd) => {
            if config.skip_cmd.contains(&cmd.cmd) {
                eprintln!("init: skipping '{} {}'", cmd.cmd, cmd.args.join(" "));
                return;
            }

            let mut command = cmd.into_command();

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
    }
}

fn main() {
    let init_config = InitConfig::new();
    switch_root(
        Path::new("/scheme/initfs"),
        Path::new("/scheme/initfs/etc"),
        &init_config,
    );

    libredox::call::setrens(0, 0).expect("init: failed to enter null namespace");

    loop {
        let mut status = 0;
        libredox::call::waitpid(0, &mut status, 0).unwrap();
    }
}
