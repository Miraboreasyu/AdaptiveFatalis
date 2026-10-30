//! External strategy persistence. No hashes; journal IO only at settlement/startup.
use crate::{
    ai_document::AiDocument, config::ScriptLearning, learning::Learning,
    register_learning::CompletedRegisterWindow, script_policy::ScriptPolicy, storage,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Serialize, Deserialize)]
struct Checkpoint {
    #[serde(default)]
    cursor: u64,
    #[serde(default)]
    hunts: u64,
    #[serde(default)]
    state: Value,
}
#[derive(Serialize, Deserialize)]
struct Batch {
    events: Vec<Value>,
}
pub struct PolicyStore {
    engine: ScriptPolicy,
    cfg: ScriptLearning,
    journal: PathBuf,
    checkpoint: PathBuf,
    debug_path: PathBuf,
    saved: Checkpoint,
}
impl PolicyStore {
    pub fn open(dir: &Path, cfg: &ScriptLearning, legacy: &Learning) -> Result<Self, String> {
        Self::open_engine(dir, cfg, legacy, None)
    }
    pub fn open_no_learning(dir: &Path) -> Result<Self, String> {
        let cfg = ScriptLearning {
            script: "builtin:no-learning".into(),
            profile: "builtin-no-learning-v1".into(),
            params: json!({}),
            import_legacy: false,
            debug: false,
        };
        Self::open_engine(
            dir,
            &cfg,
            &Learning::default(),
            Some(ScriptPolicy::no_learning()?),
        )
    }
    fn open_engine(
        dir: &Path,
        cfg: &ScriptLearning,
        legacy: &Learning,
        engine: Option<ScriptPolicy>,
    ) -> Result<Self, String> {
        cfg.validate()?;
        let root = dir.parent().ok_or("missing policy root")?;
        let engine = match engine {
            Some(engine) => engine,
            None => ScriptPolicy::load(&root.join(&cfg.script), cfg.params.clone())?,
        };
        let folder = dir.join("profiles");
        fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let checkpoint = folder.join(format!("{}.json", cfg.profile));
        let saved = if checkpoint.exists() {
            serde_json::from_slice(&fs::read(&checkpoint).map_err(|e| e.to_string())?)
                .map_err(|e| format!("strategy state: {e}"))?
        } else {
            Checkpoint {
                cursor: 0,
                hunts: if cfg.import_legacy { legacy.hunts } else { 0 },
                state: if cfg.import_legacy {
                    json!({"effects":legacy.effects})
                } else {
                    json!({})
                },
            }
        };
        Ok(Self {
            engine,
            cfg: cfg.clone(),
            journal: dir.join("events.jsonl"),
            checkpoint,
            debug_path: folder.join(format!("{}-weights.json", cfg.profile)),
            saved,
        })
    }
    pub fn append(
        &self,
        baseline: &AiDocument,
        windows: &[CompletedRegisterWindow],
    ) -> Result<(), String> {
        let mut events = Vec::with_capacity(windows.len());
        for w in windows {
            let table = baseline.table(&w.table_id).ok_or("unknown event table")?;
            let branch = table
                .branches
                .get(w.exit_index)
                .ok_or("unknown event exit")?;
            if !w.player_loss_fraction.is_finite()
                || w.player_loss_fraction < 0.0
                || !w.monster_loss_fraction.is_finite()
                || w.monster_loss_fraction < 0.0
            {
                return Err("invalid event damage".into());
            }
            events.push(json!({"branch_id":branch.id,"table_id":w.table_id,"exit_index":w.exit_index,
                "player_loss_fraction":w.player_loss_fraction,"monster_loss_fraction":w.monster_loss_fraction,
                "started_at":w.started_at,"ended_at":w.ended_at,"counter":w.counter,
                "player_fainted":null}));
        }
        let mut bytes = serde_json::to_vec(&Batch { events }).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_data())
            .map_err(|e| e.to_string())
    }
    /// Replay only unprocessed journal suffix. Output validation precedes checkpoint commit.
    pub fn refresh(&mut self, baseline: &AiDocument) -> Result<AiDocument, String> {
        let started = std::time::Instant::now();
        let mut next = self.saved.clone();
        if self.journal.exists() {
            let mut file = fs::File::open(&self.journal).map_err(|e| e.to_string())?;
            if file.metadata().map_err(|e| e.to_string())?.len() < next.cursor {
                return Err("event journal shorter than strategy cursor".into());
            }
            file.seek(SeekFrom::Start(next.cursor))
                .map_err(|e| e.to_string())?;
            let mut suffix = String::new();
            file.read_to_string(&mut suffix)
                .map_err(|e| e.to_string())?;
            for line in suffix.split_inclusive('\n') {
                if !line.ends_with('\n') {
                    return Err(
                        "incomplete event journal tail; keep backup and repair tail before retry"
                            .into(),
                    );
                }
                let batch: Batch =
                    serde_json::from_str(line).map_err(|e| format!("event journal: {e}"))?;
                next.state = self.engine.update(&next.state, &json!(batch.events))?;
                next.hunts = next.hunts.checked_add(1).ok_or("hunt overflow")?;
                next.cursor += line.len() as u64;
            }
        }
        let mut document = baseline.clone();
        let mut debug = Vec::new();
        let mut changes = Vec::new();
        let prepared = self.engine.prepare_state(&next.state)?;
        for table in baseline.tables() {
            let base: Vec<i32> = table.branches.iter().map(|b| b.weight).collect();
            let input = json!({"id":table.id,"branches":table.branches.iter().map(|b|json!({"id":b.id,"base_weight":b.weight})).collect::<Vec<_>>()});
            let values = self.engine.weights_prepared(&prepared, &input)?;
            let integers = integer_weights(&base, &values)?;
            document.set_weights(&table.id, &integers)?;
            if integers != base {
                changes.push(format!("{} {:?} -> {:?}", table.id, base, integers));
            }
            if self.cfg.debug && integers != base {
                debug
                    .push(json!({"table":table.id,"base":base,"target":values,"integer":integers}));
            }
        }
        // Write state once, not a checksum or event-log scan every frame.
        storage::json(&self.checkpoint, &next)?;
        self.saved = next;
        let log_root = self
            .journal
            .parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new("."));
        crate::runtime::log(log_root,&format!("strategy weights: profile={} hunts={} changed_tables={} (relative to original baseline)",self.cfg.profile,self.saved.hunts,changes.len()));
        for change in &changes {
            crate::runtime::log(log_root, &format!("strategy weight {change}"));
        }
        if self.cfg.debug {
            if let Err(error) = storage::json(&self.debug_path, &debug) {
                crate::runtime::log(
                    self.journal
                        .parent()
                        .and_then(Path::parent)
                        .unwrap_or(Path::new(".")),
                    &format!("strategy debug output failed (learning committed): {error}"),
                );
            }
        }
        crate::runtime::log(
            log_root,
            &format!(
                "strategy evaluation completed: {} ms (not in frame loop)",
                started.elapsed().as_millis()
            ),
        );
        Ok(document)
    }
    pub fn hunts(&self) -> u64 {
        self.saved.hunts
    }
}
/// Format adapter only: finite nonnegative weights, original zeros, sum 100.
/// Zeroing a formerly-positive arm is allowed: exploration belongs to the script.
pub fn integer_weights(base: &[i32], values: &[f64]) -> Result<Vec<i32>, String> {
    if base.len() != values.len() || base.is_empty() {
        return Err("strategy weight count mismatch".into());
    }
    if values
        .iter()
        .zip(base)
        .any(|(v, b)| !v.is_finite() || *v < 0.0 || (*b == 0 && *v != 0.0))
    {
        return Err("strategy returned invalid or originally-disabled weight".into());
    }
    let largest = values.iter().copied().fold(0.0, f64::max);
    if largest <= 0.0 {
        return Err("strategy returned zero total weight".into());
    }
    let scaled: Vec<f64> = values.iter().map(|v| v / largest).collect();
    let sum: f64 = scaled.iter().sum();
    let normalized: Vec<f64> = scaled.iter().map(|v| 100.0 * v / sum).collect();
    let mut result: Vec<i32> = normalized.iter().map(|v| v.floor() as i32).collect();
    let mut order: Vec<usize> = (0..base.len()).filter(|&i| base[i] > 0).collect();
    order.sort_by(|&a, &b| {
        (normalized[b] - f64::from(result[b]))
            .total_cmp(&(normalized[a] - f64::from(result[a])))
            .then(a.cmp(&b))
    });
    let left = 100 - result.iter().sum::<i32>();
    if left < 0 || left as usize > order.len() {
        return Err("strategy rounding failure".into());
    }
    for i in order.into_iter().take(left as usize) {
        result[i] += 1;
    }
    Ok(result)
}
#[cfg(all(test, feature = "game-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn replay_restart_switch_and_failure_preserve_events() {
        let root = std::env::temp_dir().join(format!(
            "policy-store-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = root.join("register");
        fs::create_dir_all(&dir).unwrap();
        let script = root.join("default.rhai");
        fs::write(&script, crate::script_policy::default_script()).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/fatalis_thk");
        let baseline = AiDocument::parse(
            &fs::read(fixture.join("em013_00.thk")).unwrap(),
            &fs::read(fixture.join("em013_55.thk")).unwrap(),
        )
        .unwrap();
        let cfg = ScriptLearning {
            script: "default.rhai".into(),
            profile: "test".into(),
            params: json!({"direction":1.0,"strength":2.0,"prior_samples":8.0,"exploration":0.25}),
            import_legacy: false,
            debug: true,
        };
        let mut store = PolicyStore::open(&dir, &cfg, &Learning::default()).unwrap();
        assert_eq!(
            store.refresh(&baseline).unwrap().to_thk().unwrap(),
            baseline.to_thk().unwrap()
        );
        let window = CompletedRegisterWindow {
            monster_token: 1,
            counter: 1,
            option_id: 0,
            table_id: "main:n105:s1".into(),
            exit_index: 0,
            started_at: 0,
            ended_at: 1,
            player_loss_fraction: 0.9,
            monster_loss_fraction: 0.0,
        };
        store
            .append(&baseline, std::slice::from_ref(&window))
            .unwrap();
        let learned = store.refresh(&baseline).unwrap();
        assert_eq!(store.hunts(), 1);
        assert!(learned.table("main:n105:s1").unwrap().branches[0].weight > 35);
        drop(store);
        let mut reopened = PolicyStore::open(&dir, &cfg, &Learning::default()).unwrap();
        assert_eq!(
            reopened.refresh(&baseline).unwrap().to_thk().unwrap(),
            learned.to_thk().unwrap()
        );
        assert_eq!(reopened.hunts(), 1);
        let mut friendly = cfg.clone();
        friendly.params["direction"] = json!(-1.0);
        let mut reversed = PolicyStore::open(&dir, &friendly, &Learning::default()).unwrap();
        assert!(
            reversed
                .refresh(&baseline)
                .unwrap()
                .table("main:n105:s1")
                .unwrap()
                .branches[0]
                .weight
                < 35
        );
        let checkpoint = fs::read(dir.join("profiles/test.json")).unwrap();
        fs::write(
            &script,
            "fn update(s,e,p){throw \"bad\";} fn weights(s,t,p){[1.0]}",
        )
        .unwrap();
        let mut broken = PolicyStore::open(&dir, &cfg, &Learning::default()).unwrap();
        broken.append(&baseline, &[window]).unwrap();
        assert!(broken.refresh(&baseline).is_err());
        assert_eq!(
            fs::read(dir.join("profiles/test.json")).unwrap(),
            checkpoint
        );
        fs::write(&script, crate::script_policy::default_script()).unwrap();
        let mut repaired = PolicyStore::open(&dir, &cfg, &Learning::default()).unwrap();
        repaired.refresh(&baseline).unwrap();
        assert_eq!(repaired.hunts(), 2);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn validates_and_rounds() {
        assert_eq!(
            integer_weights(&[30, 0, 70], &[1.0, 0.0, 2.0]).unwrap(),
            vec![33, 0, 67]
        );
        assert!(integer_weights(&[30, 0, 70], &[1.0, 1.0, 2.0]).is_err());
        assert!(integer_weights(&[100], &[f64::NAN]).is_err());
        assert!(integer_weights(&[100], &[0.0]).is_err());
        assert_eq!(
            integer_weights(&[50, 50], &[0.0, 1.0]).unwrap(),
            vec![0, 100]
        );
    }
}
