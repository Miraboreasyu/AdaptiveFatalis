//! Host-only exchange observations. Party slot zero is a source-derived host
//! candidate, not a guaranteed network authority identifier on every build.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostSample {
    pub player_token: u64,
    pub damage_token: u64,
    pub local_is_host: Option<bool>,
    pub slot: Option<usize>,
    pub damage_total: Option<f64>,
    pub position: Option<[f64; 3]>,
    pub distance: Option<f64>,
    pub angle: Option<f64>,
}
#[derive(Clone, Debug)]
pub struct HostWindow {
    opening: HostSample,
    last: HostSample,
    damage: f64,
    reason: Option<&'static str>,
}
impl HostWindow {
    pub fn new(s: HostSample) -> Self {
        let reason = Self::problem(&s);
        Self {
            opening: s.clone(),
            last: s,
            damage: 0.0,
            reason,
        }
    }
    fn problem(s: &HostSample) -> Option<&'static str> {
        if s.local_is_host != Some(true) {
            return Some("local_host_unconfirmed");
        }
        if s.player_token == 0 || s.damage_token == 0 || s.slot != Some(0) {
            return Some("host_identity_unavailable");
        }
        if !s.damage_total.is_some_and(|x| x.is_finite() && x >= 0.0) {
            return Some("host_damage_unavailable");
        }
        if !s.position.is_some_and(|p| p.iter().all(|x| x.is_finite()))
            || !s.distance.is_some_and(|d| d.is_finite() && d >= 0.0)
            || !s.angle.is_some_and(|a| a.is_finite())
        {
            return Some("host_position_unavailable");
        }
        None
    }
    pub fn tick(&mut self, sample: Option<&HostSample>) {
        let Some(s) = sample else {
            self.reason.get_or_insert("host_observation_gap");
            return;
        };
        if let Some(reason) = Self::problem(s) {
            self.reason.get_or_insert(reason);
        }
        if s.player_token != self.last.player_token
            || s.damage_token != self.last.damage_token
            || s.slot != self.last.slot
        {
            self.reason.get_or_insert("host_identity_changed");
        }
        match (self.last.damage_total, s.damage_total) {
            (Some(a), Some(b)) if b >= a => self.damage += b - a,
            _ => {
                self.reason.get_or_insert("host_damage_reset_or_gap");
            }
        }
        self.last = s.clone();
    }
    pub fn result(&self, monster_max: f64) -> serde_json::Value {
        serde_json::json!({"scope":"quest_host","host_slot":self.opening.slot,"local_is_host":self.opening.local_is_host,
            "host_token":self.opening.player_token,"opening_position":self.opening.position,"opening_distance":self.opening.distance,"opening_angle_cw":self.opening.angle,
            "host_damage":if self.reason.is_none(){Some(self.damage)}else{None},
            "host_damage_fraction":if self.reason.is_none(){Some(self.damage/monster_max)}else{None},
            "damage_source":"party_slot_cumulative_statistic","reliable":self.reason.is_none(),"unavailable_reason":self.reason})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn s(d: f64) -> HostSample {
        HostSample {
            player_token: 10,
            damage_token: 20,
            local_is_host: Some(true),
            slot: Some(0),
            damage_total: Some(d),
            position: Some([1.0, 2.0, 3.0]),
            distance: Some(4.0),
            angle: Some(5.0),
        }
    }
    #[test]
    fn personal_delta_and_opening_geometry() {
        let mut w = HostWindow::new(s(100.0));
        let mut n = s(125.0);
        n.position = Some([8.0; 3]);
        w.tick(Some(&n));
        let r = w.result(1000.0);
        assert_eq!(r["host_damage"], 25.0);
        assert_eq!(r["opening_position"], serde_json::json!([1.0, 2.0, 3.0]));
    }
    #[test]
    fn resets_gaps_and_nonhost_fail_closed() {
        for mut n in [s(90.0), s(110.0), s(110.0)] {
            if n.damage_total == Some(110.0) {
                n.player_token = 11;
            }
            let mut w = HostWindow::new(s(100.0));
            w.tick(Some(&n));
            assert_eq!(w.result(1000.0)["reliable"], false);
        }
        let mut w = HostWindow::new(s(100.0));
        w.tick(None);
        w.tick(Some(&s(140.0)));
        assert_eq!(w.result(1000.0)["host_damage"], serde_json::Value::Null);
        let mut n = s(0.0);
        n.local_is_host = Some(false);
        assert_eq!(HostWindow::new(n).result(1000.0)["reliable"], false);
    }
}
