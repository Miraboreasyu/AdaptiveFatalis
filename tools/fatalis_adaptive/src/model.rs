//! Conservative inverse mapping, not a THK interpreter.
//! Candidate confidence is a sharing weight, NOT probability of runtime execution.
#[cfg(all(test, feature = "game-fixtures"))]
use crate::thk::Guard;
use crate::thk::{Branch, Project};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub distance: f64,
    pub angle_cw: f64,
    pub action_id: i32,
    pub phase: Option<u8>,
    pub biped: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub block_id: String,
    pub branch_id: String,
    pub exit_index: usize,
    pub confidence: f64,
    /// false: unverified upstream route and/or unknown internal state.
    pub guards_complete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribution {
    pub candidates: Vec<Candidate>,
    pub exact: bool,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coverage {
    pub all_random_blocks: usize,
    pub candidate_blocks: usize,
    pub verified_upstream_blocks: usize,
    pub supported_nodes: Vec<usize>,
    pub limitation: String,
}
#[derive(Debug, Clone)]
struct Exit {
    block_id: String,
    branch: Branch,
    index: usize,
    node: usize,
    route: Option<(f64, f64)>,
}
#[derive(Debug, Clone)]
pub struct Model {
    exits: Vec<Exit>,
    pub coverage: Coverage,
}
fn ordinary_p1_route(node: usize) -> Option<(f64, f64)> {
    match node {
        75 => Some((0.0, 500.0)),
        76 => Some((500.0, 900.0)),
        77 => Some((900.0, 2000.0)),
        78 => Some((2000.0, 4500.0)),
        79 => Some((4500.0, f64::INFINITY)),
        _ => None,
    }
}
impl Model {
    pub fn from_project(project: &Project) -> Self {
        let mut exits = Vec::new();
        let mut nodes = std::collections::BTreeSet::new();
        let mut blocks = 0;
        let mut verified = 0;
        for b in project.random_blocks() {
            // Require a literal Global call or direct action at the branch entry.
            // No-op/target choice, posture/counter control, and local dispatcher
            // random blocks are inventoried but cannot automatically learn.
            let route = ordinary_p1_route(b.node);
            let mut accepted = false;
            for (index, branch) in b.branches.iter().enumerate() {
                let s = &project.main.nodes[b.node].segments[branch.segment];
                if branch.weight <= 0
                    || branch.action_ids.is_empty()
                    || (s.external_thk != 55 && s.action <= 0)
                {
                    continue;
                }
                if b.node < 18 {
                    continue;
                } // preserve target/phase/posture/count mechanics
                  // Exclude branches containing nested randoms: their union cannot
                  // identify which outer exit caused the observed action.
                if project.main.nodes[b.node].segments[branch.segment + 1..branch.end_segment]
                    .iter()
                    .any(|s| s.marker == 0x40)
                {
                    continue;
                }
                exits.push(Exit {
                    block_id: b.id.clone(),
                    branch: branch.clone(),
                    index,
                    node: b.node,
                    route,
                });
                nodes.insert(b.node);
                accepted = true;
            }
            if accepted {
                blocks += 1;
                if route.is_some() {
                    verified += 1;
                }
            }
        }
        Self {exits,coverage:Coverage{all_random_blocks:project.blocks.len(),candidate_blocks:blocks,verified_upstream_blocks:verified,supported_nodes:nodes.into_iter().collect(),limitation:"Only nodes75..79 have verified main11 distance/P1/biped upstream route. Other direct action/Global-call exits use local geometry only, are approximate candidates, and MUST NOT be described as uniquely identified. Recursive action sets include preparatory and later actions. Exact angle endpoints are unknown. High-priority control/empty exits are not learned.".into()}}
    }
    pub fn attribute(&self, obs: &Observation) -> Attribution {
        if !obs.distance.is_finite()
            || obs.distance < 0.0
            || !obs.angle_cw.is_finite()
            || obs.action_id <= 0
            || obs.phase.is_some_and(|p| !(1..=3).contains(&p))
        {
            return Attribution {
                candidates: vec![],
                exact: false,
                reason: "invalid observation".into(),
            };
        }
        let mut candidates = Vec::new();
        for e in &self.exits {
            if !e.branch.action_ids.contains(&obs.action_id) {
                continue;
            }
            let mut complete = false;
            if let Some((lo, hi)) = e.route {
                if (e.node != 75 && obs.distance <= lo)
                    || obs.distance > hi
                    || obs.phase.is_some_and(|p| p != 1)
                    || obs.biped == Some(false)
                {
                    continue;
                }
                complete = obs.phase == Some(1) && obs.biped == Some(true);
            }
            let mut mismatch = false;
            for g in &e.branch.guards {
                match g.geometry_matches(obs.distance, obs.angle_cw) {
                    Some(false) => {
                        mismatch = true;
                        break;
                    }
                    Some(true) => {}
                    None => complete = false,
                }
            }
            if mismatch {
                continue;
            }
            if e.branch.unknown_guard {
                complete = false;
            }
            // Recursive Global candidate extraction intentionally unions guards;
            // therefore this model cannot certify complete runtime eligibility.
            let _known_local_and_route = complete;
            complete = false;
            // Even with known upstream, recursive action unions do not prove an
            // exact first-action path. Never claim certainty from this model.
            candidates.push(Candidate {
                block_id: e.block_id.clone(),
                branch_id: e.branch.id.clone(),
                exit_index: e.index,
                confidence: 0.0,
                guards_complete: complete,
            });
        }
        let count = candidates.len();
        for c in &mut candidates {
            c.confidence = 1.0 / count as f64;
        }
        Attribution {
            candidates,
            exact: false,
            reason: if count == 0 {
                "no supported action/geometry candidate".into()
            } else {
                format!("{count} conservative candidates; normalized equal sharing, not calibrated probabilities; unknown upstream/internal guards retained")
            },
        }
    }
}

#[cfg(all(test, feature = "game-fixtures"))]
mod tests {
    use super::*;
    fn test_model() -> Model {
        let branch = Branch {
            id: "main:n75:s1".into(),
            segment: 1,
            end_segment: 2,
            weight: 50,
            action_ids: vec![88],
            guards: vec![Guard {
                function: 0x16,
                parameter1: 330,
                parameter2: 30,
                negated: false,
            }],
            unknown_guard: false,
        };
        Model {
            exits: vec![Exit {
                block_id: "main:n75:s1".into(),
                branch,
                index: 0,
                node: 75,
                route: ordinary_p1_route(75),
            }],
            coverage: Coverage {
                all_random_blocks: 603,
                candidate_blocks: 1,
                verified_upstream_blocks: 1,
                supported_nodes: vec![75],
                limitation: String::new(),
            },
        }
    }
    #[test]
    fn original_model_reports_partial_coverage() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/fatalis_thk");
        if !root.join("em013_00.thk").exists() {
            return;
        }
        let p = Project::from_paths(root.join("em013_00.thk"), root.join("em013_55.thk")).unwrap();
        let m = Model::from_project(&p);
        assert_eq!(m.coverage.all_random_blocks, 603);
        assert!(m.coverage.candidate_blocks > 400);
        assert_eq!(m.coverage.verified_upstream_blocks, 19);
        let a = m.attribute(&Observation {
            distance: 400.0,
            angle_cw: 0.0,
            action_id: 88,
            phase: None,
            biped: None,
        });
        assert!(!a.candidates.is_empty());
        assert!(!a.exact);
        assert!(a.candidates.iter().all(|c| !c.guards_complete));
    }
    #[test]
    fn unknown_phase_is_not_assumed_p1() {
        let a = test_model().attribute(&Observation {
            distance: 100.0,
            angle_cw: 0.0,
            action_id: 88,
            phase: None,
            biped: None,
        });
        assert_eq!(a.candidates.len(), 1);
        assert!(!a.candidates[0].guards_complete);
        assert!(!a.exact);
    }
    #[test]
    fn known_wrong_phase_excludes_p1() {
        assert!(test_model()
            .attribute(&Observation {
                distance: 100.0,
                angle_cw: 0.0,
                action_id: 88,
                phase: Some(2),
                biped: Some(true)
            })
            .candidates
            .is_empty());
    }
    #[test]
    fn distance_route_is_not_only_local_angle() {
        assert!(test_model()
            .attribute(&Observation {
                distance: 1001.0,
                angle_cw: 0.0,
                action_id: 88,
                phase: Some(1),
                biped: Some(true)
            })
            .candidates
            .is_empty());
    }
    #[test]
    fn weights_sum_to_one() {
        let mut m = test_model();
        m.exits.push(m.exits[0].clone());
        let a = m.attribute(&Observation {
            distance: 100.0,
            angle_cw: 0.0,
            action_id: 88,
            phase: None,
            biped: None,
        });
        assert!((a.candidates.iter().map(|c| c.confidence).sum::<f64>() - 1.0).abs() < 1e-12);
    }
}
