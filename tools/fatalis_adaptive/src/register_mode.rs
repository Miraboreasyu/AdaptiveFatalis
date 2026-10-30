//! Exact register-event runtime mode.
//!
//! This mode deploys a structurally instrumented main THK, polls five decimal
//! code registers plus a counter and both HP pairs, and applies one learning
//! update per completed hunt. Register addresses are either derived each poll
//! from the identified monster (monster+0x12278 -> AIData+0xAA8 -> cThinkEm,
//! cThinkEm+0x640+4*slot) with constructor back-reference validation, or
//! explicitly overridden in config. This module never guesses an address or a
//! slot safety.
use crate::{
    ai_document::AiDocument,
    config::{Config, RegisterMode},
    deploy::Deployer,
    learning::{Learning, Policy},
    memory,
    register_learning::{RegisterLog, RegisterSnapshot, RegisterTracker},
    runtime, storage,
    thk::{sha256, Project},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Current editable weights are recomputed from this persisted learning state;
/// the model file is diagnostic, not authoritative.
const LEARNING: &str = "learning.json";
const MODEL: &str = "model.thk";
const MANIFEST: &str = "manifest.json";
const NEXT: &str = "next.thk";

struct Session {
    dir: PathBuf,
    targets: Vec<String>,
    registers: crate::instrument::Registers,
    baseline: AiDocument,
    current: AiDocument,
    tracker: RegisterTracker,
    compiled: crate::instrument::InstrumentedThk,
    compact: bool,
    scripted: Option<crate::policy_store::PolicyStore>,
}
impl Session {
    fn new(
        dir: &Path,
        cfg: &RegisterMode,
        project: &Project,
        _policy: &Policy,
        script: Option<&crate::config::ScriptLearning>,
    ) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let baseline = AiDocument::parse(&project.main.bytes, &project.global.bytes)?;
        let learning_path = dir.join(LEARNING);
        let learning: Learning = if learning_path.exists() {
            serde_json::from_slice(&fs::read(&learning_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        } else {
            Learning::default()
        };
        let mut scripted = Some(match script {
            Some(cfg) => crate::policy_store::PolicyStore::open(dir, cfg, &learning)?,
            None => crate::policy_store::PolicyStore::open_no_learning(dir)?,
        });
        let current = match &mut scripted {
            Some(store) => store.refresh(&baseline)?,
            None => unreachable!("a built-in or external policy is always selected"),
        };
        let targets = if cfg.targets_all() {
            let mut targets = crate::instrument::instrumentable_tables(&baseline)?
                .into_iter()
                .map(|(table, _)| table)
                .collect::<Vec<_>>();
            targets.sort();
            targets
        } else {
            cfg.targets()
        };
        if targets.is_empty() {
            return Err("register target discovery found no instrumentable tables".into());
        }
        let registers = cfg.registers();
        // Experimental opt-in is a separate file, not part of the strategy selection.
        let requested = dir
            .parent()
            .is_some_and(|p| p.join("compact-marker.enabled").is_file());
        let compact = requested
            && match crate::compact_hook::try_install() {
                Ok(()) => {
                    runtime::log(dir.parent().unwrap_or(dir), "compact marker hook enabled");
                    true
                }
                Err(e) => {
                    runtime::log(
                        dir.parent().unwrap_or(dir),
                        &format!("compact marker unavailable; using native fallback: {e}"),
                    );
                    false
                }
            };
        let compiled = if compact {
            crate::instrument::compile_compact(&current, &targets, registers)?
        } else {
            current.to_instrumented_thk(&targets, registers)?
        };
        let tracker = RegisterTracker::new(compiled.manifest.option_map());
        tracker.validate_options(&current)?;
        storage::json(&dir.join(MANIFEST), &compiled.manifest)?;
        storage::save(&dir.join(MODEL), &current.to_thk()?)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            targets,
            registers,
            baseline,
            current,
            tracker,
            compiled,
            compact,
            scripted,
        })
    }
    fn persist_document(&self) -> Result<(), String> {
        storage::save(&self.dir.join(MODEL), &self.current.to_thk()?)
    }
    fn recompile(&mut self) -> Result<(), String> {
        self.compiled = if self.compact {
            crate::instrument::compile_compact(&self.current, &self.targets, self.registers)?
        } else {
            self.current
                .to_instrumented_thk(&self.targets, self.registers)?
        };
        storage::json(&self.dir.join(MANIFEST), &self.compiled.manifest)?;
        Ok(())
    }
    /// Close the observed tail, apply only exact windows, then persist before
    /// deriving the next instrumented bytes. No synthetic HP is invented.
    fn settle(&mut self) -> Result<usize, String> {
        self.tracker.finish();
        let windows = self.tracker.drain_completed();
        if windows.is_empty() {
            return Ok(0);
        }
        if let Some(store) = &mut self.scripted {
            store.append(&self.baseline, &windows)?;
            match store.refresh(&self.baseline) {
                Ok(document) => self.current = document,
                Err(error) => {
                    runtime::log(self.dir.parent().unwrap_or(&self.dir), &format!("strategy update rejected; previous weights kept; events retained for retry: {error}"));
                    return Ok(0);
                }
            }
            let hunts = store.hunts();
            self.persist_document()?;
            self.recompile()?;
            runtime::log(self.dir.parent().unwrap_or(&self.dir), &format!("script strategy settled: {} windows; hunts={hunts}; state saved and THK compiled", windows.len()));
            return Ok(windows.len());
        }
        unreachable!("a built-in or external policy is always selected")
    }
}

