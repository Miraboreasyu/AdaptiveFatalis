#![cfg(feature = "game-fixtures")]
use fatalis_adaptive::register_audit::RegisterAudit;
#[test]
fn audit_all_sixty_fatalis_scripts_and_reproduce_unused_slots() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/fatalis_thk");
    let mut audit = RegisterAudit::default();
    for entry in std::fs::read_dir(path).unwrap() {
        let p = entry.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy();
        if name.starts_with("em013_") && name.ends_with(".thk") {
            audit.add(&name, &std::fs::read(&p).unwrap()).unwrap();
        }
    }
    assert_eq!(audit.sources.len(), 60);
    assert_eq!(audit.unused_slots(), vec![14, 15, 16, 17, 18, 19]);
    audit.require_unreferenced(&[14, 15]).unwrap();
    assert!(audit.require_unreferenced(&[21]).is_err());
    assert!(audit
        .uses
        .iter()
        .any(|u| u.source == "em013_17.thk" && u.slot == 21));
}
#[test]
fn failed_parse_is_atomic_and_empty_audit_cannot_certify_slots() {
    let mut audit = RegisterAudit::default();
    assert!(audit.require_unreferenced(&[14]).is_err());
    assert!(audit.add("broken", &[]).is_err());
    assert!(audit.sources.is_empty());
    assert!(audit.uses.is_empty());
}
