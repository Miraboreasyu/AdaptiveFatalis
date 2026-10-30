//! Sandboxed, compile-once external learning policies. No game/runtime dependencies.
//!
//! Scripts export `update(state, events, params)` and `weights(state, table, params)`.
//! Arguments are owned Rhai/serde copies: scripts cannot mutate caller JSON or retained params.
//! Only the host reads the source file; the engine exposes no filesystem/process/network API.
use rhai::{Dynamic, Engine, Scope, AST};
use serde_json::Value;
use std::path::Path;

const MAX_OPERATIONS: u64 = 2_000_000;
const MAX_DEPTH: usize = 32;
const MAX_STRING: usize = 65_536;
const MAX_ARRAY: usize = 16_384;
const MAX_MAP: usize = 262_144;
const MAX_NODES: usize = 262_144;
const MAX_SOURCE: usize = 262_144;

/// Embedded reference policy, also shipped as an editable external script.
pub fn default_script() -> &'static str {
    include_str!("../policies/default.rhai")
}

pub struct ScriptPolicy {
    engine: Engine,
    ast: AST,
    params: Dynamic,
}

impl ScriptPolicy {
    /// Read and compile exactly once. Script execution only happens in update/weights.
    pub fn load(path: &Path, params: Value) -> Result<Self, String> {
        use std::io::Read;
        let file = std::fs::File::open(path)
            .map_err(|e| format!("cannot read policy {}: {e}", path.display()))?;
        let mut source = String::new();
        file.take((MAX_SOURCE + 1) as u64)
            .read_to_string(&mut source)
            .map_err(|e| format!("cannot read policy {}: {e}", path.display()))?;
        Self::compile(&source, params)
    }

    pub fn no_learning() -> Result<Self, String> {
        Self::compile(
            include_str!("../policies/no_learning.rhai"),
            serde_json::json!({}),
        )
    }

    fn compile(source: &str, params: Value) -> Result<Self, String> {
        if source.len() > MAX_SOURCE {
            return Err("policy source exceeds size limit".into());
        }
        if !params.is_object() {
            return Err("policy params must be an object".into());
        }
        let params = to_dynamic(&params)?;
        let mut engine = Engine::new();
        engine.set_max_operations(MAX_OPERATIONS);
        engine.set_max_expr_depths(MAX_DEPTH, MAX_DEPTH);
        engine.set_max_call_levels(MAX_DEPTH);
        engine.set_max_string_size(MAX_STRING);
        engine.set_max_array_size(MAX_ARRAY);
        engine.set_max_map_size(MAX_MAP);
        // No import resolver, eval, console output or host capabilities.
        engine.set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());
        engine.disable_symbol("import");
        engine.disable_symbol("eval");
        engine.on_print(|_| {});
        engine.on_debug(|_, _, _| {});
        let ast = engine
            .compile(source)
            .map_err(|e| format!("policy compile: {e}"))?;
        Ok(Self {
            engine,
            ast,
            params,
        })
    }

    /// Return a new JSON object; neither input is modified, including on failure.
    pub fn update(&self, state: &Value, events: &Value) -> Result<Value, String> {
        if !state.is_object() || !events.is_array() {
            return Err("policy update expects object state and array events".into());
        }
        let output: Dynamic = self
            .engine
            .call_fn(
                &mut Scope::new(),
                &self.ast,
                "update",
                (to_dynamic(state)?, to_dynamic(events)?, self.params.clone()),
            )
            .map_err(|e| format!("policy update: {e}"))?;
        check_dynamic(&output, 0, &mut 0)?;
        let value: Value =
            rhai::serde::from_dynamic(&output).map_err(|e| format!("policy update output: {e}"))?;
        if !value.is_object() {
            return Err("policy update must return an object".into());
        }
        check_json(&value, 0, &mut 0)?;
        Ok(value)
    }

    /// Validate/convert a JSON state once before evaluating many tables.
    /// Keep this owned (not `into_shared`): shared mutable Rhai maps would violate isolation.
    pub fn prepare_state(&self, state: &Value) -> Result<Dynamic, String> {
        if !state.is_object() {
            return Err("policy state must be an object".into());
        }
        to_dynamic(state)
    }

    /// Floating targets in table branch order. The host alone integerizes them.
    pub fn weights(&self, state: &Value, table: &Value) -> Result<Vec<f64>, String> {
        self.weights_prepared(&self.prepare_state(state)?, table)
    }

    /// Evaluate using an owned clone of a previously prepared state.
    /// A script's mutations cannot escape into this value or another table call.
    pub fn weights_prepared(&self, state: &Dynamic, table: &Value) -> Result<Vec<f64>, String> {
        if !state.is::<rhai::Map>() || state.is_shared() || !table.is_object() {
            return Err("policy weights expects owned map state and object table".into());
        }
        let branches = table
            .get("branches")
            .and_then(Value::as_array)
            .ok_or("policy table must contain branches array")?;
        if branches.is_empty() {
            return Err("policy table branches must not be empty".into());
        }
        let output: Dynamic = self
            .engine
            .call_fn(
                &mut Scope::new(),
                &self.ast,
                "weights",
                (state.clone(), to_dynamic(table)?, self.params.clone()),
            )
            .map_err(|e| format!("policy weights: {e}"))?;
        let values = output
            .try_cast::<rhai::Array>()
            .ok_or("policy weights must return an array of floats")?;
        if values.len() != branches.len() {
            return Err("policy weights length must match branches".into());
        }
        let result: Vec<f64> = values
            .into_iter()
            .map(|value| {
                let value = value
                    .try_cast::<rhai::FLOAT>()
                    .ok_or("policy weights must contain only floats")?;
                if !value.is_finite() || value < 0.0 {
                    return Err("policy weights must be finite and nonnegative");
                }
                Ok(value)
            })
            .collect::<Result<_, _>>()
            .map_err(str::to_owned)?;
        let total: f64 = result.iter().sum();
        if !total.is_finite() || total <= 0.0 {
            return Err("policy weights total must be finite and positive".into());
        }
        Ok(result)
    }
}

