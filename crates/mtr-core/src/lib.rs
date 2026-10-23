//! mtr-core — the engine: orchestration shared by the CLI and the napi
//! binding, with no I/O side effects beyond reading source files and
//! invoking the configured test runner. Presentation (stdout, reporters,
//! `std::process::exit`) stays with the caller.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mtr_cache::{changed_line_ranges, mutant_signature, mutant_touches_changed_lines, MutantCache};
use mtr_config::{Config, TestRunner};
use mtr_instrument::switch_instrument;
use mtr_mutators::Mutator;
use mtr_reporter_api::FileMutantResults;
use mtr_runner_jest::JestRunner;
use mtr_runner_vitest::VitestRunner;
use mtr_test_runner_api::run_with_instrumented_file;
use mtr_types::{Mutant, MutantId, MutantResult, MutantStatus};
use oxc_span::SourceType;
use serde::Serialize;

#[derive(Serialize)]
pub struct FileMutants {
    pub file: String,
    pub mutants: Vec<Mutant>,
}

/// Combines the core mutator catalog with any requested framework-extension
/// packs. The single place a new pack (beyond `react`) gets registered.
pub fn build_mutators(packs: &[String]) -> Vec<Box<dyn Mutator>> {
    let mut mutators = mtr_mutators::default_mutators();
    for pack in packs {
        match pack.as_str() {
            "react" => mutators.extend(mtr_mutators_react::mutators()),
            other => panic!("unknown mutator pack `{other}` (expected `react`)"),
        }
    }
    mutators
}

/// Discovers mutants in every file matching `pattern`, without running any
/// tests.
pub fn scan_files(pattern: &str, mutators: &[Box<dyn Mutator>]) -> Result<Vec<FileMutants>, String> {
    let paths = glob::glob(pattern).map_err(|e| format!("invalid glob `{pattern}`: {e}"))?;
    let mut results = Vec::new();
    for entry in paths {
        let path = entry.map_err(|e| format!("failed to read glob entry: {e}"))?;
        let source_type = SourceType::from_path(&path).unwrap_or_default();
        let source = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let mutants = mtr_mutators::scan_source_with_mutators(&source, source_type, mutators);
        results.push(FileMutants { file: path.display().to_string(), mutants });
    }
    Ok(results)
}

/// Drops mutants that don't touch any line changed since `changed_since`
/// (when set); a no-op otherwise. Applied before caching/instrumentation so
/// out-of-scope mutants never even get tested.
pub fn scope_to_diff(
    mutants: Vec<Mutant>,
    project: &str,
    file: &Path,
    original: &str,
    changed_since: Option<&str>,
) -> Vec<Mutant> {
    let Some(git_ref) = changed_since else { return mutants };
    let changed = changed_line_ranges(Path::new(project), git_ref, file);
    mutants.into_iter().filter(|m| mutant_touches_changed_lines(m, original, &changed)).collect()
}

/// The `switch` flow for one file, reusable across single-file callers and
/// the config-driven multi-file `mutate` flow. Doesn't save the cache —
/// callers own that (so a multi-file run can do it once, aggregated across
/// every file, instead of per file).
#[allow(clippy::too_many_arguments)]
pub fn switch_one_file(
    file: &Path,
    project: &str,
    runner: TestRunner,
    timeout: Duration,
    related_tests: bool,
    cache: &mut Option<MutantCache>,
    changed_since: Option<&str>,
    mutators: &[Box<dyn Mutator>],
) -> (Vec<MutantResult>, usize) {
    let file_key = file.to_string_lossy().into_owned();
    let source_type = SourceType::from_path(file).unwrap_or_default();
    let original = std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", file.display()));
    let absolute_file = std::fs::canonicalize(file)
        .unwrap_or_else(|e| panic!("failed to resolve {}: {e}", file.display()));
    let related_to_file = related_tests.then_some(absolute_file.as_path());

    let mutants = mtr_mutators::scan_source_with_mutators(&original, source_type, mutators);
    let mutants = scope_to_diff(mutants, project, &absolute_file, &original, changed_since);

    // Prune before instrumentation: cached mutants never get embedded in the
    // instrumented build at all, not just skipped at run time.
    let mut signatures: HashMap<u32, String> = HashMap::new();
    let mut results: Vec<MutantResult> = Vec::new();
    let mut to_test: Vec<Mutant> = Vec::new();
    for mutant in mutants {
        let signature = mutant_signature(&file_key, &mutant, &original);
        match cache.as_ref().and_then(|c| c.get(&signature)) {
            Some(status) => results.push(MutantResult { mutant, status }),
            None => {
                signatures.insert(mutant.id.0, signature);
                to_test.push(mutant);
            }
        }
    }
    let cache_hits = results.len();

    if !to_test.is_empty() {
        let instrumented = switch_instrument(&original, &to_test);
        let ids: Vec<MutantId> = to_test.iter().map(|m| m.id).collect();
        let statuses: Vec<(MutantId, MutantStatus)> = match runner {
            TestRunner::Jest => run_with_instrumented_file(
                file,
                &instrumented,
                &ids,
                related_to_file,
                &JestRunner::new(project, timeout),
            ),
            TestRunner::Vitest => run_with_instrumented_file(
                file,
                &instrumented,
                &ids,
                related_to_file,
                &VitestRunner::new(project, timeout),
            ),
        }
        .unwrap_or_else(|e| panic!("failed to run instrumented file: {e}"));
        let status_by_id: HashMap<u32, MutantStatus> =
            statuses.into_iter().map(|(id, status)| (id.0, status)).collect();

        for mutant in to_test {
            let status = status_by_id[&mutant.id.0];
            if let Some(c) = cache.as_mut() {
                c.set(signatures.remove(&mutant.id.0).unwrap(), status);
            }
            results.push(MutantResult { mutant, status });
        }
    }

    (results, cache_hits)
}

