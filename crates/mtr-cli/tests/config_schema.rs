use std::path::Path;

use schemars::schema_for;

/// The committed `mutate.config.schema.json` is a public artifact — it ships
/// inside the `mutate-js` npm package (`crates/mtr-napi/`) so a config's
/// `$schema` still resolves after a real `npm install`, and configs
/// reference it for editor validation/autocomplete. This guards against it
/// silently drifting out of sync with `mtr_config::Config`.
#[test]
fn committed_schema_matches_the_config_struct() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let committed_path = workspace_root.join("crates/mtr-napi/mutate.config.schema.json");

    let committed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&committed_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", committed_path.display())),
    )
    .expect("committed schema is not valid JSON");

    let generated: serde_json::Value =
        serde_json::to_value(schema_for!(mtr_config::Config)).unwrap();

    assert_eq!(
        committed, generated,
        "mutate.config.schema.json is out of sync with mtr_config::Config — \
         regenerate with `cargo run -p mtr-cli -- config-schema > mutate.config.schema.json`"
    );
}