fn to_dynamic(value: &Value) -> Result<Dynamic, String> {
    check_json(value, 0, &mut 0)?;
    rhai::serde::to_dynamic(value).map_err(|e| format!("policy input: {e}"))
}

fn check_json(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), String> {
    *nodes += 1;
    if depth > MAX_DEPTH || *nodes > MAX_NODES {
        return Err("policy JSON depth/node limit exceeded".into());
    }
    match value {
        Value::String(s) if s.len() > MAX_STRING => {
            return Err("policy string limit exceeded".into())
        }
        Value::Array(a) => {
            if a.len() > MAX_ARRAY {
                return Err("policy array limit exceeded".into());
            }
            for v in a {
                check_json(v, depth + 1, nodes)?;
            }
        }
        Value::Object(m) => {
            if m.len() > MAX_MAP {
                return Err("policy map limit exceeded".into());
            }
            for (k, v) in m {
                if k.len() > MAX_STRING {
                    return Err("policy string limit exceeded".into());
                }
                check_json(v, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// Reject non-JSON and non-finite values before serde can turn them into JSON null.
fn check_dynamic(value: &Dynamic, depth: usize, nodes: &mut usize) -> Result<(), String> {
    *nodes += 1;
    if depth > MAX_DEPTH || *nodes > MAX_NODES {
        return Err("policy output depth/node limit exceeded".into());
    }
    if value.is::<rhai::FLOAT>() {
        if !value.clone_cast::<rhai::FLOAT>().is_finite() {
            return Err("policy output contains non-finite float".into());
        }
    } else if value.is::<rhai::Array>() {
        for v in value.clone_cast::<rhai::Array>() {
            check_dynamic(&v, depth + 1, nodes)?;
        }
    } else if value.is::<rhai::Map>() {
        for v in value.clone_cast::<rhai::Map>().values() {
            check_dynamic(v, depth + 1, nodes)?;
        }
    } else if !(value.is_unit()
        || value.is::<bool>()
        || value.is::<rhai::INT>()
        || value.is::<rhai::ImmutableString>())
    {
        return Err("policy output contains a non-JSON value".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn policy(direction: i32) -> ScriptPolicy {
        ScriptPolicy::compile(default_script(), json!({"direction": direction})).unwrap()
    }
    fn table() -> Value {
        json!({"id":"t", "branches":[{"id":"a", "base_weight":65},
            {"id":"b", "base_weight":35}, {"id":"zero", "base_weight":0}]})
    }

    #[test]
    fn default_directions_and_exact_float_target() {
        let state = json!({"effects":{"a":{"samples":10.0,"player_loss":8.0,"monster_loss":1.0},
            "b":{"samples":4.0,"player_loss":0.0,"monster_loss":2.0}}});
        for direction in [-1, 0, 1] {
            let actual = policy(direction).weights(&state, &table()).unwrap();
            let a = 65.0 * (direction as f64 * 2.0 * (8.0 - 1.0) / (10.0 + 8.0)).exp();
            let b = 35.0 * (direction as f64 * 2.0 * (0.0 - 2.0) / (4.0 + 8.0)).exp();
            assert_eq!(
                actual,
                vec![
                    0.25 * 65.0 + 0.75 * 100.0 * a / (a + b),
                    0.25 * 35.0 + 0.75 * 100.0 * b / (a + b),
                    0.0
                ]
            );
            if direction == 1 {
                assert!(actual[0] > 65.0);
            }
            if direction == -1 {
                assert!(actual[0] < 65.0);
            }
        }
        assert_eq!(
            policy(1).weights(&json!({}), &table()).unwrap(),
            vec![65.0, 35.0, 0.0]
        );
    }

    #[test]
    fn update_caps_each_event_and_never_mutates_inputs() {
        let p = policy(1);
        let state = json!({"tag":"preserved"});
        let events = json!([{"branch_id":"a","player_loss_fraction":2.5,"monster_loss_fraction":0.2},
            {"branch_id":"a","player_loss_fraction":0.5,"monster_loss_fraction":3.0}]);
        let original = (state.clone(), events.clone());
        let next = p.update(&state, &events).unwrap();
        assert_eq!(
            next["effects"]["a"],
            json!({"samples":2.0,"player_loss":1.5,"monster_loss":1.2})
        );
        assert_eq!(next["tag"], "preserved");
        assert_eq!((state.clone(), events.clone()), original);
        assert_eq!(p.update(&state, &events).unwrap(), next);
        assert_eq!(
            p.weights(&next, &table()).unwrap(),
            p.weights(&next, &table()).unwrap()
        );
    }

    #[test]
    fn adversarial_script_cannot_mutate_state_or_params() {
        let p = ScriptPolicy::compile(
            "fn update(s,e,p) { s.effects.a.samples += 1.0; p.n += 1; s.n = p.n; s }\n\
             fn weights(s,t,p) { s.effects.a.samples = 999.0; [p.n.to_float(), 1.0, 0.0] }",
            json!({"n": 1}),
        )
        .unwrap();
        let state = json!({"effects":{"a":{"samples":3.0}}});
        let expected = state.clone();
        let next = p.update(&state, &json!([])).unwrap();
        assert_eq!(next["effects"]["a"]["samples"], 4.0);
        assert_eq!(next["n"], 2);
        assert_eq!(p.update(&state, &json!([])).unwrap(), next);
        assert_eq!(p.weights(&state, &table()).unwrap(), vec![1.0, 1.0, 0.0]);
        let prepared = p.prepare_state(&state).unwrap();
        for _ in 0..2 {
            assert_eq!(
                p.weights_prepared(&prepared, &table()).unwrap(),
                vec![1.0, 1.0, 0.0]
            );
        }
        let after: Value = rhai::serde::from_dynamic(&prepared).unwrap();
        assert_eq!(after, expected);
        assert_eq!(state, expected);
    }

    #[test]
    fn operation_limit_stops_infinite_loop() {
        let p = ScriptPolicy::compile("fn update(s,e,p) { loop {} }", json!({})).unwrap();
        let error = p.update(&json!({}), &json!([])).unwrap_err();
        assert!(error.contains("operations"), "{error}");
    }

    #[test]
    fn invalid_outputs_are_errors() {
        for body in [
            "42",
            "[1, 2, 3]",
            "[1.0]",
            "[-1.0, 2.0, 0.0]",
            "[0.0, 0.0, 0.0]",
            "[1.0 / 0.0, 1.0, 0.0]",
        ] {
            let source = format!("fn weights(s,t,p) {{ {body} }}");
            let p = ScriptPolicy::compile(&source, json!({})).unwrap();
            assert!(p.weights(&json!({}), &table()).is_err(), "{body}");
        }
        for body in ["42", "#{bad: 1.0 / 0.0}"] {
            let source = format!("fn update(s,e,p) {{ {body} }}");
            let p = ScriptPolicy::compile(&source, json!({})).unwrap();
            assert!(p.update(&json!({}), &json!([])).is_err());
        }
    }

    #[test]
    fn input_limits_and_imports_are_enforced() {
        let p = policy(1);
        assert!(p
            .update(&json!({"x":"x".repeat(MAX_STRING + 1)}), &json!([]))
            .is_err());
        assert!(p
            .update(&json!({}), &json!(vec![0; MAX_ARRAY + 1]))
            .is_err());
        let mut deep = json!(null);
        for _ in 0..MAX_DEPTH + 2 {
            deep = json!({"child":deep});
        }
        assert!(p.update(&deep, &json!([])).is_err());
        assert!(ScriptPolicy::compile("import \"anything\" as io;", json!({})).is_err());
    }

    #[test]
    fn external_file_is_not_reread_after_load() {
        let path = std::env::temp_dir().join(format!(
            "fatalis-policy-{}-{}.rhai",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, default_script()).unwrap();
        let p = ScriptPolicy::load(&path, json!({})).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            p.weights(&json!({}), &table()).unwrap(),
            vec![65.0, 35.0, 0.0]
        );
    }
}
