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
use mtr_test_runner_api::{run_with_instrumented_file, DynTestRunner};
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

/// The `switch` flow for one file, reusable across single-file callers and
/// the config-driven multi-file `mutate` flow. Doesn't save the cache —
/// callers own that (so a multi-file run can do it once, aggregated across
/// every file, instead of per file).
///
/// Returns the file's original source alongside its results — this function
/// already reads it to scan for mutants, so returning it lets reporters
/// (e.g. HTML, which highlights each mutant's `span` in context) use that
/// same read instead of re-reading the file themselves.
#[allow(clippy::too_many_arguments)]
pub fn switch_one_file(
    file: &Path,
    project: &str,
    runner: &DynTestRunner,
    related_tests: bool,
    cache: &mut Option<MutantCache>,
    changed_since: Option<&str>,
    mutators: &[Box<dyn Mutator>],
) -> (Vec<MutantResult>, usize, String) {
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
        let statuses: Vec<(MutantId, MutantStatus)> =
            run_with_instrumented_file(file, &instrumented, &ids, related_to_file, runner)
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

    (results, cache_hits, original)
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
        let (results, hits, source) = switch_one_file(
            file,
            &config.project,
            runner.as_ref(),
            config.related_tests,
            &mut cache,
            config.changed_since.as_deref(),
            &mutators,
        );
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
