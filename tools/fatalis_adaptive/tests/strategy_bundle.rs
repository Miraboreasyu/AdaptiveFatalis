use fatalis_adaptive::{
    strategy_config::{self, Selection},
    ScriptPolicy,
};
use std::path::Path;
#[test]
fn selection_loads_bundled_params_and_fixed_protocol() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let none = strategy_config::resolve(root, &Selection::default()).unwrap();
    assert!(none.scripted_learning.is_none());
    assert_eq!(none.register.unwrap().slots(), [14, 15, 16, 17, 18, 19]);
    for name in [
        "standard-malevolent",
        "standard-benevolent",
        "fast-malevolent",
        "fast-benevolent",
    ] {
        let c = strategy_config::resolve(
            root,
            &Selection {
                strategy: name.into(),
            },
        )
        .unwrap();
        let s = c.scripted_learning.unwrap();
        assert_eq!(s.profile, format!("{name}-v1"));
        assert_eq!(
            s.params["direction"].as_f64().unwrap(),
            if name.ends_with("-benevolent") {
                -1.0
            } else {
                1.0
            }
        );
        ScriptPolicy::load(Path::new(&s.script), s.params).unwrap();
    }
    assert!(strategy_config::resolve(
        root,
        &Selection {
            strategy: "../bad".into()
        }
    )
    .is_err());
    assert!(strategy_config::resolve(
        root,
        &Selection {
            strategy: "missing".into()
        }
    )
    .is_err());
    assert!(serde_json::from_str::<Selection>(r#"{"strategy":"none","register":{}}"#).is_err());
}
