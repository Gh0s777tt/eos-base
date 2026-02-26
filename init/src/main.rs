use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::ffi::OsString;
use std::io::Result;
use std::path::{Path, PathBuf};

use libredox::flag::{O_RDONLY, O_WRONLY};
use serde::Deserialize;

use crate::script::Command;
use crate::unit::{UnitId, UnitStore};

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

#[derive(Clone, Deserialize)]
struct SwitchRoot {
    prefix: PathBuf,
    etcdir: PathBuf,
}

impl SwitchRoot {
    fn apply(
        self,
        pending_units: &mut VecDeque<UnitId>,
        unit_store: &mut UnitStore,
        config: &mut InitConfig,
    ) {
        config
            .envs
            .insert("PATH".to_owned(), self.prefix.join("bin").into_os_string());
        config.envs.insert(
            "LD_LIBRARY_PATH".to_owned(),
            self.prefix.join("lib").into_os_string(),
        );

        unit_store.config_dirs = vec![
            self.prefix.join("lib").join("init.d"),
            self.etcdir.join("init.d"),
        ];

        let (loaded_units, errors) = unit_store.load_units();
        for error in errors {
            eprintln!("init: {error}");
        }
        pending_units.extend(loaded_units);
    }
}

fn run(
    unit: &UnitId,
    pending_units: &mut VecDeque<UnitId>,
    unit_store: &mut UnitStore,
    config: &mut InitConfig,
) -> Result<()> {
    let unit = unit_store.unit_mut(unit);

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
            if config.log_debug {
                eprintln!(
                    "Starting {}",
                    unit.info.description.as_ref().unwrap_or(&unit.id.0),
                );
            }
            service.spawn(&config.envs);
        }
        unit::UnitKind::Target {} => {
            if config.log_debug {
                eprintln!(
                    "Started target {}",
                    unit.info.description.as_ref().unwrap_or(&unit.id.0),
                );
            }
        }
        unit::UnitKind::SwitchRoot { switchroot } => {
            eprintln!(
                "init: switchroot to {} {}",
                switchroot.prefix.display(),
                switchroot.etcdir.display()
            );
            switchroot.clone().apply(pending_units, unit_store, config);
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
    let mut pending_units = VecDeque::new();

    SwitchRoot {
        prefix: Path::new("/scheme/initfs").to_owned(),
        etcdir: Path::new("/scheme/initfs/etc").to_owned(),
    }
    .apply(&mut pending_units, &mut unit_store, &mut init_config);

    let mut command = std::process::Command::new("logd");
    command.env_clear().envs(&init_config.envs);
    daemon::SchemeDaemon::spawn(command, "log");
    if let Err(err) = switch_stdio("/scheme/log") {
        eprintln!("init: failed to switch stdio to '/scheme/log': {err}");
    }

    let runtime_target = UnitId("00_runtime.target".to_owned());

    'a: while let Some(unit) = pending_units.pop_front() {
        if unit_store.unit(&unit).info.default_dependencies {
            if pending_units.contains(&runtime_target) {
                pending_units.push_back(unit);
                continue 'a;
            }
        }
        for dep in &unit_store.unit(&unit).info.requires_weak {
            if pending_units.contains(dep) {
                pending_units.push_back(unit);
                continue 'a;
            }
        }

        if let Err(err) = run(&unit, &mut pending_units, &mut unit_store, &mut init_config) {
            eprintln!("init: failed to run {}: {}", unit.0, err);
        }
    }

    libredox::call::setrens(0, 0).expect("init: failed to enter null namespace");

    loop {
        let mut status = 0;
        libredox::call::waitpid(0, &mut status, 0).unwrap();
    }
}
