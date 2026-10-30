//! Standalone offline AI query/editor. Never loads a DLL, attaches, or deploys.
use fatalis_adaptive::{
    geometry::{GeometryModel, Query},
    instrument::{instrumentable_tables, Registers},
    thk::{Project, Thk},
    weight_edit::{self, EditRequest},
    AiDocument,
};
use std::{collections::BTreeMap, env, fs, path::Path};
fn arg(a: &[String], i: usize) -> Result<&str, String> {
    a.get(i)
        .map(String::as_str)
        .ok_or("missing argument; run help".into())
}
fn parse_slots<const N: usize>(text: &str) -> Result<[u8; N], String> {
    let values = text
        .split(',')
        .map(|value| {
            value
                .trim()
                .parse::<u8>()
                .map_err(|_| format!("invalid slot in {text:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    values
        .try_into()
        .map_err(|_| format!("expected {N} comma-separated slots, got {text:?}"))
}
fn main() {
    if let Err(e) = run() {
        eprintln!("fatalis-ai: {e}");
        std::process::exit(1);
    }
}
fn print_json<T: serde::Serialize>(value: &T) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|e| e.to_string())?
    );
    Ok(())
}
fn run() -> Result<(), String> {
    let a: Vec<String> = env::args().collect();
    let command = a.get(1).map(String::as_str).unwrap_or("help");
    if !matches!(
        command,
        "query" | "edit" | "template" | "tables" | "instrument" | "policy-replay"
    ) {
        println!("Offline Fatalis AI model (no game access)\n  query ORIGINAL_MAIN ORIGINAL_GLOBAL DISTANCE ANGLE_CW [ENTRY_NODE=0] [CURRENT_MAIN]\n  template ORIGINAL_MAIN ORIGINAL_GLOBAL [CURRENT_MAIN]\n  edit ORIGINAL_MAIN ORIGINAL_GLOBAL CURRENT_MAIN EDIT_JSON NEW_OUTPUT\n  tables ORIGINAL_MAIN ORIGINAL_GLOBAL [TABLE_ID...]\n  instrument ORIGINAL_MAIN ORIGINAL_GLOBAL TABLE_CODE_SLOTS OPTION_SLOTS COUNTER_SLOT OUT_THK OUT_MANIFEST [TABLE_ID...]\nQuery emits independent tables with direct Global targets and path constraints.\nTemplate emits hash-pinned current weights; retain only the tables you want to edit.\nEdit never overwrites existing output and reports every changed byte.\nTables lists instrumentable leaf tables and positive arms. Empty table lists in\nInstrument mean all discovered tables; TABLE_CODE_SLOTS is \"hundreds,tens,units\"\nand OPTION_SLOTS is \"tens,units\".\nInstrument writes a structurally instrumented main THK plus its ID manifest;\nslots are physical and must still be audited/proven against the running build.");
        println!(
            "  policy-replay MAIN GLOBAL PRIVATE_DIR NEW_OUTPUT (use a COPY of state directory)"
        );
        return Ok(());
    }
    let p = Project::from_paths(arg(&a, 2)?, arg(&a, 3)?)?;
    match command {
        "policy-replay" => {
            // Use a COPY of the private directory; replay advances its profile cursor.
            let dir = Path::new(arg(&a, 4)?);
            let config = fatalis_adaptive::strategy_config::load(dir)?;
            let baseline = AiDocument::parse(&p.main.bytes, &p.global.bytes)?;
            let legacy_path = dir.join("register/learning.json");
            let legacy = if legacy_path.exists() {
                serde_json::from_slice(&fs::read(legacy_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?
            } else {
                fatalis_adaptive::learning::Learning::default()
            };
            let mut store = match &config.scripted_learning {
                Some(cfg) => fatalis_adaptive::policy_store::PolicyStore::open(
                    &dir.join("register"),
                    cfg,
                    &legacy,
                )?,
                None => fatalis_adaptive::policy_store::PolicyStore::open_no_learning(
                    &dir.join("register"),
                )?,
            };
            let document = store.refresh(&baseline)?;
            let output = arg(&a, 5)?;
            use std::io::Write;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output)
                .map_err(|e| e.to_string())?
                .write_all(&document.to_thk()?)
                .map_err(|e| e.to_string())?;
            print_json(
                &serde_json::json!({"hunts":store.hunts(),"output":output,"note":"offline profile advanced; use a copy of private directory"}),
            )?;
        }
        "query" => {
            let q = Query {
                distance: arg(&a, 4)?.parse().map_err(|_| "invalid distance")?,
                angle_cw: arg(&a, 5)?.parse().map_err(|_| "invalid angle")?,
                entry_node: a
                    .get(6)
                    .map(|s| s.parse::<usize>().map_err(|_| "invalid entry node"))
                    .transpose()?
                    .unwrap_or(0),
            };
            let mut result = GeometryModel::new(&p).query(&q)?;
            if let Some(current) = a.get(7) {
                let bytes = fs::read(current).map_err(|e| e.to_string())?;
                p.patch_current_main_weights(&bytes, &BTreeMap::new())?;
                let parsed = Thk::parse(bytes)?;
                for table in &mut result.tables {
                    for option in &mut table.options {
                        option.weight = parsed.nodes[table.node].segments[option.segment].p1;
                    }
                }
            }
            print_json(&result)?;
        }
        "tables" => {
            let document = AiDocument::parse(&p.main.bytes, &p.global.bytes)?;
            let all = instrumentable_tables(&document)?;
            let selected = if a.len() > 4 {
                Some(a[4..].to_vec())
            } else {
                None
            };
            let selected_arms = selected
                .as_ref()
                .map(|ids| {
                    let mut sum = 0usize;
                    for id in ids {
                        let (_, arms) = all
                            .iter()
                            .find(|(table, _)| table == id)
                            .ok_or_else(|| format!("table is not instrumentable: {id}"))?;
                        sum += arms;
                    }
                    Ok::<usize, String>(sum)
                })
                .transpose()?;
            let total_arms: usize = all.iter().map(|(_, arms)| arms).sum();
            let listed = match &selected {
                Some(ids) => ids
                    .iter()
                    .map(|id| {
                        all.iter()
                            .find(|(table, _)| table == id)
                            .expect("selected IDs were validated above")
                    })
                    .collect::<Vec<_>>(),
                None => all.iter().collect(),
            };
            print_json(&serde_json::json!({
                "tables": listed.iter().map(|(id, arms)| serde_json::json!({
                    "table_id": id,
                    "positive_arms": arms,
                })).collect::<Vec<_>>(),
                "all_tables": all.len(),
                "all_positive_arms": total_arms,
                "max_table_codes": 1000,
                "max_arms_per_table": 100,
                "selected_positive_arms": selected_arms,
                "note": "Decimal-digit wrappers encode a sorted table index and an exit index, so all discovered tables can be instrumented in one THK without the old 256-ID cap. Runtime sampling still requires the derived context or verified explicit addresses."
            }))?;
        }
        "template" => {
            let bytes = if let Some(current) = a.get(4) {
                fs::read(current).map_err(|e| e.to_string())?
            } else {
                p.main.bytes.clone()
            };
            p.patch_current_main_weights(&bytes, &BTreeMap::new())?;
            let parsed = Thk::parse(bytes.clone())?;
            let tables = p
                .blocks
                .iter()
                .map(|b| {
                    (
                        b.id.clone(),
                        b.branches
                            .iter()
                            .map(|r| parsed.nodes[b.node].segments[r.segment].p1)
                            .collect(),
                    )
                })
                .collect();
            print_json(&EditRequest {
                source_sha256: fatalis_adaptive::thk::sha256(&bytes),
                tables,
            })?;
        }
        "edit" => {
            let current = fs::read(arg(&a, 4)?).map_err(|e| e.to_string())?;
            let request: EditRequest =
                serde_json::from_slice(&fs::read(arg(&a, 5)?).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let (bytes, report) = weight_edit::apply(&p, &current, &request)?;
            let out = Path::new(arg(&a, 6)?);
            use std::io::Write;
            // create_new protects existing files, including the input file.
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out)
                .map_err(|e| e.to_string())?;
            f.write_all(&bytes)
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
            print_json(&report)?;
        }
        "instrument" => {
            let table_code_slots = parse_slots::<3>(arg(&a, 4)?)?;
            let option_slots = parse_slots::<2>(arg(&a, 5)?)?;
            let counter_slot: u8 = arg(&a, 6)?.parse().map_err(|_| "invalid counter slot")?;
            let out = Path::new(arg(&a, 7)?);
            let manifest = Path::new(arg(&a, 8)?);
            if out.exists() || manifest.exists() {
                return Err("refusing to overwrite existing output".into());
            }
            let document = AiDocument::parse(&p.main.bytes, &p.global.bytes)?;
            let tables = if a.len() > 9 {
                a[9..].to_vec()
            } else {
                instrumentable_tables(&document)?
                    .into_iter()
                    .map(|(table, _)| table)
                    .collect()
            };
            let compiled = document.to_instrumented_thk(
                &tables,
                Registers {
                    table_code: table_code_slots,
                    option: option_slots,
                    counter: counter_slot,
                },
            )?;
            fs::write(out, &compiled.bytes).map_err(|e| e.to_string())?;
            fs::write(
                manifest,
                serde_json::to_vec_pretty(&compiled.manifest).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            print_json(&serde_json::json!({
                "output": out,
                "manifest": manifest,
                "bytes": compiled.bytes.len(),
                "sha256": fatalis_adaptive::thk::sha256(&compiled.bytes),
                "markers": compiled.manifest.markers.len(),
                "selected_tables": tables.len(),
                "safety": compiled.manifest.safety,
            }))?;
        }
        _ => unreachable!(),
    }
    Ok(())
}
