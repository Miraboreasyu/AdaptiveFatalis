use crate::{
    config::Config,
    deploy::Deployer,
    learning::Learning,
    memory,
    model::{Model, Observation},
    session::{ActionSample, CompletedWindow, Tracker},
    storage,
    thk::Project,
};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub fn log(dir: &Path, message: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("runtime.log"))
    {
        let _ = writeln!(f, "{:?} {message}", std::time::SystemTime::now());
    }
}
pub fn project(dir: &Path) -> Result<Project, String> {
    Project::from_paths(
        dir.join("baseline/em013_00.thk"),
        dir.join("baseline/em013_55.thk"),
    )
}
pub fn targets(
    project: &Project,
    learning: &Learning,
    config: &Config,
) -> Result<BTreeMap<String, Vec<i32>>, String> {
    project
        .blocks
        .iter()
        .map(|b| {
            let keys = b.branches.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
            let base = b.branches.iter().map(|x| x.weight).collect::<Vec<_>>();
            Ok((
                b.id.clone(),
                learning.weights(&keys, &base, &config.policy)?,
            ))
        })
        .collect()
}
fn record(window: CompletedWindow, model: &Model, learning: &mut Learning, config: &Config) {
    let s = window.start;
    let a = model.attribute(&Observation {
        distance: s.distance,
        angle_cw: s.angle,
        action_id: s.action,
        phase: None,
        biped: None,
    });
    if !config.share_ambiguous && (!a.exact || a.candidates.len() != 1) {
        return;
    }
    for c in a.candidates {
        learning.observe(
            &c.branch_id,
            window.player_loss_fraction,
            window.monster_loss_fraction,
            c.confidence,
        );
    }
}
fn deploy_generated(
    d: &Deployer,
    project: &Project,
    learning: &Learning,
    config: &Config,
    dir: &Path,
) -> Result<(), String> {
    let global = d.target().with_file_name("em013_55.thk");
    if global.exists()
        && crate::thk::sha256(&fs::read(global).map_err(|e| e.to_string())?)
            != crate::thk::GLOBAL_SHA256
    {
        return Err(
            "existing Global THK conflicts with the fixed model; refusing deployment".into(),
        );
    }
    let weights = targets(project, learning, config)?;
    if d.installed().map_err(|e| e.to_string())?.is_some() {
        d.update(|current| {
            project
                .patch_current_main_weights(current, &weights)
                .map_err(crate::deploy::DeployError::Integrity)
        })
        .map_err(|e| e.to_string())?;
    } else {
        let bytes = project.patch_main_weights(&weights)?;
        let source = dir.join("next.thk");
        storage::save(&source, &bytes)?;
        // Refuse another AI mod instead of silently replacing incompatible behavior.
        if d.target().exists() {
            let current = fs::read(d.target()).map_err(|e| e.to_string())?;
            if current != project.main.bytes {
                return Err("existing nativePC main THK differs from baseline; remove incompatible mod manually".into());
            }
        }
        d.install(&source).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn start_watchdog(
    plugin_dir: &Path,
    root: &Path,
    dir: &Path,
) -> Result<std::process::Child, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let ready = dir.join(format!("watchdog-{}.ready", std::process::id()));
        // A nonce prevents an old readiness file from authorizing deployment.
        let nonce = format!("{:?}", std::time::SystemTime::now());
        let mut child = std::process::Command::new(plugin_dir.join("fatalis-adaptive-tool.exe"))
            .arg("watch")
            .arg(std::process::id().to_string())
            .arg(root)
            .arg(dir)
            .arg(&ready)
            .arg(&nonce)
            .creation_flags(0x08000000)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if fs::read_to_string(&ready).ok().as_deref() == Some(&nonce) {
                return Ok(child);
            }
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                return Err("watchdog exited before ready".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err("watchdog readiness timeout; deployment disabled".into())
    }
    #[cfg(not(windows))]
    {
        let _ = (plugin_dir, root, dir);
        Err("plugin requires Windows".into())
    }
}
struct RestoreOnDrop<'a> {
    deployer: &'a Deployer,
    log_dir: &'a Path,
}
impl Drop for RestoreOnDrop<'_> {
    fn drop(&mut self) {
        if let Err(e) = self.deployer.restore() {
            log(
                self.log_dir,
                &format!("worker cleanup failed (ownership preserved): {e}"),
            );
        }
    }
}
pub fn run(plugin_dir: PathBuf, stop: &AtomicBool) {
    let dir = plugin_dir.join("FatalisAdaptive");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Err(e) = run_inner(&plugin_dir, &dir, stop) {
        log(&dir, &format!("STOPPED: {e}"));
    }
}
fn run_inner(plugin_dir: &Path, dir: &Path, stop: &AtomicBool) -> Result<(), String> {
    let cfg_path = dir.join("config.json");
    if !cfg_path.exists() {
        storage::json(&cfg_path, &crate::strategy_config::Selection::default())?;
    }
    let mut config = crate::strategy_config::load(dir)?;
    config.validate()?;
    let root = plugin_dir
        .parent()
        .and_then(Path::parent)
        .ok_or("expected game/nativePC/plugins")?;
    let state = dir.join("deployment");
    if state.join("ownership.json").exists() || state.join("journal.json").exists() {
        let old = Deployer::new(root, &state).map_err(|e| e.to_string())?;
        old.restore().map_err(|e| e.to_string())?;
        log(dir, "startup: recovered/restored prior owned override");
    }
    if !config.enabled || !config.allow_unverified_offsets {
        log(dir,"disabled: set enabled and allow_unverified_offsets only after reviewing build/offset limitations");
        return Ok(());
    }
    let project = project(dir)?;
    let d = Deployer::new(root, dir.join("deployment")).map_err(|e| e.to_string())?;
    let _cleanup = RestoreOnDrop {
        deployer: &d,
        log_dir: dir,
    };
    let mut watchdog = start_watchdog(plugin_dir, root, &dir.join("deployment"))?;
    d.recover().map_err(|e| e.to_string())?;
    if config.register.is_some() {
        return crate::register_mode::run(dir, &config, stop, &d, &project, &mut watchdog);
    }
    // Legacy approximate runtime has no external-policy contract: never learn implicitly.
    config.policy.orientation = 0.0;
    log(
        dir,
        "built-in no-learning: baseline weights; external policies require register mode",
    );
    let model = Model::from_project(&project);
    log(
        dir,
        &format!(
            "coverage: {:?}; attribution is approximate, not THK execution tracing",
            model.coverage
        ),
    );
    let learning_path = dir.join("learning.json");
    let mut learning: Learning = if learning_path.exists() {
        serde_json::from_slice(&fs::read(&learning_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
    } else {
        Learning::default()
    };
    let mut tracker = Tracker::default();
    let mut active = false;
    let mut hunt_id = None;
    let mut stable_key = None;
    let mut stable_since = Instant::now();
    let mut handled = None;
    let mut diagnostic = Instant::now();
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
        if diagnostic.elapsed() > Duration::from_secs(10) {
            log(
                dir,
                &format!(
                    "snapshot quest={:?} id={:?} stage={:?} player={} monster={}",
                    s.quest_state,
                    s.quest_id,
                    s.stage,
                    s.player_hp.is_some(),
                    s.monster.is_some()
                ),
            );
            diagnostic = Instant::now();
        }
        if s.quest_state == Some(2) && s.stage == Some(417) {
            if hunt_id != s.quest_id {
                tracker.reset();
                active = false;
                hunt_id = s.quest_id;
            }
            if let (Some((pmax, p)), Some(m)) = (s.player_hp, s.monster) {
                let sample = ActionSample {
                    token: m.addr,
                    action: m.action_id,
                    state: m.state_ptr,
                    frame: m.frame,
                    distance: m.ai_distance,
                    angle: m.ai_angle,
                    player: p,
                    player_max: pmax,
                    monster: m.hp.1,
                    monster_max: m.hp.0,
                };
                if let (_, Some(w)) = tracker.tick(sample) {
                    record(w, &model, &mut learning, &config);
                }
                active = true;
            } else {
                tracker.reset();
            } // Never bridge a missing-memory interval.
        } else {
            if active && matches!(s.quest_state, Some(3..=7)) {
                // Record the tail up to the last valid in-quest HP sample, never invent terminal HP=0.
                if let Some(w) = tracker.finish() {
                    record(w, &model, &mut learning, &config);
                }
            }
            tracker.reset();
        }
        if stable_since.elapsed() >= Duration::from_millis(config.stable_ms) && handled != Some(key)
        {
            let ended = matches!(s.quest_state, Some(3..=7));
            if active && ended {
                learning.hunts += 1;
                storage::json(&learning_path, &learning)?;
                active = false;
                hunt_id = None;
                deploy_generated(&d, &project, &learning, &config, dir)?;
                log(dir,"hunt saved and current managed THK weights updated; next-hunt resource reload NOT live-verified");
            }
            if s.stage == Some(0) && s.quest_state != Some(2) {
                if active {
                    storage::json(&learning_path, &learning)?;
                    active = false;
                }
                d.restore().map_err(|e| e.to_string())?;
                log(dir, "stable title: override restored/removed");
            } else if matches!(s.quest_state, Some(0 | 1))
                && s.stage.is_some_and(|x| x != 0 && x != 417)
            {
                deploy_generated(&d, &project, &learning, &config, dir)?;
                log(
                    dir,
                    "outside quest: generated THK deployed (requires load before next hunt)",
                );
            }
            handled = Some(key);
        }
        std::thread::sleep(Duration::from_millis(config.poll_ms));
    }
    storage::json(&learning_path, &learning)?;
    d.restore().map_err(|e| e.to_string())?;
    Ok(())
}
