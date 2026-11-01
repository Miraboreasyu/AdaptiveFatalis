//! Exact register-event attribution, entirely in memory.
//!
//! The instrumentation manifest supplies `OptionMap`; action IDs and geometry
//! are deliberately not used. A nonzero counter identifies an event occurrence,
//! including consecutive occurrences of the same option. Initial counter zero
//! means no event has been emitted. MAX -> zero is accepted as one event (the
//! game's actual overflow behavior is not runtime-verified).
use crate::{
    ai_document::AiDocument,
    learning::{DamageWindow, Learning, Policy},
};
use std::collections::BTreeMap;

/// Manifest option ID -> (random-table ID, zero-based branch/exit index).
pub type OptionMap = BTreeMap<i32, (String, usize)>;

/// One coherent read of the registers and both HP pairs. Timestamp units are
/// caller-defined, but must be monotonic within a monster token.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegisterSnapshot {
    pub monster_token: u64,
    pub counter: u32,
    pub option_id: i32,
    pub player_hp: f64,
    pub player_max_hp: f64,
    pub monster_hp: f64,
    pub monster_max_hp: f64,
    pub timestamp: u64,
}
impl RegisterSnapshot {
    fn valid(&self) -> bool {
        [
            (self.player_hp, self.player_max_hp),
            (self.monster_hp, self.monster_max_hp),
        ]
        .iter()
        .all(|(hp, max)| hp.is_finite() && max.is_finite() && *hp >= 0.0 && *max > 0.0 && hp <= max)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompletedRegisterWindow {
    pub monster_token: u64,
    pub counter: u32,
    pub option_id: i32,
    pub table_id: String,
    pub exit_index: usize,
    pub started_at: u64,
    pub ended_at: u64,
    /// Sum of negative HP deltas, divided by the opening maximum HP.
    /// Healing never subtracts an earlier hit. Can exceed one before Learning's cap.
    pub player_loss_fraction: f64,
    pub monster_loss_fraction: f64,
    pub end_reason: &'static str,
    pub part_threshold: bool,
    pub player_hp_reached_zero: bool,
    /// Unknown unless backed by a separately verified observation source.
    pub player_fainted: Option<bool>,
}

/// Structured, explicit diagnostics; callers can format/persist these as desired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterLog {
    Opened {
        counter: u32,
        option_id: i32,
    },
    Completed {
        counter: u32,
        option_id: i32,
    },
    ReadFailed,
    InvalidSnapshot,
    TokenChanged {
        previous: u64,
        current: u64,
    },
    CounterReset {
        previous: u32,
        current: u32,
    },
    /// The ENTIRE active window and the unobserved interval were discarded.
    CounterJumpDiscarded {
        previous: u32,
        current: u32,
        missed_events: u32,
    },
    TimestampRegressed {
        previous: u64,
        current: u64,
    },
    UnknownOption {
        counter: u32,
        option_id: i32,
    },
    /// An ID change without a new counter is incoherent, not a new event.
    IncoherentOption {
        counter: u32,
        previous: i32,
        current: i32,
    },
    MaximumHpChanged,
    /// Reliable prefix retained; the changed snapshot is NOT charged as damage.
    BoundaryPreserved {
        detail: String,
    },
}

#[derive(Clone, Debug)]
struct ActiveWindow {
    start: RegisterSnapshot,
    end: u64,
    table_id: String,
    exit_index: usize,
    damage: DamageWindow,
    player_hp_reached_zero: bool,
}

#[derive(Clone, Debug)]
pub struct RegisterTracker {
    options: OptionMap,
    last: Option<RegisterSnapshot>,
    active: Option<ActiveWindow>,
    completed: Vec<CompletedRegisterWindow>,
}
impl RegisterTracker {
    pub fn new(options: OptionMap) -> Self {
        Self {
            options,
            last: None,
            active: None,
            completed: Vec::new(),
        }
    }

    /// Validate every manifest destination before starting a hunt (optional).
    pub fn validate_options(&self, document: &AiDocument) -> Result<(), String> {
        for (id, (table_id, exit_index)) in &self.options {
            let table = document
                .table(table_id)
                .ok_or_else(|| format!("option {id}: unknown table {table_id}"))?;
            if *exit_index >= table.branches.len() {
                return Err(format!(
                    "option {id}: invalid exit index {exit_index} in {table_id}"
                ));
            }
        }
        Ok(())
    }

    pub fn completed(&self) -> &[CompletedRegisterWindow] {
        &self.completed
    }

