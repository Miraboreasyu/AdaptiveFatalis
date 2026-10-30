//! Offline distance/angle projection of independent random tables.
//! This is NOT a runtime THK interpreter: opaque Global calls may preempt/restart
//! execution, register mutations are not simulated, and geometry is frozen at query time.
use crate::thk::{Guard, Project, Segment};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    pub distance: f64,
    pub angle_cw: f64,
    pub entry_node: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Condition {
    pub function: i32,
    pub parameter1: i32,
    pub parameter2: i32,
    pub expected: bool,
    pub geometry: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Route {
    pub node_path: Vec<usize>,
    pub conditions: Vec<Condition>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableOption {
    pub segment: usize,
    pub weight: i32,
    pub weight_offset: usize,
    pub global_node: Option<usize>,
    pub global_id: Option<i32>,
    pub local_node: Option<usize>,
    pub action_id: Option<i32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableMatch {
    pub table_id: String,
    pub node: usize,
    pub start_segment: usize,
    pub options: Vec<TableOption>,
    pub routes: Vec<Route>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub query: Query,
    pub tables: Vec<TableMatch>,
    /// Node11 ordinary distance-dispatch tables; prelude/control tables remain in tables.
    pub ordinary_table_ids: Vec<String>,
    pub warnings: Vec<String>,
    pub semantics: String,
}
pub struct GeometryModel<'a> {
    project: &'a Project,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct State {
    conditions: Vec<Condition>,
}
struct Context<'a, 'b> {
    model: &'a GeometryModel<'b>,
    query: &'a Query,
    tables: BTreeMap<(usize, usize), TableMatch>,
    warnings: BTreeSet<String>,
    steps: usize,
}
impl<'a> GeometryModel<'a> {
    pub fn new(project: &'a Project) -> Self {
        Self { project }
    }
    pub fn query(&self, query: &Query) -> Result<QueryResult, String> {
        if !query.distance.is_finite() || query.distance < 0.0 || !query.angle_cw.is_finite() {
            return Err("invalid distance or angle".into());
        }
        if query.entry_node >= self.project.main.nodes.len() {
            return Err("entry node out of range".into());
        }
        let mut c = Context {
            model: self,
            query,
            tables: BTreeMap::new(),
            warnings: BTreeSet::new(),
            steps: 0,
        };
        c.node(query.entry_node, State { conditions: vec![] }, &mut vec![])?;
        for table in c.tables.values_mut() {
            table.routes.sort();
            table.routes.dedup();
        }
        let ordinary_table_ids = c
            .tables
            .values()
            .filter(|t| (75..=125).contains(&t.node))
            .map(|t| t.table_id.clone())
            .collect();
        Ok(QueryResult{query:query.clone(),ordinary_table_ids,tables:c.tables.into_values().collect(),warnings:c.warnings.into_iter().collect(),semantics:"Frozen-geometry structural path projection. Unknown non-geometric predicates are symbolic, with prior elif negations and contradictory identical atoms excluded. Global calls are opaque. Callee preemption, state mutation and timing are not a full VM; returned tables are conditional structural candidates, not proof of actual runtime execution.".into()})
    }
}
type ConditionalArms = (Vec<(usize, usize, usize)>, usize);

impl Context<'_, '_> {
    fn budget(&mut self) -> Result<(), String> {
        self.steps += 1;
        if self.steps > 250_000 {
            return Err(
                "geometry query exceeded 250000 structural steps; no partial result returned"
                    .into(),
            );
        }
        Ok(())
    }
    fn constrain(&mut self, state: &State, s: &Segment, expected: bool) -> Option<State> {
        let function = s.function.checked_abs()?;
        // function 0/2 is a no-predicate segment, including unconditional else.
        if function == 0 || function == 2 {
            return expected.then(|| state.clone());
        }
        let expected = if s.function < 0 { !expected } else { expected };
        let geometry = matches!(function, 8 | 9 | 0x16);
        if geometry {
            let g = Guard {
                function,
                parameter1: s.p1,
                parameter2: s.p2,
                negated: false,
            };
            if let Some(value) = g.geometry_matches(self.query.distance, self.query.angle_cw) {
                if value != expected {
                    return None;
                }
            } else {
                self.warnings.insert("Exact angle endpoints are not runtime-verified; endpoint predicates remain symbolic and may yield both adjacent tables.".into());
            }
        }
        let condition = Condition {
            function,
            parameter1: s.p1,
            parameter2: s.p2,
            expected,
            geometry,
        };
        if state.conditions.iter().any(|c| {
            c.function == function
                && c.parameter1 == s.p1
                && c.parameter2 == s.p2
                && c.expected != expected
        }) {
            return None;
        }
        let mut result = state.clone();
        if !result.conditions.contains(&condition) {
            result.conditions.push(condition);
        }
        Some(result)
    }
    fn node(&mut self, node: usize, state: State, path: &mut Vec<usize>) -> Result<(), String> {
        self.budget()?;
        if path.contains(&node) {
            self.warnings.insert(format!("Local call cycle at node {node} was cut; this is a structural query, not repeated execution."));
            return Ok(());
        }
        if path.len() >= 64 {
            return Err("local call depth exceeded 64".into());
        }
        let segments = &self
            .model
            .project
            .main
            .nodes
            .get(node)
            .ok_or("local call target out of range")?
            .segments;
        path.push(node);
        self.range(node, 0, segments.len(), vec![state], path)?;
        path.pop();
        Ok(())
    }
    fn if_arms(&self, node: usize, start: usize, end: usize) -> Result<ConditionalArms, String> {
        let seg = &self.model.project.main.nodes[node].segments;
        let mut depth = 1;
        let mut heads = vec![start];
        for (i, s) in seg.iter().enumerate().take(end).skip(start + 1) {
            match s.branch {
                2 => depth += 1,
                8 => {
                    depth -= 1;
                    if depth == 0 {
                        let arms = heads
                            .iter()
                            .enumerate()
                            .map(|(a, h)| (*h, *h + 1, heads.get(a + 1).copied().unwrap_or(i)))
                            .collect();
                        return Ok((arms, i));
                    }
                }
                4 if depth == 1 => heads.push(i),
                _ => {}
            }
        }
        Err(format!("unterminated conditional {node}:{start}"))
    }
    fn random_arms(
        &self,
        node: usize,
        start: usize,
        end: usize,
    ) -> Result<(Vec<(usize, usize)>, usize), String> {
        let seg = &self.model.project.main.nodes[node].segments;
        let mut depth = 1;
        let mut heads = vec![start];
        for (i, s) in seg.iter().enumerate().take(end).skip(start + 1) {
            if s.marker == 0x40 {
                depth += 1;
            }
            if s.branch == 1 {
                depth -= 1;
                if depth == 0 {
                    let arms = heads
                        .iter()
                        .enumerate()
                        .map(|(a, h)| (*h, heads.get(a + 1).copied().unwrap_or(i)))
                        .collect();
                    return Ok((arms, i));
                }
            }
            if depth == 1 && matches!(s.marker, 0x80 | 0xc0) {
                heads.push(i);
            }
        }
        Err(format!("unterminated random {node}:{start}"))
    }
    fn add_table(
        &mut self,
        node: usize,
        start: usize,
        arms: &[(usize, usize)],
        states: &[State],
        path: &[usize],
    ) -> Result<(), String> {
        let mut options = Vec::new();
        for (head, _) in arms {
            let s = &self.model.project.main.nodes[node].segments[*head];
            let global_id = (s.external_thk == 55).then_some(s.external_id);
            let global_node = if let Some(id) = global_id {
                Some(
                    self.model
                        .project
                        .global
                        .nodes
                        .iter()
                        .position(|n| n.id == id as u32)
                        .ok_or_else(|| format!("unresolved Global id {id}"))?,
                )
            } else {
                None
            };
            options.push(TableOption {
                segment: *head,
                weight: s.p1,
                weight_offset: s.offset + 12,
                global_node,
                global_id,
                local_node: (s.local >= 0).then_some(s.local as usize),
                action_id: (s.action > 0).then_some(s.action),
            });
        }
        let table = self
            .tables
            .entry((node, start))
            .or_insert_with(|| TableMatch {
                table_id: format!("main:n{node}:s{start}"),
                node,
                start_segment: start,
                options,
                routes: vec![],
            });
        for s in states {
            table.routes.push(Route {
                node_path: path.to_vec(),
                conditions: s.conditions.clone(),
            });
        }
        Ok(())
    }
    fn range(
        &mut self,
        node: usize,
        start: usize,
        end: usize,
        mut states: Vec<State>,
        path: &mut Vec<usize>,
    ) -> Result<Vec<State>, String> {
        let mut i = start;
        while i < end && !states.is_empty() {
            self.budget()?;
            let s = self.model.project.main.nodes[node].segments[i].clone();
            if s.branch == 2 {
                let (arms, close) = self.if_arms(node, i, end)?;
                let mut remaining = states;
                let mut next = Vec::new();
                let mut has_else = false;
                for (head, body, stop) in arms {
                    let h = self.model.project.main.nodes[node].segments[head].clone();
                    let unconditional = matches!(h.function, 0 | 2);
                    let selected = if unconditional {
                        has_else = true;
                        std::mem::take(&mut remaining)
                    } else {
                        remaining
                            .iter()
                            .filter_map(|st| self.constrain(st, &h, true))
                            .collect()
                    };
                    if !unconditional {
                        remaining = remaining
                            .iter()
                            .filter_map(|st| self.constrain(st, &h, false))
                            .collect();
                    }
                    let selected = self.plain(node, head, selected, path)?;
                    next.extend(self.range(node, body, stop, selected, path)?);
                }
                if !has_else {
                    next.extend(remaining);
                }
                states = dedup(next);
                i = close;
                // endif-with-operation executes after the join, not in either arm.
            } else if s.marker == 0x40 {
                let (arms, close) = self.random_arms(node, i, end)?;
                self.add_table(node, i, &arms, &states, path)?;
                let mut next = Vec::new();
                for (head, stop) in arms {
                    if self.model.project.main.nodes[node].segments[head].p1 > 0 {
                        next.extend(self.plain_then_range(
                            node,
                            head,
                            stop,
                            states.clone(),
                            path,
                        )?);
                    }
                }
                states = dedup(next);
                i = close;
            }
            if states.is_empty() {
                break;
            }
            states = self.plain(node, i, states, path)?;
            i += 1;
        }
        Ok(states)
    }
    fn plain_then_range(
        &mut self,
        node: usize,
        head: usize,
        end: usize,
        states: Vec<State>,
        path: &mut Vec<usize>,
    ) -> Result<Vec<State>, String> {
        let states = self.plain(node, head, states, path)?;
        self.range(node, head + 1, end, states, path)
    }
    fn plain(
        &mut self,
        node: usize,
        index: usize,
        states: Vec<State>,
        path: &mut Vec<usize>,
    ) -> Result<Vec<State>, String> {
        let s = self.model.project.main.nodes[node].segments[index].clone();
        // An inline geometric predicate gates the remainder of its segment only.
        // A false result advances to the next segment, rather than ending the node.
        let (states, skipped) = if s.branch == 0 && matches!(s.function.abs(), 8 | 9 | 0x16) {
            let selected = states
                .iter()
                .filter_map(|st| self.constrain(st, &s, true))
                .collect::<Vec<_>>();
            let skipped = states
                .iter()
                .filter_map(|st| self.constrain(st, &s, false))
                .collect::<Vec<_>>();
            (selected, skipped)
        } else {
            (states, vec![])
        };
        if s.marker == 1 || s.branch == 0x10 {
            return Ok(vec![]);
        }
        if s.external_thk >= 0 {
            self.warnings.insert("External/Global calls are opaque: their actions, geometry changes, resets and preemption are not executed; caller continuation is a conditional structural possibility.".into());
        }
        if s.local >= 0 {
            let target = s.local as usize;
            for state in &states {
                self.node(target, state.clone(), path)?;
            }
            self.warnings.insert("Local callee paths are inspected, but callee return/reset effects and state mutations are not propagated into caller continuation.".into());
        }
        if s.function != 0 && s.function != 2 && s.branch == 0 {
            self.warnings.insert("Inline predicates and register side effects are not executed; conditional block predicates alone are used for route constraints.".into());
        }
        if matches!(s.flow, 4 | 8 | 0x80) {
            return Ok(skipped);
        }
        let mut next = states;
        next.extend(skipped);
        Ok(dedup(next))
    }
}
fn dedup(states: Vec<State>) -> Vec<State> {
    states
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(all(test, feature = "game-fixtures"))]
mod tests {
    use super::*;
    fn original() -> Option<Project> {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/fatalis_thk");
        if !root.join("em013_00.thk").exists() {
            return None;
        }
        Some(Project::from_paths(root.join("em013_00.thk"), root.join("em013_55.thk")).unwrap())
    }
    #[test]
    fn node105_selects_one_independent_table() {
        let Some(p) = original() else { return };
        let m = GeometryModel::new(&p);
        for (angle, start) in [
            (0., 1),
            (50., 11),
            (300., 21),
            (180., 31),
            (120., 41),
            (240., 41),
        ] {
            let r = m
                .query(&Query {
                    distance: 700.,
                    angle_cw: angle,
                    entry_node: 105,
                })
                .unwrap();
            assert_eq!(r.tables.len(), 1, "{angle}");
            assert_eq!(r.tables[0].start_segment, start);
            assert_eq!(
                r.tables[0].options.iter().map(|x| x.weight).sum::<i32>(),
                100
            );
            assert_eq!(r.tables[0].options[0].global_node, Some(102));
        }
    }
    #[test]
    fn else_preserves_all_prior_angle_negations() {
        let Some(p) = original() else { return };
        let r = GeometryModel::new(&p)
            .query(&Query {
                distance: 700.,
                angle_cw: 120.,
                entry_node: 105,
            })
            .unwrap();
        let route = &r.tables[0].routes[0];
        assert_eq!(
            route
                .conditions
                .iter()
                .filter(|c| c.geometry && !c.expected)
                .count(),
            4
        );
        assert_eq!(r.tables[0].options.len(), 9);
    }
    #[test]
    fn main11_inherits_distance_to_node105() {
        let Some(p) = original() else { return };
        let m = GeometryModel::new(&p);
        let r = m
            .query(&Query {
                distance: 700.,
                angle_cw: 0.,
                entry_node: 11,
            })
            .unwrap();
        let t = r.tables.iter().find(|t| t.node == 105).unwrap();
        assert_eq!(t.start_segment, 1);
        assert!(t.routes.iter().any(|r| r.node_path == vec![11, 105]
            && r.conditions
                .iter()
                .any(|c| c.function == 8 && c.parameter1 == 500 && !c.expected)
            && r.conditions
                .iter()
                .any(|c| c.function == 8 && c.parameter1 == 1000 && c.expected)));
        let r = m
            .query(&Query {
                distance: 1200.,
                angle_cw: 0.,
                entry_node: 11,
            })
            .unwrap();
        assert!(!r.tables.iter().any(|t| t.node == 105));
    }
    #[test]
    fn conditional_head_call_is_not_skipped() {
        let Some(mut p) = original() else { return };
        p.main.nodes[105].segments[0].local = 106;
        let r = GeometryModel::new(&p)
            .query(&Query {
                distance: 700.,
                angle_cw: 0.,
                entry_node: 105,
            })
            .unwrap();
        assert!(r
            .tables
            .iter()
            .any(|t| t.node == 106 && t.start_segment == 1));
    }
    #[test]
    fn inline_geometry_only_gates_its_segment() {
        let Some(mut p) = original() else { return };
        let mut condition = p.main.nodes[105].segments[0].clone();
        condition.branch = 0;
        condition.local = 105;
        let terminal = p.main.nodes[105].segments[52].clone();
        p.main.nodes[0].segments = vec![condition, terminal];
        let m = GeometryModel::new(&p);
        let yes = m
            .query(&Query {
                distance: 700.,
                angle_cw: 0.,
                entry_node: 0,
            })
            .unwrap();
        assert!(yes.tables.iter().any(|t| t.node == 105));
        let no = m
            .query(&Query {
                distance: 700.,
                angle_cw: 120.,
                entry_node: 0,
            })
            .unwrap();
        assert!(no.tables.is_empty());
    }
    #[test]
    fn invalid_query_rejected() {
        let Some(p) = original() else { return };
        assert!(GeometryModel::new(&p)
            .query(&Query {
                distance: -1.,
                angle_cw: 0.,
                entry_node: 105
            })
            .is_err());
    }
}
