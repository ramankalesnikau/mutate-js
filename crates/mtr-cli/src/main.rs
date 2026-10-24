use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Parser as ClapParser, Subcommand, ValueEnum};
use mtr_cache::{mutant_signature, MutantCache};
use mtr_core::{scan_files, scope_to_diff, switch_one_file, MutatorRegistry, ReporterRegistry};
use mtr_mutators::apply_mutant;
use mtr_reporter_api::FileMutantResults;
use mtr_runner_jest::JestRunner;
use mtr_runner_vitest::VitestRunner;
use mtr_test_runner_api::run_with_file_swapped;
use mtr_types::{MutantResult, MutantStatus};
use oxc_span::SourceType;
use schemars::schema_for;

#[derive(ClapParser)]
#[command(name = "mutate-js")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Discover mutants in files matching a glob, without running any tests.
    Scan {
        glob: String,
        /// Include the `mtr-mutators-react` operator pack (JSX-specific
        /// mutators) alongside the core catalog.
        #[arg(long)]
        react: bool,
    },
    /// Mutate one file, running the whole suite once per mutant (naive: no
    /// coverage-based test filtering yet).
    Run {
        file: String,
        #[arg(long, default_value = ".")]
        project: String,
        #[arg(long, value_enum, default_value = "jest")]
        runner: RunnerKind,
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
        /// Only run tests related to `file` (the runner's own dependency
        /// resolver), instead of the whole suite.
        #[arg(long)]
        related_tests: bool,
        /// Path to a JSON cache file. When set, mutants whose enclosing code
        /// is unchanged since the last recorded run reuse that result
        /// instead of being re-tested.
        #[arg(long)]
        cache: Option<String>,
        /// Only test mutants touching lines changed since this git ref
        /// (e.g. `origin/main`). Requires `file` to be inside a git repo.
        #[arg(long)]
        changed_since: Option<String>,
    },
    /// Like `run`, but embeds every mutant in one instrumented file and
    /// switches between them via an env var instead of rewriting the file
    /// per mutant.
    Switch {
        file: String,
        #[arg(long, default_value = ".")]
        project: String,
        #[arg(long, value_enum, default_value = "jest")]
        runner: RunnerKind,
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
        #[arg(long)]
        related_tests: bool,
        #[arg(long)]
        cache: Option<String>,
        #[arg(long)]
        changed_since: Option<String>,
    },
    /// Mutation-test every file matched by the config's `mutate` globs
    /// (using the fast `switch` path), aggregate a mutation score, and exit
    /// non-zero if it falls below `thresholds.break`.
    Mutate {
        /// Defaults to `mutate.config.jsonc` in the current directory; if
        /// that file doesn't exist, runs with built-in defaults.
        #[arg(long)]
        config: Option<String>,
    },
    /// Print the JSON Schema for the config file, for editor autocomplete.
    ConfigSchema,
}

#[derive(Clone, ValueEnum)]
enum RunnerKind {
    Jest,
    Vitest,
}

impl From<RunnerKind> for mtr_config::TestRunner {
    fn from(kind: RunnerKind) -> Self {
        match kind {
            RunnerKind::Jest => mtr_config::TestRunner::Jest,
            RunnerKind::Vitest => mtr_config::TestRunner::Vitest,
        }
    }
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan { glob, react } => scan(&glob, react),
        Command::Run { file, project, runner, timeout_secs, related_tests, cache, changed_since } => {
            run(&file, &project, runner, timeout_secs, related_tests, cache, changed_since)
        }
        Command::Switch { file, project, runner, timeout_secs, related_tests, cache, changed_since } => {
            switch(&file, &project, runner, timeout_secs, related_tests, cache, changed_since)
        }
        Command::Mutate { config } => mutate(config),
        Command::ConfigSchema => {
            let schema = schema_for!(mtr_config::Config);
            println!("{}", serde_json::to_string_pretty(&schema).unwrap());
        }
    }
}

fn scan(pattern: &str, react: bool) {
    let packs = if react { vec!["react".to_string()] } else { Vec::new() };
    let mutators = MutatorRegistry::default().build(&packs, &[]).unwrap_or_else(|e| panic!("{e}"));
    let results = scan_files(pattern, &mutators).unwrap_or_else(|e| panic!("{e}"));
    println!("{}", serde_json::to_string_pretty(&results).unwrap());
}