    pub fn drain_completed(&mut self) -> Vec<CompletedRegisterWindow> {
        std::mem::take(&mut self.completed)
    }

    /// `None` represents any failed/incoherent read. It drops the active window
    /// without assigning damage and retains event identity: a later read of the
    /// same counter MUST NOT reopen a partially observed event.
    pub fn push(&mut self, snapshot: Option<RegisterSnapshot>) -> Vec<RegisterLog> {
        let Some(s) = snapshot else {
            self.active = None;
            return vec![RegisterLog::ReadFailed];
        };
        if !s.valid() {
            self.active = None;
            return vec![RegisterLog::InvalidSnapshot];
        }
        let mut logs = Vec::new();
        let previous = self.last;
        self.last = Some(s);
        if let Some(p) = previous {
            if p.monster_token != s.monster_token {
                self.preserve_boundary("monster_token_changed", &s, &mut logs);
                logs.push(RegisterLog::TokenChanged {
                    previous: p.monster_token,
                    current: s.monster_token,
                });
            } else if s.timestamp < p.timestamp {
                self.active = None;
                logs.push(RegisterLog::TimestampRegressed {
                    previous: p.timestamp,
                    current: s.timestamp,
                });
                return logs;
            } else if s.counter < p.counter && !(p.counter == u32::MAX && s.counter == 0) {
                self.preserve_boundary("counter_reset", &s, &mut logs);
                logs.push(RegisterLog::CounterReset {
                    previous: p.counter,
                    current: s.counter,
                });
            } else if s.counter == p.counter {
                if s.option_id != p.option_id {
                    self.active = None;
                    logs.push(RegisterLog::IncoherentOption {
                        counter: s.counter,
                        previous: p.option_id,
                        current: s.option_id,
                    });
                } else if let Some(w) = &mut self.active {
                    if s.player_max_hp != w.start.player_max_hp
                        || s.monster_max_hp != w.start.monster_max_hp
                    {
                        self.preserve_boundary("maximum_hp_changed", &s, &mut logs);
                        logs.push(RegisterLog::MaximumHpChanged);
                    } else {
                        w.player_hp_reached_zero |= p.player_hp > 0.0 && s.player_hp == 0.0;
                        w.damage.tick(s.player_hp, s.monster_hp);
                        w.end = s.timestamp;
                    }
                }
                return logs;
            } else if s.counter.wrapping_sub(p.counter) > 1 {
                self.active = None;
                logs.push(RegisterLog::CounterJumpDiscarded {
                    previous: p.counter,
                    current: s.counter,
                    missed_events: s.counter.wrapping_sub(p.counter) - 1,
                });
            } else if !self.options.contains_key(&s.option_id) {
                // Do NOT first charge this boundary's HP loss to the previous ID.
                self.active = None;
            } else if let Some(w) = &mut self.active {
                if s.player_max_hp != w.start.player_max_hp
                    || s.monster_max_hp != w.start.monster_max_hp
                {
                    self.preserve_boundary("maximum_hp_changed", &s, &mut logs);
                    logs.push(RegisterLog::MaximumHpChanged);
                } else {
                    w.player_hp_reached_zero |= p.player_hp > 0.0 && s.player_hp == 0.0;
                    w.damage.tick(s.player_hp, s.monster_hp);
                    w.end = s.timestamp;
                    self.close(&mut logs);
                }
            }
        }
        let wrapped =
            previous.is_some_and(|p| p.monster_token == s.monster_token && p.counter == u32::MAX);
        if s.counter == 0 && !wrapped {
            return logs;
        }
        match self.options.get(&s.option_id) {
            Some((table_id, exit_index)) => {
                self.active = Some(ActiveWindow {
                    start: s,
                    end: s.timestamp,
                    table_id: table_id.clone(),
                    exit_index: *exit_index,
                    damage: DamageWindow::new(s.player_hp, s.monster_hp),
                    player_hp_reached_zero: false,
                });
                logs.push(RegisterLog::Opened {
                    counter: s.counter,
                    option_id: s.option_id,
                });
            }
            None => {
                self.active = None;
                logs.push(RegisterLog::UnknownOption {
                    counter: s.counter,
                    option_id: s.option_id,
                });
            }
        }
        logs
    }

