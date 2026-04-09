use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::io::Result;
use std::path::Path;

use libredox::flag::{O_RDONLY, O_WRONLY};

use crate::scheduler::Scheduler;
use crate::script::Command;
use crate::unit::{Unit, UnitId, UnitStore};

mod scheduler;
mod script;
mod service;
mod unit;

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
    envs: BTreeMap<String, OsString>,
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
            envs: BTreeMap::from([("RUST_BACKTRACE".to_owned(), "1".into())]),
        }
    }
}

fn switch_root(unit_store: &mut UnitStore, config: &mut InitConfig, prefix: &Path, etcdir: &Path) {
    eprintln!(
        "init: switchroot to {} {}",
        prefix.display(),
        etcdir.display()
    );

    config
        .envs
        .insert("PATH".to_owned(), prefix.join("bin").into_os_string());
    config.envs.insert(
        "LD_LIBRARY_PATH".to_owned(),
        prefix.join("lib").into_os_string(),
    );

    unit_store.config_dirs = vec![prefix.join("lib").join("init.d"), etcdir.join("init.d")];
}

fn run(unit: &mut Unit, config: &mut InitConfig) -> Result<()> {
    match &unit.kind {
        unit::UnitKind::LegacyScript { script } => {
            for cmd in script.0.clone() {
                if config.log_debug {
                    eprintln!("init: running: {cmd:?}");
                }
                run_command(cmd, config);
            }
        }
        unit::UnitKind::Service { service } => {
            if config.skip_cmd.contains(&service.cmd) {
                eprintln!("Skipping '{} {}'", service.cmd, service.args.join(" "));
                return Ok(());
            }
            if config.log_debug {
                eprintln!(
                    "Starting {} ({})",
                    unit.info.description.as_ref().unwrap_or(&unit.id.0),
                    service.cmd,
                );
            }
            service.spawn(&config.envs);
        }
        unit::UnitKind::Target {} => {
            if config.log_debug {
                eprintln!(
                    "Reached target {}",
                    unit.info.description.as_ref().unwrap_or(&unit.id.0),
                );
            }
        }
    }

    Ok(())
}

fn run_command(cmd: Command, config: &mut InitConfig) {
    match cmd {
        Command::RequiresWeak(_) => {} // handled by unit parsing code
        Command::Nothing => {}
        Command::Echo(text) => println!("{text}"),
        Command::Service(service) => {
            if config.skip_cmd.contains(&service.cmd) {
                eprintln!(
                    "init: skipping '{} {}'",
                    service.cmd,
                    service.args.join(" ")
                );
                return;
            }

            service.spawn(&config.envs);
        }
    }
}

fn main() {
    let mut init_config = InitConfig::new();
    let mut unit_store = UnitStore::new();
    let mut scheduler = Scheduler::new();

    switch_root(
        &mut unit_store,
        &mut init_config,
        Path::new("/scheme/initfs"),
        Path::new("/scheme/initfs/etc"),
    );

    let runtime_target = UnitId("00_runtime.target".to_owned());
    scheduler.schedule_start_and_report_errors(&mut unit_store, runtime_target.clone());
    unit_store.set_runtime_target(runtime_target);

    scheduler
        .schedule_start_and_report_errors(&mut unit_store, UnitId("90_initfs.target".to_owned()));

    let mut command = std::process::Command::new("logd");
    command.env_clear().envs(&init_config.envs);
    daemon::SchemeDaemon::spawn(command, "log");
    if let Err(err) = switch_stdio("/scheme/log") {
        eprintln!("init: failed to switch stdio to '/scheme/log': {err}");
    }

    scheduler.step(&mut unit_store, &mut init_config);

    switch_root(
        &mut unit_store,
        &mut init_config,
        Path::new("/usr"),
        Path::new("/etc"),
    );
    {
        // FIXME introduce multi-user.target unit and replace the config dir iteration
        // scheduler.schedule_start_and_report_errors(&mut unit_store, UnitId("multi-user.target".to_owned()));

        let entries = match config::config_for_dirs(&unit_store.config_dirs) {
            Ok(entries) => entries,
            Err(err) => {
                eprintln!(
                    "init: failed to read configs from {}: {err}",
                    unit_store
                        .config_dirs
                        .iter()
                        .map(|dir| dir.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                return;
            }
        };
        for entry in entries {
            scheduler.schedule_start_and_report_errors(
                &mut unit_store,
                UnitId(entry.file_name().unwrap().to_str().unwrap().to_owned()),
            );
        }
    };

    scheduler.step(&mut unit_store, &mut init_config);

    libredox::call::setrens(0, 0).expect("init: failed to enter null namespace");

    loop {
        let mut status = 0;
        libredox::call::waitpid(0, &mut status, 0).unwrap();
    }
}