/// Install or replace the instrumented main THK. Returns whether bytes changed.
/// Only a baseline-or-owned target is accepted; a foreign AI mod is never replaced.
fn ensure_deployed(d: &Deployer, baseline: &Project, session: &Session) -> Result<bool, String> {
    let global = d.target().with_file_name("em013_55.thk");
    if global.exists()
        && sha256(&fs::read(&global).map_err(|e| e.to_string())?) != crate::thk::GLOBAL_SHA256
    {
        return Err(
            "existing Global THK conflicts with the fixed model; refusing deployment".into(),
        );
    }
    let want = sha256(&session.compiled.bytes);
    if let Some(installed) = d.installed().map_err(|e| e.to_string())? {
        if installed.deployed_hash == want {
            return Ok(false);
        }
        d.update(|_| Ok(session.compiled.bytes.clone()))
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if d.target().exists() {
        let current = fs::read(d.target()).map_err(|e| e.to_string())?;
        if current != baseline.main.bytes {
            return Err(
                "existing nativePC main THK differs from baseline; remove incompatible mod manually"
                    .into(),
            );
        }
    }
    let source = session.dir.join(NEXT);
    storage::save(&source, &session.compiled.bytes)?;
    d.install(&source).map_err(|e| e.to_string())?;
    Ok(true)
}

pub fn run(
    dir: &Path,
    config: &Config,
    stop: &AtomicBool,
    d: &Deployer,
    project: &Project,
    watchdog: &mut std::process::Child,
) -> Result<(), String> {
    let cfg = config
        .register
        .as_ref()
        .ok_or("register mode selected without a register section")?;
    cfg.validate()?;
    let mut session = Session::new(
        &dir.join("register"),
        cfg,
        project,
        &config.policy,
        config.scripted_learning.as_ref(),
    )?;
    if let Some(scripted) = &config.scripted_learning {
        runtime::log(dir, &format!("script strategy loaded: {} profile={} debug={}; compiled once, no frame-loop script execution", scripted.script, scripted.profile, scripted.debug));
    }
    runtime::log(
        dir,
        &format!(
            "register mode: {} target tables, {} instrumented arms, slots table={:?} option={:?} counter={}, addresses {}",
            session.targets.len(),
            session.compiled.manifest.markers.len(),
            session.registers.table_code,
            session.registers.option,
            session.registers.counter,
            cfg.address_mode()
        ),
    );
    if cfg.explicit_addresses().is_none() {
        runtime::log(
            dir,
            "derived register chain: monster+0x12278 -> AIData, AIData+0xAA8 -> cThinkEm, cThinkEm+0x630 == AIData, registers cThinkEm+0x640+4*slot; source-derived, not live-verified",
        );
    }
    runtime::log(
        dir,
        "register mode deploys only outside a quest; a quest already in progress cannot use new instrumented THK bytes",
    );
    let mut active = false;
    let mut hunt_id = None;
    let mut stable_key = None;
    let mut stable_since = Instant::now();
    let mut handled = None;
    let mut left_combat: Option<Instant> = None;
    let started = Instant::now();
    let mut last_diag = Instant::now();
    let mut last_anomaly = Instant::now();
    let mut last_read_log = Instant::now();
    let mut events = 0u64;
    let mut completed = 0u64;
    let mut read_failures = 0u64;
    let mut context_failures = 0u64;
    let mut context_logged = false;
    let mut first_pair_logged = false;
    while !stop.load(Ordering::Relaxed) {
        if watchdog.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("watchdog lost; stop deployment and sampling".into());
        }
        let s = memory::read_snapshot();
        let key = (s.quest_state, s.quest_id, s.stage);
        if stable_key != Some(key) {
            stable_key = Some(key);
            stable_since = Instant::now();
            handled = None;
        }
        let in_combat = s.quest_state == Some(2) && s.stage == Some(417);
        if in_combat {
            left_combat = None;
            if hunt_id != s.quest_id {
                hunt_id = s.quest_id;
            }
            if let (Some((pmax, p)), Some(m)) = (s.player_hp, s.monster.as_ref()) {
                let slots = cfg.slots();
                let resolved: Option<Vec<u64>> = match cfg.explicit_addresses() {
                    Some(addresses) => addresses
                        .iter()
                        .map(crate::config::RegisterAddress::resolve)
                        .collect(),
                    None => memory::resolve_register_addresses(m.addr, &slots),
                };
                if let Some(addresses) = &resolved {
                    if !context_logged {
                        runtime::log(
                            dir,
                            &format!(
                                "register context resolved: monster={:#x} code=({:#x},{:#x},{:#x},{:#x},{:#x}) counter={:#x} mode={} slots={:?}",
                                m.addr,
                                addresses[0],
                                addresses[1],
                                addresses[2],
                                addresses[3],
                                addresses[4],
                                addresses[5],
                                cfg.address_mode(),
                                slots
                            ),
                        );
                        context_logged = true;
                    }
                } else {
                    context_failures += 1;
                }
                let pair = resolved.as_deref().and_then(|addresses| {
                    if session.compact {
                        // Slots are fixed in compact mode; resolve from the same validated context.
                        let context = addresses[0].checked_sub(0x640 + 14 * 4)?;
                        crate::compact_hook::read_code(context)
                    } else {
                        let code: &[u64; 5] = addresses[..5].try_into().ok()?;
                        memory::read_register_code(code, addresses[5])
                    }
                });
                match pair {
                    Some((counter, option_id)) => {
                        if !first_pair_logged {
                            runtime::log(
                                dir,
                                &format!(
                                    "first register read: counter={counter} option_id={option_id}"
                                ),
                            );
                            first_pair_logged = true;
                        }
                        let timestamp =
                            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                        let logs = session.tracker.push(Some(RegisterSnapshot {
                            monster_token: m.addr,
                            counter,
                            option_id,
                            player_hp: p,
                            player_max_hp: pmax,
                            monster_hp: m.hp.1,
                            monster_max_hp: m.hp.0,
                            timestamp,
                        }));
                        for log in &logs {
                            match log {
                                RegisterLog::Opened { .. } => events += 1,
                                RegisterLog::Completed { .. } => completed += 1,
                                RegisterLog::BoundaryPreserved { .. } => {
                                    runtime::log(dir, &format!("register boundary: {log:?}"));
                                }
                                _ if last_anomaly.elapsed() >= Duration::from_secs(5) => {
                                    runtime::log(dir, &format!("register anomaly: {log:?}"));
                                    last_anomaly = Instant::now();
                                }
                                _ => {}
                            }
                        }
                        active = true;
                    }
                    None => {
                        read_failures += 1;
                        if last_read_log.elapsed() >= Duration::from_secs(10) {
                            runtime::log(
                                dir,
                                &format!(
                                    "register sample failed: context={context_failures} read={read_failures} mode={} slots={:?}; derived mode requires readable monster/AIData/cThinkEm backrefs",
                                    cfg.address_mode(),
                                    slots
                                ),
                            );
                            last_read_log = Instant::now();
                        }
                    }
                }
            } else {
                // Combat tuple visible but HP missing: drop the window rather than
                // charge an unknown interval to an option.
                session.tracker.push(None);
            }
        } else if active {
            let since = *left_combat.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(config.stable_ms) {
                let settled = session.settle()?;
                active = false;
                left_combat = None;
                runtime::log(
                    dir,
                    &format!("register mode: combat data ended; settled {settled} windows"),
                );
            }
        }
        if stable_since.elapsed() >= Duration::from_millis(config.stable_ms) && handled != Some(key)
        {
            if matches!(s.quest_state, Some(3..=7)) {
                if active {
                    let settled = session.settle()?;
                    active = false;
                    runtime::log(
                        dir,
                        &format!("register mode: quest ended; settled {settled} windows"),
                    );
                }
                let changed = ensure_deployed(d, project, &session)?;
                runtime::log(
                    dir,
                    if changed {
                        "hunt saved; instrumented THK deployed for the next resource load (not live-verified)"
                    } else {
                        "hunt end observed; deployed instrumented THK is already current"
                    },
                );
            }
            if s.stage == Some(0) && s.quest_state != Some(2) {
                if active {
                    let settled = session.settle()?;
                    active = false;
                    runtime::log(
                        dir,
                        &format!("register mode: title transition; settled {settled} windows"),
                    );
                }
                // Register mode keeps the instrumented THK on disk across the title
                // screen: restoring it here would make the next quest load the
                // uninstrumented bytecode and produce zero events.
                let changed = ensure_deployed(d, project, &session)?;
                runtime::log(
                    dir,
                    if changed {
                        "stable title: instrumented THK (re)deployed for the next quest"
                    } else {
                        "stable title: instrumented THK already current"
                    },
                );
            } else if matches!(s.quest_state, Some(0 | 1))
                && s.stage.is_some_and(|x| x != 0 && x != 417)
                && ensure_deployed(d, project, &session)?
            {
                runtime::log(
                    dir,
                    "outside quest: instrumented THK deployed (requires load before next hunt)",
                );
            }
            handled = Some(key);
        }
        if last_diag.elapsed() >= Duration::from_secs(10) {
            runtime::log(
                dir,
                &format!(
                    "register snapshot quest={:?} id={:?} stage={:?} combat={in_combat} player={} monster={} events={events} completed={completed} failed_reads={read_failures} failed_context={context_failures}",
                    s.quest_state,
                    s.quest_id,
                    s.stage,
                    s.player_hp.is_some(),
                    s.monster.is_some()
                ),
            );
            last_diag = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(config.poll_ms));
    }
    if active {
        let settled = session.settle()?;
        runtime::log(
            dir,
            &format!("register mode: stopping; settled {settled} windows"),
        );
    }
    Ok(())
}

