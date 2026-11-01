//! Source-derived counters, not proof of hit source, local faint, or interruption.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct AuxiliarySnapshot {
    pub quest_token: u64,
    pub deaths: Option<i32>,
    pub max_deaths: Option<i32>,
    pub monster_token: Option<u64>,
    pub part_table_token: Option<u64>,
    pub part_counts: Option<[i32; 11]>,
}
#[derive(Default)]
pub struct CounterObserver {
    last: Option<AuxiliarySnapshot>,
}
#[derive(Default, Debug, PartialEq)]
pub struct CounterEvents {
    pub parts: Vec<(usize, u32)>,
    pub deaths: u32,
}
impl CounterObserver {
    pub fn push(&mut self, s: AuxiliarySnapshot) -> CounterEvents {
        let mut result = CounterEvents::default();
        if let Some(p) = &self.last {
            if p.quest_token == s.quest_token {
                if p.max_deaths.is_some() && p.max_deaths == s.max_deaths {
                    if let (Some(a), Some(b)) = (p.deaths, s.deaths) {
                        if b > a {
                            result.deaths = (b - a) as u32;
                        }
                    }
                }
                if p.monster_token.is_some()
                    && p.monster_token == s.monster_token
                    && p.part_table_token.is_some()
                    && p.part_table_token == s.part_table_token
                {
                    if let (Some(a), Some(b)) = (p.part_counts, s.part_counts) {
                        for i in 0..11 {
                            if b[i] > a[i] {
                                result.parts.push((i, (b[i] - a[i]) as u32));
                            }
                        }
                    }
                }
            }
        }
        self.last = Some(s);
        result
    }
    pub fn reset(&mut self) {
        self.last = None;
    }
}
/// Independent local HP edge detector. Missing reads/max-HP or session changes
/// break continuity; initial zero and repeated zero are not transitions.
#[derive(Default)]
pub struct HpObserver {
    last: Option<(u64, f64, f64)>,
}
impl HpObserver {
    pub fn push(&mut self, token: u64, hp: Option<(f64, f64)>) -> bool {
        let next = hp
            .filter(|(max, current)| {
                max.is_finite()
                    && current.is_finite()
                    && *max > 0.0
                    && *current >= 0.0
                    && *current <= *max
            })
            .map(|(m, h)| (token, m, h));
        let zero = matches!((self.last,next),(Some((a,m,p)),Some((b,n,h))) if a==b && m==n && p>0.0 && h==0.0);
        self.last = next;
        zero
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn s() -> AuxiliarySnapshot {
        AuxiliarySnapshot {
            quest_token: 1,
            deaths: Some(0),
            max_deaths: Some(3),
            monster_token: Some(10),
            part_table_token: Some(20),
            part_counts: Some([0; 11]),
        }
    }
    #[test]
    fn first_sample_resets_gaps_and_tokens_are_not_events() {
        let mut o = CounterObserver::default();
        assert_eq!(o.push(s()), CounterEvents::default());
        let mut n = s();
        n.quest_token = 2;
        n.deaths = Some(1);
        assert_eq!(o.push(n), CounterEvents::default());
        o.push(AuxiliarySnapshot {
            deaths: None,
            part_counts: None,
            ..s()
        });
        assert_eq!(o.push(s()), CounterEvents::default());
    }
    #[test]
    fn hp_zero_requires_continuity_and_does_not_repeat() {
        let mut o = HpObserver::default();
        assert!(!o.push(1, Some((200.0, 0.0))));
        assert!(!o.push(1, Some((200.0, 100.0))));
        assert!(o.push(1, Some((200.0, 0.0))));
        assert!(!o.push(1, Some((200.0, 0.0))));
        o.push(1, Some((200.0, 100.0)));
        o.push(1, None);
        assert!(!o.push(1, Some((200.0, 0.0))));
        o.push(1, Some((200.0, 100.0)));
        assert!(!o.push(1, Some((150.0, 0.0))));
        o.push(1, Some((150.0, 100.0)));
        assert!(!o.push(2, Some((150.0, 0.0))));
    }
    #[test]
    fn positive_deltas_only() {
        let mut o = CounterObserver::default();
        o.push(s());
        let mut n = s();
        n.deaths = Some(1);
        n.part_counts.as_mut().unwrap()[0] = 2;
        assert_eq!(
            o.push(n),
            CounterEvents {
                parts: vec![(0, 2)],
                deaths: 1
            }
        );
        assert_eq!(o.push(s()), CounterEvents::default());
    }
    #[test]
    fn replaced_part_table_and_changed_max_deaths_reset() {
        let mut o = CounterObserver::default();
        o.push(s());
        let mut n = s();
        n.part_table_token = Some(21);
        n.max_deaths = Some(4);
        n.deaths = Some(1);
        n.part_counts = Some([5; 11]);
        assert_eq!(o.push(n), CounterEvents::default());
        o.reset();
        assert_eq!(o.push(s()), CounterEvents::default());
    }
}
