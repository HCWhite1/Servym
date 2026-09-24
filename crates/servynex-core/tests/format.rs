//! Le formateur produit, octet par octet, la même sortie que `formatConfig` de la version Node
//! (fichiers de référence écrits par tools/gen-format-golden.mjs).

use std::path::Path;

use servynex_core::config::format_config;

#[test]
fn meme_sortie_que_la_version_node() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let input: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let expected = std::fs::read_to_string(path.with_extension("out")).unwrap();
        assert_eq!(
            format!("{}\n", format_config(&input)),
            expected,
            "{}",
            path.display()
        );
        checked += 1;
    }
    assert!(checked >= 6, "fichiers de référence absents");
}