    fn preserve_boundary(
        &mut self,
        reason: &'static str,
        next: &RegisterSnapshot,
        logs: &mut Vec<RegisterLog>,
    ) {
        if let Some(w) = &self.active {
            logs.push(RegisterLog::BoundaryPreserved { detail: format!(
                "reason={reason} table={} exit={} option={} counter={} observed={}..{} player_loss={} monster_loss={} opening={:?} observed_damage={:?} next={:?}; changed snapshot excluded",
                w.table_id, w.exit_index, w.start.option_id, w.start.counter,
                w.start.timestamp, w.end, w.damage.player_loss / w.start.player_max_hp,
                w.damage.monster_loss / w.start.monster_max_hp, w.start, w.damage, next
            ) });
        }
        self.close_reason(logs, reason, false);
    }

    fn close(&mut self, logs: &mut Vec<RegisterLog>) {
        self.close_reason(logs, "next_marker", false);
    }
    fn close_reason(&mut self, logs: &mut Vec<RegisterLog>, reason: &'static str, flinch: bool) {
        if let Some(w) = self.active.take() {
            self.completed.push(CompletedRegisterWindow {
                monster_token: w.start.monster_token,
                counter: w.start.counter,
                option_id: w.start.option_id,
                table_id: w.table_id,
                exit_index: w.exit_index,
                started_at: w.start.timestamp,
                ended_at: w.end,
                player_loss_fraction: w.damage.player_loss / w.start.player_max_hp,
                monster_loss_fraction: w.damage.monster_loss / w.start.monster_max_hp,
                end_reason: reason,
                part_threshold: flinch,
                player_hp_reached_zero: w.player_hp_reached_zero,
                player_fainted: None,
            });
            logs.push(RegisterLog::Completed {
                counter: w.start.counter,
                option_id: w.start.option_id,
            });
        }
    }

    /// Finish only the already observed final window; no synthetic final HP read.
    /// Repeated calls are harmless. A new hunt should use a fresh tracker.
    pub fn finish(&mut self) -> Vec<RegisterLog> {
        let mut logs = Vec::new();
        self.close_reason(&mut logs, "observation_ended", false);
        logs
    }