fn run(
    file: &str,
    project: &str,
    runner: RunnerKind,
    timeout_secs: u64,
    related_tests: bool,
    cache_path: Option<String>,
    changed_since: Option<String>,
) {
    let file_path = PathBuf::from(file);
    let source_type = SourceType::from_path(&file_path).unwrap_or_default();
    let original = std::fs::read_to_string(&file_path)
        .unwrap_or_else(|e| panic!("failed to read {file}: {e}"));
    // Runners (and `git diff`) spawn with `current_dir(project)`, so a
    // relative `file_path` (relative to *our* cwd) must be made absolute
    // before being handed to them — otherwise it resolves against the wrong
    // directory.
    let absolute_file = std::fs::canonicalize(&file_path)
        .unwrap_or_else(|e| panic!("failed to resolve {file}: {e}"));
    let related_to_file = related_tests.then_some(absolute_file.as_path());

    let mutants = mtr_mutators::scan_source(&original, source_type);
    let mutants =
        scope_to_diff(mutants, project, &absolute_file, &original, changed_since.as_deref());
    let timeout = Duration::from_secs(timeout_secs);

    let mut cache = cache_path.as_deref().map(|p| MutantCache::load(Path::new(p)));
    let mut cache_hits = 0;

    let mut results = Vec::new();
    for mutant in mutants {
        let signature = mutant_signature(file, &mutant, &original);
        if let Some(status) = cache.as_ref().and_then(|c| c.get(&signature)) {
            cache_hits += 1;
            results.push(MutantResult { mutant, status });
            continue;
        }

        let mutated = apply_mutant(&original, &mutant);
        let status = match runner {
            RunnerKind::Jest => run_with_file_swapped(
                &file_path,
                &mutated,
                related_to_file,
                &JestRunner::new(project, timeout),
            ),
            RunnerKind::Vitest => run_with_file_swapped(
                &file_path,
                &mutated,
                related_to_file,
                &VitestRunner::new(project, timeout),
            ),
        }
        .unwrap_or_else(|e| panic!("failed to run mutant {}: {e}", mutant.id.0));
        if let Some(c) = cache.as_mut() {
            c.set(signature, status);
        }
        results.push(MutantResult { mutant, status });
    }

    save_cache_and_report(cache.as_ref(), cache_path.as_deref(), cache_hits, results.len());
    println!("{}", serde_json::to_string_pretty(&results).unwrap());
}

fn switch(
    file: &str,
    project: &str,
    runner: RunnerKind,
    timeout_secs: u64,
    related_tests: bool,
    cache_path: Option<String>,
    changed_since: Option<String>,
) {
    let mut cache = cache_path.as_deref().map(|p| MutantCache::load(Path::new(p)));
    let mutators = mtr_mutators::default_mutators();
    let (results, cache_hits) = switch_one_file(
        Path::new(file),
        project,
        runner.into(),
        Duration::from_secs(timeout_secs),
        related_tests,
        &mut cache,
        changed_since.as_deref(),
        &mutators,
    );

    save_cache_and_report(cache.as_ref(), cache_path.as_deref(), cache_hits, results.len());
    let mut results = results;
    results.sort_by_key(|r| r.mutant.id.0);
    println!("{}", serde_json::to_string_pretty(&results).unwrap());
}

fn mutate(config_path: Option<String>) {
    let default_path = "mutate.config.jsonc".to_string();
    let path = Path::new(config_path.as_deref().unwrap_or(&default_path));
    let config = if path.exists() {
        mtr_config::Config::load(path).unwrap_or_else(|e| panic!("{e}"))
    } else {
        mtr_config::Config::default()
    };

    let ignore_globs: Vec<glob::Pattern> = config
        .ignore_patterns
        .iter()
        .map(|p| glob::Pattern::new(p).unwrap_or_else(|e| panic!("invalid ignore pattern `{p}`: {e}")))
        .collect();

    let mut files: Vec<PathBuf> = Vec::new();
    for pattern in &config.mutate {
        for entry in glob::glob(pattern).unwrap_or_else(|e| panic!("invalid glob `{pattern}`: {e}")) {
            let path = entry.expect("failed to read glob entry");
            if !ignore_globs.iter().any(|g| g.matches_path(&path)) {
                files.push(path);
            }
        }
    }

    let timeout = Duration::from_secs(config.timeout_secs);
    let mut cache = config.cache.as_deref().map(|p| MutantCache::load(Path::new(p)));
    let mut cache_hits_total = 0;
    let mut per_file: Vec<FileMutantResults> = Vec::new();
    let mut reporters = ReporterRegistry::default()
        .build(&config.reporters)
        .unwrap_or_else(|e| panic!("{e}"));
    let mutators = MutatorRegistry::default()
        .build(&config.mutator_packs, &config.excluded_mutators)
        .unwrap_or_else(|e| panic!("{e}"));

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
        let file_key = file.display().to_string();
        for reporter in &mut reporters {
            for result in &results {
                reporter.on_mutant_tested(&file_key, result);
            }
            reporter.on_file_complete(&file_key, &results);
        }
        per_file.push(FileMutantResults { file: file_key, mutants: results });
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

    for reporter in &mut reporters {
        reporter.on_run_complete(&per_file, score);
        reporter.wrap_up();
    }

    eprintln!(
        "Mutation score: {score:.1}% ({killed} killed, {survived} survived, {other} other) across {} file(s)",
        per_file.len()
    );
    save_cache_and_report(cache.as_ref(), config.cache.as_deref(), cache_hits_total, total_mutants);

    if let Some(break_score) = config.thresholds.as_ref().and_then(|t| t.break_score) {
        if score < break_score {
            eprintln!("Mutation score {score:.1}% is below break threshold {break_score}%");
            std::process::exit(1);
        }
    }
}

fn save_cache_and_report(
    cache: Option<&MutantCache>,
    cache_path: Option<&str>,
    hits: usize,
    total: usize,
) {
    let (Some(cache), Some(path)) = (cache, cache_path) else { return };
    cache
        .save(Path::new(path))
        .unwrap_or_else(|e| panic!("failed to save cache to {path}: {e}"));
    eprintln!("cache: {hits} hit(s), {} tested", total - hits);
}
