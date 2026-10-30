//! Pure action-window detector. No display-log debounce or same-name grouping.
use crate::learning::DamageWindow;

#[derive(Clone, Copy, Debug)]
pub struct ActionSample {
    pub token: u64,
    pub action: i32,
    pub state: u64,
    pub frame: f64,
    pub distance: f64,
    pub angle: f64,
    pub player: f64,
    pub player_max: f64,
    pub monster: f64,
    pub monster_max: f64,
}
#[derive(Debug)]
pub struct CompletedWindow {
    pub start: ActionSample,
    pub player_loss_fraction: f64,
    pub monster_loss_fraction: f64,
}
#[derive(Default)]
pub struct Tracker {
    previous: Option<ActionSample>,
    start: Option<ActionSample>,
    damage: Option<DamageWindow>,
}
impl Tracker {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn tick(&mut self, sample: ActionSample) -> (bool, Option<CompletedWindow>) {
        let previous = self.previous;
        if previous.map(|p| p.token != sample.token).unwrap_or(false) {
            self.reset();
        }
        if let Some(d) = self.damage.as_mut() {
            d.tick(sample.player, sample.monster);
        }
        let changed = self
            .previous
            .map(|p| {
                p.action != sample.action
                    || p.state != sample.state
                    || (p.state == sample.state && p.frame > 1.0 && sample.frame < 0.5)
            })
            .unwrap_or(true);
        let completed = if changed { self.finish() } else { None };
        if changed {
            self.start = Some(sample);
            self.damage = Some(DamageWindow::new(sample.player, sample.monster));
        }
        self.previous = Some(sample);
        (changed, completed)
    }
    pub fn finish(&mut self) -> Option<CompletedWindow> {
        let start = self.start.take()?;
        let damage = self.damage.take()?;
        if start.player_max <= 0.0 || start.monster_max <= 0.0 {
            return None;
        }
        Some(CompletedWindow {
            start,
            player_loss_fraction: damage.player_loss / start.player_max,
            monster_loss_fraction: damage.monster_loss / start.monster_max,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> ActionSample {
        ActionSample {
            token: 1,
            action: 88,
            state: 4,
            frame: 4.0,
            distance: 700.0,
            angle: 0.0,
            player: 100.0,
            player_max: 100.0,
            monster: 1000.0,
            monster_max: 1000.0,
        }
    }
    #[test]
    fn same_action_reentry_is_not_merged() {
        let mut t = Tracker::default();
        assert!(t.tick(sample()).0);
        let mut s = sample();
        s.frame = 0.0;
        s.player = 80.0;
        s.monster = 900.0;
        let (new, window) = t.tick(s);
        assert!(new);
        let w = window.unwrap();
        assert_eq!(w.player_loss_fraction, 0.2);
        assert_eq!(w.monster_loss_fraction, 0.1);
    }
    #[test]
    fn geometry_drift_does_not_fragment_action() {
        let mut t = Tracker::default();
        t.tick(sample());
        let mut s = sample();
        s.distance += 1.0;
        s.angle += 0.1;
        s.frame = 5.0;
        assert!(!t.tick(s).0);
    }
    #[test]
    fn state_switch_starts_new_action() {
        let mut t = Tracker::default();
        t.tick(sample());
        let mut s = sample();
        s.state = 5;
        assert!(t.tick(s).0);
    }
    #[test]
    fn new_instance_discards_old_window() {
        let mut t = Tracker::default();
        t.tick(sample());
        let mut s = sample();
        s.token = 2;
        s.player = 20.0;
        assert!(t.tick(s).1.is_none());
    }
}
