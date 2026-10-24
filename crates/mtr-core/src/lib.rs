//! mtr-core — the engine: orchestration shared by the CLI and the napi
//! binding, with no I/O side effects beyond reading source files and
//! invoking the configured test runner. Presentation (stdout, reporters,
//! `std::process::exit`) stays with the caller.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use mtr_cache::{changed_line_ranges, mutant_signature, mutant_touches_changed_lines, MutantCache};
use mtr_config::Config;
use mtr_instrument::switch_instrument;
use mtr_mutators::Mutator;
use mtr_reporter_api::{FileMutantResults, Reporter};
use mtr_reporters::{HtmlReporter, JsonReporter, TextReporter};
use mtr_runner_jest::JestRunner;
use mtr_runner_vitest::VitestRunner;
use mtr_test_runner_api::{
    run_with_instrumented_file, run_with_instrumented_file_targeted, DynTestRunner,
};
use mtr_types::{Mutant, MutantId, MutantResult, MutantStatus};
use oxc_span::SourceType;
use serde::Serialize;

#[derive(Serialize)]
pub struct FileMutants {
    pub file: String,
    pub mutants: Vec<Mutant>,
}

type MutatorFactory = fn() -> Vec<Box<dyn Mutator>>;

/// Registers which `mutator_packs` config names are available, each backed
/// by a plain factory function — the boundary a plugin crate extends by
/// calling `register` instead of adding a match arm in `mtr-cli`.
/// `BTreeMap` keeps `names()` (used in error messages) in a stable order.
pub struct MutatorRegistry {
    packs: BTreeMap<&'static str, MutatorFactory>,
}

impl MutatorRegistry {
    pub fn new() -> Self {
        Self { packs: BTreeMap::new() }
    }

    pub fn register(&mut self, name: &'static str, factory: MutatorFactory) -> &mut Self {
        self.packs.insert(name, factory);
        self
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.packs.keys().copied().collect()
    }

    /// Combines the core mutator catalog (always included) with every named
    /// pack, in the order requested, then drops any operator (by
    /// `Mutator::name()`) listed in `excluded` — applied after assembly so
    /// it works uniformly across the core catalog and every pack.
    pub fn build(
        &self,
        requested: &[String],
        excluded: &[String],
    ) -> Result<Vec<Box<dyn Mutator>>, String> {
        let mut mutators = mtr_mutators::default_mutators();
        for name in requested {
            let factory = self.packs.get(name.as_str()).ok_or_else(|| {
                format!("unknown mutator pack `{name}` (available: {})", self.names().join(", "))
            })?;
            mutators.extend(factory());
        }
        mutators.retain(|m| !excluded.iter().any(|name| name == m.name()));
        Ok(mutators)
    }
}

impl Default for MutatorRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register("react", mtr_mutators_react::mutators);
        registry
    }
}

type ReporterFactory = fn() -> Box<dyn Reporter>;

/// Registers which `reporters` config names are available, each backed by a
/// plain factory function — same boundary shape as `MutatorRegistry`.
pub struct ReporterRegistry {
    reporters: BTreeMap<&'static str, ReporterFactory>,
}

impl ReporterRegistry {
    pub fn new() -> Self {
        Self { reporters: BTreeMap::new() }
    }

    pub fn register(&mut self, name: &'static str, factory: ReporterFactory) -> &mut Self {
        self.reporters.insert(name, factory);
        self
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.reporters.keys().copied().collect()
    }

    pub fn build(&self, requested: &[String]) -> Result<Vec<Box<dyn Reporter>>, String> {
        requested
            .iter()
            .map(|name| {
                self.reporters.get(name.as_str()).map(|factory| factory()).ok_or_else(|| {
                    format!("unknown reporter `{name}` (available: {})", self.names().join(", "))
                })
            })
            .collect()
    }
}

impl Default for ReporterRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register("json", || Box::new(JsonReporter::default()));
        registry.register("text", || Box::new(TextReporter));
        registry.register("html", || Box::new(HtmlReporter::default()));
        registry
    }
}

type TestRunnerFactory = fn(&str, Duration) -> Box<DynTestRunner>;

/// Registers which `testRunner` config names are available, each backed by a
/// plain factory function taking the project dir and timeout every runner
/// needs — same boundary shape as `MutatorRegistry`/`ReporterRegistry`, so
/// adding a runner (e.g. Mocha) is a `register` call, not a new match arm.
pub struct TestRunnerRegistry {
    runners: BTreeMap<&'static str, TestRunnerFactory>,
}

