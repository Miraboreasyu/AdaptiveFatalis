//! Policy-independent measurements and bounded between-hunt learning.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Effect {
    pub samples: f64,
    pub player_loss: f64,
    pub monster_loss: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Learning {
    pub schema: u32,
    pub hunts: u64,
    pub effects: BTreeMap<String, Effect>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    /// Positive = malevolent; negative = benevolent; zero = baseline.
    pub orientation: f64,
    pub strength: f64,
    pub prior_samples: f64,
    /// Fraction of the original distribution always retained.
    pub exploration: f64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            orientation: 1.0,
            strength: 2.0,
            prior_samples: 8.0,
            exploration: 0.25,
        }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<(), String> {
        if !self.orientation.is_finite()
            || self.orientation.abs() > 1.0
            || !self.strength.is_finite()
            || !(0.0..=10.0).contains(&self.strength)
            || !self.prior_samples.is_finite()
            || self.prior_samples <= 0.0
            || !self.exploration.is_finite()
            || !(0.05..=1.0).contains(&self.exploration)
        {
            return Err("invalid learning policy".into());
        }
        Ok(())
    }
}
impl Learning {
    pub fn observe(&mut self, key: &str, player_loss: f64, monster_loss: f64, confidence: f64) {
        if !player_loss.is_finite()
            || !monster_loss.is_finite()
            || !confidence.is_finite()
            || player_loss < 0.0
            || monster_loss < 0.0
            || !(0.0..=1.0).contains(&confidence)
            || confidence == 0.0
        {
            return;
        }
        self.schema = 1;
        let effect = self.effects.entry(key.to_owned()).or_default();
        effect.samples += confidence;
        effect.player_loss += player_loss.min(1.0) * confidence;
        effect.monster_loss += monster_loss.min(1.0) * confidence;
    }
    /// Compute a target from the immutable baseline, never compound an already adapted weight.
    /// The caller applies the resulting targets to the currently managed THK.
    pub fn weights(
        &self,
        keys: &[String],
        baseline: &[i32],
        policy: &Policy,
    ) -> Result<Vec<i32>, String> {
        policy.validate()?;
        if keys.len() != baseline.len()
            || baseline.is_empty()
            || baseline.iter().any(|v| *v < 0)
            || baseline.iter().sum::<i32>() != 100
        {
            return Err("invalid baseline distribution".into());
        }
        let mut scores = Vec::with_capacity(keys.len());
        for (key, base) in keys.iter().zip(baseline) {
            let score = self
                .effects
                .get(key)
                .map(|e| (e.player_loss - e.monster_loss) / (e.samples + policy.prior_samples))
                .unwrap_or(0.0);
            let value = (*base as f64)
                * (policy.orientation * policy.strength * score)
                    .clamp(-20.0, 20.0)
                    .exp();
            if !value.is_finite() {
                return Err("non-finite learning state".into());
            }
            scores.push(value);
        }
        let total: f64 = scores.iter().sum();
        if total <= 0.0 || !total.is_finite() {
            return Err("invalid learned distribution".into());
        }
        let values: Vec<f64> = scores
            .iter()
            .zip(baseline)
            .map(|(s, b)| {
                policy.exploration * (*b as f64) + (1.0 - policy.exploration) * 100.0 * s / total
            })
            .collect();
        let mut result: Vec<i32> = values.iter().map(|v| v.floor() as i32).collect();
        let mut order: Vec<usize> = (0..result.len()).filter(|i| baseline[*i] > 0).collect();
        order.sort_by(|a, b| {
            (values[*b] - result[*b] as f64)
                .total_cmp(&(values[*a] - result[*a] as f64))
                .then(a.cmp(b))
        });
        let remainder = 100 - result.iter().sum::<i32>();
        if remainder < 0 || remainder as usize > order.len() {
            return Err("rounding failure".into());
        }
        for i in order.into_iter().take(remainder as usize) {
            result[i] += 1;
        }
        // Preserve every originally possible choice, including tiny baseline probabilities.
        for i in 0..result.len() {
            if baseline[i] > 0 && result[i] == 0 {
                let donor = (0..result.len()).max_by_key(|j| result[*j]).unwrap();
                if result[donor] <= 1 {
                    return Err("cannot preserve exploration".into());
                }
                result[donor] -= 1;
                result[i] = 1;
            }
        }
        Ok(result)
    }
}

/// Accumulate negative HP deltas, not net HP change: healing does not erase earlier hits.
#[derive(Clone, Debug)]
pub struct DamageWindow {
    last_player: f64,
    last_monster: f64,
    pub player_loss: f64,
    pub monster_loss: f64,
}
impl DamageWindow {
    pub fn new(player: f64, monster: f64) -> Self {
        Self {
            last_player: player,
            last_monster: monster,
            player_loss: 0.0,
            monster_loss: 0.0,
        }
    }
    pub fn tick(&mut self, player: f64, monster: f64) {
        if !player.is_finite() || !monster.is_finite() || player < 0.0 || monster < 0.0 {
            return;
        }
        self.player_loss += (self.last_player - player).max(0.0);
        self.monster_loss += (self.last_monster - monster).max(0.0);
        self.last_player = player;
        self.last_monster = monster;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unobserved_is_baseline() {
        let l = Learning::default();
        let k = vec!["a".into(), "b".into(), "sentinel".into()];
        assert_eq!(
            l.weights(&k, &[65, 35, 0], &Policy::default()).unwrap(),
            vec![65, 35, 0]
        );
    }
    #[test]
    fn orientation_reverses_preference() {
        let mut l = Learning::default();
        for _ in 0..100 {
            l.observe("a", 0.8, 0.0, 1.0);
            l.observe("b", 0.0, 0.2, 1.0);
        }
        let k = vec!["a".into(), "b".into()];
        let mut p = Policy::default();
        assert!(l.weights(&k, &[50, 50], &p).unwrap()[0] > 50);
        p.orientation = -1.0;
        assert!(l.weights(&k, &[50, 50], &p).unwrap()[0] < 50);
    }
    #[test]
    fn healing_does_not_cancel_damage() {
        let mut w = DamageWindow::new(100.0, 1000.0);
        w.tick(50.0, 900.0);
        w.tick(90.0, 950.0);
        w.tick(80.0, 900.0);
        assert_eq!(w.player_loss, 60.0);
        assert_eq!(w.monster_loss, 150.0);
    }
    #[test]
    fn repeated_generation_does_not_compound() {
        let mut l = Learning::default();
        l.observe("a", 1.0, 0.0, 1.0);
        let k = vec!["a".into(), "b".into()];
        assert_eq!(
            l.weights(&k, &[50, 50], &Policy::default()).unwrap(),
            l.weights(&k, &[50, 50], &Policy::default()).unwrap()
        );
    }
    #[test]
    fn sub_integer_effect_is_recomputed_and_eventually_crosses() {
        let policy = Policy::default();
        let keys = vec!["a".into(), "b".into()];
        let mut l = Learning::default();
        assert_eq!(l.weights(&keys, &[50, 50], &policy).unwrap(), vec![50, 50]);
        // One sample is not enough to change the integer distribution.
        l.observe("a", 0.1, 0.0, 1.0);
        assert_eq!(l.weights(&keys, &[50, 50], &policy).unwrap(), vec![50, 50]);
        // The second sample uses both observations, so the first sample's
        // sub-integer contribution is not lost or double-counted.
        l.observe("a", 0.1, 0.0, 1.0);
        assert!(l.weights(&keys, &[50, 50], &policy).unwrap()[0] > 50);
    }
}
