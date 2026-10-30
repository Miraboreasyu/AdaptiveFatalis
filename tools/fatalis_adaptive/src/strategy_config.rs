//! Player-facing selection and strategy bundles. Runtime protocol is internal.
use crate::config::{Config, RegisterMode, ScriptLearning};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    pub strategy: String,
}
impl Default for Selection {
    fn default() -> Self {
        Self {
            strategy: "none".into(),
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub id: String,
    pub state_version: u32,
    pub script: String,
    pub params: String,
}
fn simple(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn member(root: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains(['/', '\\', ':']) || name == "." || name == ".." {
        return Err("strategy member must be a plain filename".into());
    }
    let path = root
        .join(name)
        .canonicalize()
        .map_err(|e| format!("strategy member {name}: {e}"))?;
    if path.parent() != Some(root) || !path.is_file() {
        return Err("strategy member escapes bundle".into());
    }
    Ok(path)
}
pub fn load(dir: &Path) -> Result<Config, String> {
    let selection: Selection =
        serde_json::from_slice(&fs::read(dir.join("config.json")).map_err(|e| e.to_string())?)
            .map_err(|e| format!("player config (only strategy is supported): {e}"))?;
    resolve(dir, &selection)
}
pub fn resolve(dir: &Path, selection: &Selection) -> Result<Config, String> {
    let mut config = Config {
        enabled: true,
        allow_unverified_offsets: true,
        register: Some(RegisterMode {
            table_code_slots: [14, 15, 16],
            option_slots: [17, 18],
            counter_slot: 19,
            target_tables: vec![],
            addresses: None,
        }),
        ..Config::default()
    };
    config.policy.orientation = 0.0;
    if selection.strategy == "none" {
        return Ok(config);
    }
    if !simple(&selection.strategy) {
        return Err("invalid strategy name".into());
    }
    let strategies = dir
        .join("strategies")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let folder = strategies
        .join(&selection.strategy)
        .canonicalize()
        .map_err(|e| format!("strategy not found: {e}"))?;
    if folder.parent() != Some(strategies.as_path()) {
        return Err("strategy folder escapes strategies directory".into());
    }
    let manifest = member(&folder, "strategy.json")?;
    let bundle: Bundle = serde_json::from_slice(&fs::read(manifest).map_err(|e| e.to_string())?)
        .map_err(|e| format!("strategy manifest: {e}"))?;
    if !simple(&bundle.id) || bundle.state_version == 0 {
        return Err("strategy requires simple id and positive state_version".into());
    }
    let params: serde_json::Value = serde_json::from_slice(
        &fs::read(member(&folder, &bundle.params)?).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("strategy params: {e}"))?;
    if !params.is_object() {
        return Err("strategy params must be a JSON object".into());
    }
    let script = member(&folder, &bundle.script)?;
    config.scripted_learning = Some(ScriptLearning {
        script: script.to_string_lossy().into_owned(),
        profile: format!("{}-v{}", bundle.id, bundle.state_version),
        params,
        import_legacy: false,
        debug: true,
    });
    config.validate()?;
    Ok(config)
}