impl TestRunnerRegistry {
    pub fn new() -> Self {
        Self { runners: BTreeMap::new() }
    }

    pub fn register(&mut self, name: &'static str, factory: TestRunnerFactory) -> &mut Self {
        self.runners.insert(name, factory);
        self
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.runners.keys().copied().collect()
    }

    pub fn build(&self, name: &str, project: &str, timeout: Duration) -> Result<Box<DynTestRunner>, String> {
        let factory = self.runners.get(name).ok_or_else(|| {
            format!("unknown test runner `{name}` (available: {})", self.names().join(", "))
        })?;
        Ok(factory(project, timeout))
    }
}

impl Default for TestRunnerRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register("jest", |project, timeout| Box::new(JestRunner::new(project, timeout)));
        registry.register("vitest", |project, timeout| Box::new(VitestRunner::new(project, timeout)));
        registry
    }
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

/// Shared setup for both `switch_one_file` and `switch_one_file_with_coverage`:
/// read the file, scan for mutants, scope to the changed-since diff, and
/// split into cache hits (resolved immediately) vs. mutants that still need
/// to be tested. Pruning happens before instrumentation so cached mutants
/// never get embedded in the instrumented build at all, not just skipped at
/// run time.
struct ScanResult {
    original: String,
    absolute_file: PathBuf,
    /// Mutant id -> cache signature, for the mutants still in `to_test`.
    signatures: HashMap<u32, String>,
    cached_results: Vec<MutantResult>,
    to_test: Vec<Mutant>,
}

fn scan_and_prune(
    file: &Path,
    project: &str,
    cache: &Option<MutantCache>,
    changed_since: Option<&str>,
    mutators: &[Box<dyn Mutator>],
) -> ScanResult {
    let file_key = file.to_string_lossy().into_owned();
    let source_type = SourceType::from_path(file).unwrap_or_default();
    let original = std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", file.display()));
    let absolute_file = std::fs::canonicalize(file)
        .unwrap_or_else(|e| panic!("failed to resolve {}: {e}", file.display()));

    let mutants = mtr_mutators::scan_source_with_mutators(&original, source_type, mutators);
    let mutants = scope_to_diff(mutants, project, &absolute_file, &original, changed_since);

    let mut signatures: HashMap<u32, String> = HashMap::new();
    let mut cached_results: Vec<MutantResult> = Vec::new();
    let mut to_test: Vec<Mutant> = Vec::new();
    for mutant in mutants {
        let signature = mutant_signature(&file_key, &mutant, &original);
        match cache.as_ref().and_then(|c| c.get(&signature)) {
            Some(status) => cached_results.push(MutantResult { mutant, status }),
            None => {
                signatures.insert(mutant.id.0, signature);
                to_test.push(mutant);
            }
        }
    }

    ScanResult { original, absolute_file, signatures, cached_results, to_test }
}

/// The `switch` flow for one file, reusable across single-file callers and
/// the config-driven multi-file `mutate` flow. Doesn't save the cache —
/// callers own that (so a multi-file run can do it once, aggregated across
/// every file, instead of per file).
///
/// Returns the file's original source alongside its results — this function
/// already reads it to scan for mutants, so returning it lets reporters
/// (e.g. HTML, which highlights each mutant's `span` in context) use that
/// same read instead of re-reading the file themselves.
pub fn switch_one_file(
    file: &Path,
    project: &str,
    runner: &DynTestRunner,
    related_tests: bool,
    cache: &mut Option<MutantCache>,
    changed_since: Option<&str>,
    mutators: &[Box<dyn Mutator>],
) -> (Vec<MutantResult>, usize, String) {
    let scan = scan_and_prune(file, project, cache, changed_since, mutators);
    let cache_hits = scan.cached_results.len();
    let mut results = scan.cached_results;
    let mut signatures = scan.signatures;
    let related_to_file = related_tests.then_some(scan.absolute_file.as_path());

    if !scan.to_test.is_empty() {
        let instrumented = switch_instrument(&scan.original, &scan.to_test);
        let ids: Vec<MutantId> = scan.to_test.iter().map(|m| m.id).collect();
        let statuses: Vec<(MutantId, MutantStatus)> =
            run_with_instrumented_file(file, &instrumented, &ids, related_to_file, runner)
                .unwrap_or_else(|e| panic!("failed to run instrumented file: {e}"));
        let status_by_id: HashMap<u32, MutantStatus> =
            statuses.into_iter().map(|(id, status)| (id.0, status)).collect();

        for mutant in scan.to_test {
            let status = status_by_id[&mutant.id.0];
            if let Some(c) = cache.as_mut() {
                c.set(signatures.remove(&mutant.id.0).unwrap(), status);
            }
            results.push(MutantResult { mutant, status });
        }
    }

    (results, cache_hits, scan.original)
}

