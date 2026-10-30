//! Batch-local multiplayer exclusion; never rolls back already committed events.
#[derive(Default)]
pub struct BatchGuard {
    reason: Option<&'static str>,
    last_update_ms: Option<u64>,
}
impl BatchGuard {
    pub fn observe(&mut self, party: Option<bool>, now_ms: u64) -> bool {
        self.last_update_ms = Some(now_ms);
        let before = self.reason;
        match party {
            Some(true) => self.reason = Some("multiplayer_detected"),
            None if self.reason.is_none() => self.reason = Some("party_state_unknown"),
            _ => {}
        }
        before != self.reason
    }
    pub fn reason(&self) -> Option<&'static str> {
        self.reason
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Only reset while not receiving valid combat updates. Caller must settle
    /// the old batch BEFORE resetting and recheck party on the next valid sample.
    pub fn idle(&self, now_ms: u64) -> bool {
        self.last_update_ms
            .is_some_and(|last| now_ms.saturating_sub(last) >= 60_000)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiplayer_latches_until_reset() {
        let mut g = BatchGuard::default();
        g.observe(Some(false), 0);
        assert_eq!(g.reason(), None);
        g.observe(Some(true), 1);
        g.observe(Some(false), 2);
        assert_eq!(g.reason(), Some("multiplayer_detected"));
        assert!(!g.idle(60_001));
        assert!(g.idle(60_002));
        g.reset();
        assert_eq!(g.reason(), None);
    }
    #[test]
    fn unknown_is_not_solo_and_multiplayer_overrides_reason() {
        let mut g = BatchGuard::default();
        g.observe(None, 0);
        assert_eq!(g.reason(), Some("party_state_unknown"));
        g.observe(Some(false), 5);
        assert!(g.reason().is_some());
        g.observe(Some(true), 10);
        assert_eq!(g.reason(), Some("multiplayer_detected"));
    }
}
