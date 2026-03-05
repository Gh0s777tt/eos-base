use std::collections::VecDeque;

use crate::InitConfig;
use crate::unit::{UnitId, UnitStore};

pub struct Scheduler {
    pending: VecDeque<Job>,
}

struct Job {
    unit: UnitId,
    kind: JobKind,
}

enum JobKind {
    Start,
}

impl Scheduler {
    pub fn new() -> Scheduler {
        Scheduler {
            pending: VecDeque::new(),
        }
    }

    pub fn schedule_start_and_report_errors(
        &mut self,
        unit_store: &mut UnitStore,
        unit_id: UnitId,
    ) {
        let mut errors = vec![];
        self.schedule_start(unit_store, unit_id, &mut errors);
        for error in errors {
            eprintln!("init: {error}");
        }
    }

    pub fn schedule_start(
        &mut self,
        unit_store: &mut UnitStore,
        unit_id: UnitId,
        errors: &mut Vec<String>,
    ) {
        let loaded_units = unit_store.load_units(unit_id.clone(), errors);
        for unit_id in loaded_units {
            if !unit_store.unit(&unit_id).conditions_met() {
                continue;
            }

            self.pending.push_back(Job {
                unit: unit_id,
                kind: JobKind::Start,
            });
        }
    }

    pub fn step(&mut self, unit_store: &mut UnitStore, init_config: &mut InitConfig) {
        'a: loop {
            let Some(job) = self.pending.pop_front() else {
                return;
            };

            match job.kind {
                JobKind::Start => {
                    let unit = unit_store.unit_mut(&job.unit);

                    for dep in &unit.info.requires_weak {
                        for pending_job in &self.pending {
                            if &pending_job.unit == dep {
                                self.pending.push_back(job);
                                continue 'a;
                            }
                        }
                    }

                    if let Err(err) = crate::run(unit, init_config) {
                        eprintln!("init: failed to run {}: {}", job.unit.0, err);
                    }
                }
            }
        }
    }
}