/// Like `switch_one_file`, but resolves which tests cover each mutant via
/// `TestRunner::collect_coverage` (one whole-suite run, diffed per test)
/// instead of running the whole suite — or a related-file prefilter —
/// against every mutant individually. A mutant with zero covering tests
/// resolves to `MutantStatus::NoCoverage` directly; no test process is ever
/// spawned for it.
pub fn switch_one_file_with_coverage(
    file: &Path,
    project: &str,
    runner: &DynTestRunner,
    cache: &mut Option<MutantCache>,
    changed_since: Option<&str>,
    mutators: &[Box<dyn Mutator>],
) -> Result<(Vec<MutantResult>, usize, String), String> {
    let scan = scan_and_prune(file, project, cache, changed_since, mutators);
    let cache_hits = scan.cached_results.len();
    let mut results = scan.cached_results;
    let mut signatures = scan.signatures;

    if scan.to_test.is_empty() {
        return Ok((results, cache_hits, scan.original));
    }

    let coverage = runner.collect_coverage(Some(&scan.absolute_file))?;

    let mut mutant_by_id: HashMap<u32, Mutant> = HashMap::new();
    let mut targeted: Vec<(MutantId, Vec<String>)> = Vec::new();
    for mutant in scan.to_test {
        let tests = mtr_coverage::tests_covering_mutant(
            &coverage,
            &scan.absolute_file,
            &scan.original,
            &mutant,
        );
        if tests.is_empty() {
            let signature = signatures.remove(&mutant.id.0).unwrap();
            if let Some(c) = cache.as_mut() {
                c.set(signature, MutantStatus::NoCoverage);
            }
            results.push(MutantResult { mutant, status: MutantStatus::NoCoverage });
        } else {
            targeted.push((mutant.id, tests));
            mutant_by_id.insert(mutant.id.0, mutant);
        }
    }

    if !targeted.is_empty() {
        let to_instrument: Vec<Mutant> = mutant_by_id.values().cloned().collect();
        let instrumented = switch_instrument(&scan.original, &to_instrument);
        let statuses =
            run_with_instrumented_file_targeted(file, &instrumented, &targeted, runner)
                .map_err(|e| format!("failed to run instrumented file: {e}"))?;

        for (id, status) in statuses {
            let mutant = mutant_by_id.remove(&id.0).expect("status for an untargeted mutant id");
            if let Some(c) = cache.as_mut() {
                c.set(signatures.remove(&id.0).unwrap(), status);
            }
            results.push(MutantResult { mutant, status });
        }
    }

    Ok((results, cache_hits, scan.original))
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
    let mutators =
        MutatorRegistry::default().build(&config.mutator_packs, &config.excluded_mutators)?;
    let runner = TestRunnerRegistry::default().build(&config.test_runner, &config.project, timeout)?;

    for file in &files {
        let (results, hits, source) = match config.test_filter.as_str() {
            "all" => switch_one_file(
                file,
                &config.project,
                runner.as_ref(),
                false,
                &mut cache,
                config.changed_since.as_deref(),
                &mutators,
            ),
            "related" => switch_one_file(
                file,
                &config.project,
                runner.as_ref(),
                true,
                &mut cache,
                config.changed_since.as_deref(),
                &mutators,
            ),
            "coverage" => switch_one_file_with_coverage(
                file,
                &config.project,
                runner.as_ref(),
                &mut cache,
                config.changed_since.as_deref(),
                &mutators,
            )?,
            other => {
                return Err(format!(
                    "unknown test filter `{other}` (available: all, related, coverage)"
                ))
            }
        };
        if results.is_empty() {
            continue;
        }
        cache_hits_total += hits;
        per_file.push(FileMutantResults { file: file.display().to_string(), mutants: results, source });
    }

    let (killed, survived, other) = per_file.iter().flat_map(|f| &f.mutants).fold(
        (0u32, 0u32, 0u32),
        |(k, s, o), r| match r.status {
            MutantStatus::Killed => (k + 1, s, o),
            // An uncovered mutant is exactly as bad for the score as one the
            // suite ran against and missed — folded into the same bucket as
            // Survived, even though it's reported as its own status above.
            MutantStatus::Survived | MutantStatus::NoCoverage => (k, s + 1, o),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutator_registry_includes_react_by_default() {
        assert_eq!(MutatorRegistry::default().names(), vec!["react"]);
    }

    #[test]
    fn mutator_registry_builds_core_catalog_with_no_packs_requested() {
        let core_len = mtr_mutators::default_mutators().len();
        let mutators = MutatorRegistry::default().build(&[], &[]).unwrap();
        assert_eq!(mutators.len(), core_len);
    }

    #[test]
    fn mutator_registry_extends_with_a_registered_pack() {
        let core_len = mtr_mutators::default_mutators().len();
        let react_len = mtr_mutators_react::mutators().len();
        let mutators = MutatorRegistry::default().build(&["react".to_string()], &[]).unwrap();
        assert_eq!(mutators.len(), core_len + react_len);
    }

    #[test]
    fn mutator_registry_rejects_unknown_pack_without_panicking() {
        let Err(err) = MutatorRegistry::default().build(&["unknown".to_string()], &[]) else {
            panic!("expected an error for an unregistered pack");
        };
        assert!(err.contains("unknown"), "error should name the bad pack: {err}");
        assert!(err.contains("react"), "error should list available packs: {err}");
    }

    #[test]
    fn mutator_registry_accepts_a_custom_registration() {
        let mut registry = MutatorRegistry::new();
        registry.register("custom", Vec::new);
        assert!(registry.build(&["custom".to_string()], &[]).is_ok());
    }

    #[test]
    fn mutator_registry_excludes_named_operators_from_the_core_catalog() {
        let mutators =
            MutatorRegistry::default().build(&[], &["StringLiteral".to_string()]).unwrap();
        assert!(mutators.iter().all(|m| m.name() != "StringLiteral"));
    }

    #[test]
    fn mutator_registry_excludes_named_operators_from_a_pack_too() {
        let mutators = MutatorRegistry::default()
            .build(&["react".to_string()], &["JsxBooleanAttribute".to_string()])
            .unwrap();
        assert!(mutators.iter().all(|m| m.name() != "JsxBooleanAttribute"));
    }

    #[test]
    fn reporter_registry_includes_json_text_and_html_by_default() {
        assert_eq!(ReporterRegistry::default().names(), vec!["html", "json", "text"]);
    }

    #[test]
    fn reporter_registry_rejects_unknown_reporter_without_panicking() {
        let Err(err) = ReporterRegistry::default().build(&["unknown".to_string()]) else {
            panic!("expected an error for an unregistered reporter");
        };
        assert!(err.contains("unknown"), "error should name the bad reporter: {err}");
        assert!(err.contains("json"), "error should list available reporters: {err}");
    }

    #[test]
    fn test_runner_registry_includes_jest_and_vitest_by_default() {
        assert_eq!(TestRunnerRegistry::default().names(), vec!["jest", "vitest"]);
    }

    #[test]
    fn test_runner_registry_builds_a_registered_runner() {
        assert!(TestRunnerRegistry::default()
            .build("jest", ".", Duration::from_secs(30))
            .is_ok());
        assert!(TestRunnerRegistry::default()
            .build("vitest", ".", Duration::from_secs(30))
            .is_ok());
    }

    #[test]
    fn test_runner_registry_rejects_unknown_runner_without_panicking() {
        let Err(err) = TestRunnerRegistry::default().build("unknown", ".", Duration::from_secs(30))
        else {
            panic!("expected an error for an unregistered test runner");
        };
        assert!(err.contains("unknown"), "error should name the bad runner: {err}");
        assert!(err.contains("jest"), "error should list available runners: {err}");
    }

    #[test]
    fn test_runner_registry_accepts_a_custom_registration() {
        struct NoopRunner;
        impl mtr_test_runner_api::TestRunner for NoopRunner {
            fn run(&self, _opts: mtr_test_runner_api::RunOptions) -> MutantStatus {
                MutantStatus::Killed
            }
        }
        impl std::panic::RefUnwindSafe for NoopRunner {}

        let mut registry = TestRunnerRegistry::new();
        registry.register("noop", |_project, _timeout| Box::new(NoopRunner));
        assert!(registry.build("noop", ".", Duration::from_secs(30)).is_ok());
    }
}