    /// Caller supplies a verified part-flinch event AFTER pushing the coherent
    /// HP snapshot. No same-counter reopening; post-flinch damage is excluded.
    pub fn part_threshold(&mut self) -> Vec<RegisterLog> {
        let mut logs = Vec::new();
        self.close_reason(&mut logs, "part_threshold", true);
        logs
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HuntUpdate {
    pub observed_windows: usize,
    pub table_weights: BTreeMap<String, Vec<i32>>,
}

/// Apply exact samples with confidence 1, then independently recompute each
/// table using Learning's existing policy and an IMMUTABLE original baseline.
/// No sharing across plausible tables; no compounding current adapted weights.
/// All validation and updates happen on clones, so failure leaves both outputs
/// untouched. Each successful call represents ONE hunt; callers must not replay it.
pub fn apply_hunt(
    baseline: &AiDocument,
    current: &mut AiDocument,
    learning: &mut Learning,
    policy: &Policy,
    windows: &[CompletedRegisterWindow],
) -> Result<HuntUpdate, String> {
    policy.validate()?;
    let mut next_learning = learning.clone();
    let mut next_document = current.clone();
    for w in windows {
        if w.ended_at < w.started_at
            || !w.player_loss_fraction.is_finite()
            || w.player_loss_fraction < 0.0
            || !w.monster_loss_fraction.is_finite()
            || w.monster_loss_fraction < 0.0
        {
            return Err("invalid completed register window".into());
        }
        let table = baseline
            .table(&w.table_id)
            .ok_or_else(|| format!("unknown table {}", w.table_id))?;
        let branch = table
            .branches
            .get(w.exit_index)
            .ok_or_else(|| format!("invalid exit index {} in {}", w.exit_index, w.table_id))?;
        next_learning.observe(
            &branch.id,
            w.player_loss_fraction,
            w.monster_loss_fraction,
            1.0,
        );
    }
    let table_weights = recompute_into(baseline, &mut next_document, &next_learning, policy)?;
    next_learning.hunts = next_learning
        .hunts
        .checked_add(1)
        .ok_or_else(|| "hunt counter overflow".to_string())?;
    *learning = next_learning;
    *current = next_document;
    Ok(HuntUpdate {
        observed_windows: windows.len(),
        table_weights,
    })
}

/// Recompute every table from the immutable baseline and the persisted
/// learning state. This is the startup path: no model file is authoritative,
/// and no hunt is counted. Failure leaves no partial document.
pub fn recompute(
    baseline: &AiDocument,
    learning: &Learning,
    policy: &Policy,
) -> Result<AiDocument, String> {
    policy.validate()?;
    let mut document = baseline.clone();
    recompute_into(baseline, &mut document, learning, policy)?;
    Ok(document)
}

fn recompute_into(
    baseline: &AiDocument,
    current: &mut AiDocument,
    learning: &Learning,
    policy: &Policy,
) -> Result<BTreeMap<String, Vec<i32>>, String> {
    let mut table_weights = BTreeMap::new();
    if current.tables().len() != baseline.tables().len() {
        return Err("current/baseline table count differs".into());
    }
    for table in baseline.tables() {
        let existing = current
            .table(&table.id)
            .ok_or_else(|| format!("current document missing table {}", table.id))?;
        if existing.branches.len() != table.branches.len()
            || existing
                .branches
                .iter()
                .zip(&table.branches)
                .any(|(a, b)| a.id != b.id)
        {
            return Err(format!("current/baseline exits differ for {}", table.id));
        }
        let keys = table
            .branches
            .iter()
            .map(|b| b.id.clone())
            .collect::<Vec<_>>();
        let base = table.branches.iter().map(|b| b.weight).collect::<Vec<_>>();
        let weights = learning.weights(&keys, &base, policy)?;
        current.set_weights(&table.id, &weights)?;
        table_weights.insert(table.id.clone(), weights);
    }
    Ok(table_weights)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tracker() -> RegisterTracker {
        RegisterTracker::new(BTreeMap::from([
            (7, ("main:n105:s1".into(), 0)),
            (8, ("main:n105:s11".into(), 1)),
        ]))
    }
    fn s(counter: u32, option_id: i32, player: f64, monster: f64) -> RegisterSnapshot {
        RegisterSnapshot {
            monster_token: 9,
            counter,
            option_id,
            player_hp: player,
            player_max_hp: 100.0,
            monster_hp: monster,
            monster_max_hp: 1000.0,
            timestamp: u64::from(counter) * 100,
        }
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-10, "{a} != {b}");
    }

    #[test]
    fn part_threshold_closes_and_excludes_followup_until_next_marker() {
        let mut t = tracker();
        t.push(Some(s(1, 7, 100.0, 1000.0)));
        let mut hit = s(1, 7, 80.0, 900.0);
        hit.timestamp = 150;
        t.push(Some(hit));
        t.part_threshold();
        let mut followup = s(1, 7, 60.0, 500.0);
        followup.timestamp = 170;
        t.push(Some(followup));
        t.push(Some(s(2, 8, 60.0, 500.0)));
        assert_eq!(t.completed().len(), 1);
        assert_eq!(t.completed()[0].end_reason, "part_threshold");
        assert!(t.completed()[0].part_threshold);
        assert_eq!(t.completed()[0].ended_at - t.completed()[0].started_at, 50);
        near(t.completed()[0].monster_loss_fraction, 0.1);
    }

    #[test]
    fn zero_hp_flag_survives_respawn_boundary_and_is_not_invented() {
        let mut t = tracker();
        t.push(Some(s(1, 7, 100.0, 1000.0)));
        t.push(Some(s(1, 7, 0.0, 1000.0)));
        let mut respawn = s(1, 7, 150.0, 1000.0);
        respawn.player_max_hp = 150.0;
        t.push(Some(respawn));
        assert!(t.completed()[0].player_hp_reached_zero);
        let mut t = tracker();
        t.push(Some(s(1, 7, 100.0, 1000.0)));
        t.push(None);
        t.push(Some(s(2, 8, 0.0, 1000.0)));
        t.finish();
        assert!(!t.completed()[0].player_hp_reached_zero);
    }
    #[test]
    fn same_id_new_counter_settles_and_healing_does_not_erase_hits() {
        let mut t = tracker();
        t.push(Some(s(1, 7, 100.0, 1000.0)));
        t.push(Some(s(1, 7, 50.0, 800.0)));
        t.push(Some(s(1, 7, 90.0, 900.0)));
        let logs = t.push(Some(s(2, 7, 80.0, 850.0)));
        assert!(logs.contains(&RegisterLog::Completed {
            counter: 1,
            option_id: 7
        }));
        assert_eq!(t.completed().len(), 1);
        near(t.completed()[0].player_loss_fraction, 0.6);
        near(t.completed()[0].monster_loss_fraction, 0.25);
        t.finish();
        assert_eq!(t.completed().len(), 2);
        near(t.completed()[1].player_loss_fraction, 0.0);
    }
    #[test]
    fn maximum_hp_boundary_preserves_prefix_not_respawn_delta() {
        for advance in [false, true] {
            let mut t = tracker();
            t.push(Some(s(1, 7, 100.0, 1000.0)));
            let mut hit = s(1, 7, 5.0, 900.0);
            hit.timestamp = 110;
            t.push(Some(hit));
            let mut boundary = s(if advance { 2 } else { 1 }, 7, 0.0, 800.0);
            boundary.timestamp = 120;
            boundary.player_max_hp = 150.0;
            let logs = t.push(Some(boundary));
            assert!(logs
                .iter()
                .any(|x| matches!(x, RegisterLog::BoundaryPreserved { .. })));
            assert_eq!(t.completed().len(), 1);
            near(t.completed()[0].player_loss_fraction, 0.95);
            near(t.completed()[0].monster_loss_fraction, 0.1);
            assert_eq!(t.completed()[0].ended_at, 110);
            if !advance {
                boundary.player_hp = 150.0;
                boundary.timestamp = 130;
                t.push(Some(boundary));
                t.finish();
                assert_eq!(t.completed().len(), 1);
            }
        }
    }

    #[test]
    fn no_damage_before_first_event() {
        let mut t = tracker();
        t.push(Some(s(0, 0, 100.0, 1000.0)));
        t.push(Some(s(0, 0, 10.0, 200.0)));
        t.push(Some(s(1, 7, 5.0, 100.0)));
        t.push(Some(s(2, 8, 3.0, 90.0)));
        near(t.completed()[0].player_loss_fraction, 0.02);
        near(t.completed()[0].monster_loss_fraction, 0.01);
    }
    #[test]
    fn failed_read_drops_and_same_counter_cannot_resume() {
        let mut t = tracker();
        t.push(Some(s(1, 7, 100.0, 1000.0)));
        t.push(Some(s(1, 7, 70.0, 800.0)));
        assert_eq!(t.push(None), vec![RegisterLog::ReadFailed]);
        t.push(Some(s(1, 7, 30.0, 300.0)));
        t.push(Some(s(2, 8, 20.0, 200.0)));
        assert!(t.completed().is_empty());
        t.push(Some(s(3, 7, 10.0, 100.0)));
        assert_eq!(t.completed()[0].option_id, 8);
        near(t.completed()[0].player_loss_fraction, 0.1);
    }
    #[test]
    fn jump_discards_entire_ambiguous_interval_with_explicit_log() {
        let mut t = tracker();
        t.push(Some(s(1, 7, 100.0, 1000.0)));
        t.push(Some(s(1, 7, 90.0, 900.0)));
        let logs = t.push(Some(s(4, 7, 40.0, 400.0)));
        assert!(logs.contains(&RegisterLog::CounterJumpDiscarded {
            previous: 1,
            current: 4,
            missed_events: 2
        }));
        assert!(t.completed().is_empty());
        t.push(Some(s(5, 8, 30.0, 300.0)));
        assert_eq!(t.completed()[0].counter, 4);
        near(t.completed()[0].player_loss_fraction, 0.1);
    }
    #[test]
    fn unknown_id_never_charges_preceding_option() {
        let mut t = tracker();
        t.push(Some(s(1, 7, 100.0, 1000.0)));
        assert!(t
            .push(Some(s(2, 999, 20.0, 200.0)))
            .contains(&RegisterLog::UnknownOption {
                counter: 2,
                option_id: 999
            }));
        t.push(Some(s(3, 8, 10.0, 100.0)));
        assert!(t.completed().is_empty());
        t.push(Some(s(4, 7, 5.0, 50.0)));
        assert_eq!(t.completed()[0].option_id, 8);
        near(t.completed()[0].player_loss_fraction, 0.05);
    }
    #[test]
    fn token_change_and_counter_reset_do_not_bridge() {
        let mut t = tracker();
        t.push(Some(s(8, 7, 100.0, 1000.0)));
        t.push(Some(s(8, 7, 5.0, 900.0)));
        let mut next = s(1, 8, 40.0, 400.0);
        next.monster_token = 10;
        assert!(t.push(Some(next)).contains(&RegisterLog::TokenChanged {
            previous: 9,
            current: 10
        }));
        assert_eq!(t.completed().len(), 1);
        near(t.completed()[0].player_loss_fraction, 0.95);
        near(t.completed()[0].monster_loss_fraction, 0.1);
        next.player_hp = 10.0;
        t.push(Some(next));
        next.counter = 0;
        assert!(t.push(Some(next)).contains(&RegisterLog::CounterReset {
            previous: 1,
            current: 0
        }));
        t.finish();
        assert_eq!(t.completed().len(), 2);
        near(t.completed()[1].player_loss_fraction, 0.3);
    }
    #[test]
    fn id_alone_never_creates_an_event_and_invalid_hp_breaks_window() {
        let mut t = tracker();
        t.push(Some(s(1, 7, 100.0, 1000.0)));
        assert!(t
            .push(Some(s(1, 8, 90.0, 900.0)))
            .contains(&RegisterLog::IncoherentOption {
                counter: 1,
                previous: 7,
                current: 8
            }));
        t.push(Some(s(2, 8, 80.0, 800.0)));
        assert!(t.completed().is_empty());
        assert_eq!(
            t.push(Some(s(2, 8, f64::NAN, 700.0))),
            vec![RegisterLog::InvalidSnapshot]
        );
        t.push(Some(s(3, 7, 60.0, 600.0)));
        assert!(t.completed().is_empty());
    }
    #[cfg(feature = "game-fixtures")]
    #[test]
    fn recompute_matches_apply_hunt_without_counting_a_hunt() {
        const MAIN: &[u8] = include_bytes!("../../../research/fatalis_thk/em013_00.thk");
        const GLOBAL: &[u8] = include_bytes!("../../../research/fatalis_thk/em013_55.thk");
        let baseline = AiDocument::parse(MAIN, GLOBAL).unwrap();
        let key = baseline.table("main:n105:s1").unwrap().branches[0]
            .id
            .clone();
        let mut learning = Learning::default();
        learning.observe(&key, 0.9, 0.1, 1.0);
        let document = recompute(&baseline, &learning, &Policy::default()).unwrap();
        assert_eq!(learning.hunts, 0);
        let mut applied = baseline.clone();
        let mut counted = learning.clone();
        apply_hunt(
            &baseline,
            &mut applied,
            &mut counted,
            &Policy::default(),
            &[],
        )
        .unwrap();
        assert_eq!(counted.hunts, 1);
        assert_eq!(document.to_thk().unwrap(), applied.to_thk().unwrap());
    }

    #[cfg(feature = "game-fixtures")]
    #[test]
    fn hunt_uses_exact_confidence_and_independent_tables_without_compounding() {
        const MAIN: &[u8] = include_bytes!("../../../research/fatalis_thk/em013_00.thk");
        const GLOBAL: &[u8] = include_bytes!("../../../research/fatalis_thk/em013_55.thk");
        let baseline = AiDocument::parse(MAIN, GLOBAL).unwrap();
        let mut current = baseline.clone();
        let mut learning = Learning::default();
        let mut t = tracker();
        t.validate_options(&baseline).unwrap();
        t.push(Some(s(1, 7, 100.0, 1000.0)));
        t.push(Some(s(2, 8, 10.0, 1000.0)));
        // Only first window is committed, so the second table has no observation.
        let result = apply_hunt(
            &baseline,
            &mut current,
            &mut learning,
            &Policy::default(),
            t.completed(),
        )
        .unwrap();
        assert_eq!(result.observed_windows, 1);
        let key = &baseline.table("main:n105:s1").unwrap().branches[0].id;
        assert_eq!(learning.effects.len(), 1);
        assert_eq!(learning.effects[key].samples, 1.0);
        near(learning.effects[key].player_loss, 0.9);
        let other = "main:n105:s11";
        assert_eq!(
            result.table_weights[other],
            baseline
                .table(other)
                .unwrap()
                .branches
                .iter()
                .map(|b| b.weight)
                .collect::<Vec<_>>()
        );
        assert!(
            current.table("main:n105:s1").unwrap().branches[0].weight
                > baseline.table("main:n105:s1").unwrap().branches[0].weight
        );
        let before = current.to_thk().unwrap();
        apply_hunt(
            &baseline,
            &mut current,
            &mut learning,
            &Policy::default(),
            &[],
        )
        .unwrap();
        assert_eq!(before, current.to_thk().unwrap());
        assert_eq!(learning.hunts, 2);
        let before_learning = learning.effects[key].samples;
        let mut bad = t.completed()[0].clone();
        bad.exit_index = usize::MAX;
        assert!(apply_hunt(
            &baseline,
            &mut current,
            &mut learning,
            &Policy::default(),
            &[bad]
        )
        .is_err());
        assert_eq!(learning.hunts, 2);
        assert_eq!(learning.effects[key].samples, before_learning);
        assert_eq!(before, current.to_thk().unwrap());
    }
}