#[cfg(all(test, feature = "game-fixtures"))]
mod tests {
    use super::*;
    use crate::config::RegisterMode;

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!(
            "fatalis-register-session-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn session_persists_learning_and_recompiles_from_register_events() {
        let root = scratch();
        let research = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/fatalis_thk");
        let project =
            Project::from_paths(research.join("em013_00.thk"), research.join("em013_55.thk"))
                .unwrap();
        let cfg = RegisterMode {
            table_code_slots: [14, 15, 16],
            option_slots: [17, 18],
            counter_slot: 19,
            target_tables: vec!["main:n105:s1".into()],
            addresses: None,
        };
        let policy = Policy::default();
        let mut session = Session::new(&root, &cfg, &project, &policy, None).unwrap();
        assert!(root.join(MODEL).is_file());
        assert!(root.join(MANIFEST).is_file());
        assert!(
            !root.join(LEARNING).exists(),
            "default learning is only persisted by settle"
        );
        let marker = session.compiled.manifest.markers[0].clone();
        let before =
            session.current.table(&marker.table_id).unwrap().branches[marker.exit_index].weight;
        for counter in 1..=40u32 {
            let base = u64::from(counter) * 10;
            for (hp, timestamp) in [(100.0, base), (20.0, base + 1)] {
                session.tracker.push(Some(RegisterSnapshot {
                    monster_token: 1,
                    counter,
                    option_id: marker.id,
                    player_hp: hp,
                    player_max_hp: 100.0,
                    monster_hp: 1000.0,
                    monster_max_hp: 1000.0,
                    timestamp,
                }));
            }
        }
        assert_eq!(session.settle().unwrap(), 40);
        let after =
            session.current.table(&marker.table_id).unwrap().branches[marker.exit_index].weight;
        assert!(
            after == before,
            "built-in no-learning must ignore even repeated player damage"
        );
        assert_eq!(session.scripted.as_ref().unwrap().hunts(), 1);
        let map = session.compiled.manifest.option_map();
        let reloaded = Session::new(&root, &cfg, &project, &policy, None).unwrap();
        assert_eq!(reloaded.scripted.as_ref().unwrap().hunts(), 1);
        assert_eq!(
            reloaded.current.table(&marker.table_id).unwrap().branches[marker.exit_index].weight,
            after,
            "learned weights must come from persisted learning, not model.thk"
        );
        assert_eq!(reloaded.compiled.manifest.option_map(), map);
        fs::remove_dir_all(&root).unwrap();
    }
}