#[derive(Serialize)]
pub struct MutateSummary {
    pub files: Vec<FileMutantResults>,
    pub score: f64,
    pub killed: u32,
    pub survived: u32,
    pub other: u32,
    pub cache_hits: usize,
    pub total_mutants: usize,
}

/// Resolves `config.mutate`/`ignorePatterns` to a concrete file list,
/// mutation-tests every file via the `switch` path, aggregates a mutation
/// score, and persists the cache — everything the `mutate` CLI command and
/// the napi binding both need, minus reporters and process exit codes,
/// which are presentation concerns owned by the caller.
pub fn run_mutate(config: &Config) -> Result<MutateSummary, String> {
    let ignore_globs: Vec<glob::Pattern> = config
        .ignore_patterns
        .iter()
        .map(|p| glob::Pattern::new(p).map_err(|e| format!("invalid ignore pattern `{p}`: {e}")))
        .collect::<Result<_, _>>()?;

    let mut files: Vec<PathBuf> = Vec::new();
    for pattern in &config.mutate {
        for entry in glob::glob(pattern).map_err(|e| format!("invalid glob `{pattern}`: {e}"))? {
            let path = entry.map_err(|e| format!("failed to read glob entry: {e}"))?;
            if !ignore_globs.iter().any(|g| g.matches_path(&path)) {
                files.push(path);
            }
        }
    }

    let timeout = Duration::from_secs(config.timeout_secs);
    let mut cache = config.cache.as_deref().map(|p| MutantCache::load(Path::new(p)));
    let mut cache_hits_total = 0;
    let mut per_file: Vec<FileMutantResults> = Vec::new();
    let mutators = build_mutators(&config.mutator_packs);

    for file in &files {
        let (results, hits) = switch_one_file(
            file,
            &config.project,
            config.test_runner,
            timeout,
            config.related_tests,
            &mut cache,
            config.changed_since.as_deref(),
            &mutators,
        );
        if results.is_empty() {
            continue;
        }
        cache_hits_total += hits;
        per_file.push(FileMutantResults { file: file.display().to_string(), mutants: results });
    }

    let (killed, survived, other) = per_file.iter().flat_map(|f| &f.mutants).fold(
        (0u32, 0u32, 0u32),
        |(k, s, o), r| match r.status {
            MutantStatus::Killed => (k + 1, s, o),
            MutantStatus::Survived => (k, s + 1, o),
            _ => (k, s, o + 1),
        },
    );
    let tested = killed + survived;
    let score = if tested > 0 { f64::from(killed) / f64::from(tested) * 100.0 } else { 100.0 };
    let total_mutants: usize = per_file.iter().map(|f| f.mutants.len()).sum();

    if let Some(path) = config.cache.as_deref() {
        if let Some(cache) = &cache {
            cache.save(Path::new(path)).map_err(|e| format!("failed to save cache to {path}: {e}"))?;
        }
    }

    Ok(MutateSummary {
        files: per_file,
        score,
        killed,
        survived,
        other,
        cache_hits: cache_hits_total,
        total_mutants,
    })
}
