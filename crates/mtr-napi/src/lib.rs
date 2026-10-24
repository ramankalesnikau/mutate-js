//! mtr-napi — thin napi-rs binding over the `mtr-core` engine for Node/TS
//! consumers. Results cross the FFI boundary as JSON strings (not mapped
//! napi objects) — the engine's types already have a stable `Serialize`
//! shape (the committed JSON Schema in this crate), so re-deriving a
//! parallel napi-object mapping would just be a second representation to
//! keep in sync.

#[macro_use]
extern crate napi_derive;

use napi::bindgen_prelude::*;

/// Discovers mutants in every file matching `glob`, without running any
/// tests. Mirrors `mtr-cli scan`. Returns the same JSON shape as the CLI.
#[napi]
pub fn scan(glob: String, react: bool) -> Result<String> {
    let packs = if react { vec!["react".to_string()] } else { Vec::new() };
    let mutators = mtr_core::MutatorRegistry::default()
        .build(&packs, &[])
        .map_err(|e| Error::new(Status::GenericFailure, e))?;
    let results = mtr_core::scan_files(&glob, &mutators)
        .map_err(|e| Error::new(Status::GenericFailure, e))?;
    serde_json::to_string(&results).map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
}

/// Mutation-tests every file matched by the config's `mutate` globs (the
/// `switch` path), aggregates a mutation score, and persists the cache.
/// Mirrors `mtr-cli mutate`, minus reporters and process exit codes — the
/// Node caller decides what to do with a below-threshold score.
///
/// `config_json` is the config already parsed from JSON/JSONC on the JS
/// side (e.g. via `mutate.config.schema.json`-validated JSONC), serialized
/// back to plain JSON — avoids needing a second JSONC parser in Rust just
/// for this entry point.
#[napi]
pub fn mutate(config_json: String) -> Result<String> {
    let config: mtr_config::Config = serde_json::from_str(&config_json)
        .map_err(|e| Error::new(Status::InvalidArg, format!("invalid config: {e}")))?;
    let summary =
        mtr_core::run_mutate(&config).map_err(|e| Error::new(Status::GenericFailure, e))?;
    serde_json::to_string(&summary).map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
}
