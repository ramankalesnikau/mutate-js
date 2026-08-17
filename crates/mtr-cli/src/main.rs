use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser as ClapParser, Subcommand, ValueEnum};
use mtr_instrument::switch_instrument;
use mtr_mutators::apply_mutant;
use mtr_runner_jest::JestRunner;
use mtr_runner_vitest::VitestRunner;
use mtr_test_runner_api::{run_with_file_swapped, run_with_instrumented_file};
use mtr_types::{Mutant, MutantId, MutantResult, MutantStatus};
use oxc_span::SourceType;
use serde::Serialize;

#[derive(ClapParser)]
#[command(name = "mutate-js")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Discover mutants in files matching a glob, without running any tests.
    Scan { glob: String },
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
    },
}

#[derive(Clone, ValueEnum)]
enum RunnerKind {
    Jest,
    Vitest,
}

#[derive(Serialize)]
struct FileMutants {
    file: String,
    mutants: Vec<Mutant>,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan { glob } => scan(&glob),
        Command::Run { file, project, runner, timeout_secs, related_tests } => {
            run(&file, &project, runner, timeout_secs, related_tests)
        }
        Command::Switch { file, project, runner, timeout_secs, related_tests } => {
            switch(&file, &project, runner, timeout_secs, related_tests)
        }
    }
}

fn scan(pattern: &str) {
    let paths = glob::glob(pattern).unwrap_or_else(|e| panic!("invalid glob `{pattern}`: {e}"));

    let mut results = Vec::new();
    for entry in paths {
        let path = entry.expect("failed to read glob entry");
        let source_type = SourceType::from_path(&path).unwrap_or_default();
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let mutants = mtr_mutators::scan_source(&source, source_type);
        results.push(FileMutants { file: path.display().to_string(), mutants });
    }

    println!("{}", serde_json::to_string_pretty(&results).unwrap());
}

fn run(file: &str, project: &str, runner: RunnerKind, timeout_secs: u64, related_tests: bool) {
    let file_path = PathBuf::from(file);
    let source_type = SourceType::from_path(&file_path).unwrap_or_default();
    let original = std::fs::read_to_string(&file_path)
        .unwrap_or_else(|e| panic!("failed to read {file}: {e}"));
    let mutants = mtr_mutators::scan_source(&original, source_type);
    let timeout = Duration::from_secs(timeout_secs);
    // Runners spawn with `current_dir(project)`, so a relative `file_path`
    // (relative to *our* cwd) must be made absolute before being handed to
    // them — otherwise it resolves against the wrong directory.
    let absolute_file = std::fs::canonicalize(&file_path)
        .unwrap_or_else(|e| panic!("failed to resolve {file}: {e}"));
    let related_to_file = related_tests.then_some(absolute_file.as_path());

    let mut results = Vec::new();
    for mutant in mutants {
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
        results.push(MutantResult { mutant, status });
    }

    println!("{}", serde_json::to_string_pretty(&results).unwrap());
}

fn switch(file: &str, project: &str, runner: RunnerKind, timeout_secs: u64, related_tests: bool) {
    let file_path = PathBuf::from(file);
    let source_type = SourceType::from_path(&file_path).unwrap_or_default();
    let original = std::fs::read_to_string(&file_path)
        .unwrap_or_else(|e| panic!("failed to read {file}: {e}"));
    let mutants = mtr_mutators::scan_source(&original, source_type);
    let instrumented = switch_instrument(&original, &mutants);
    let ids: Vec<MutantId> = mutants.iter().map(|m| m.id).collect();
    let timeout = Duration::from_secs(timeout_secs);
    let absolute_file = std::fs::canonicalize(&file_path)
        .unwrap_or_else(|e| panic!("failed to resolve {file}: {e}"));
    let related_to_file = related_tests.then_some(absolute_file.as_path());

    let statuses: Vec<(MutantId, MutantStatus)> = match runner {
        RunnerKind::Jest => run_with_instrumented_file(
            &file_path,
            &instrumented,
            &ids,
            related_to_file,
            &JestRunner::new(project, timeout),
        ),
        RunnerKind::Vitest => run_with_instrumented_file(
            &file_path,
            &instrumented,
            &ids,
            related_to_file,
            &VitestRunner::new(project, timeout),
        ),
    }
    .unwrap_or_else(|e| panic!("failed to run instrumented file: {e}"));

    let results: Vec<MutantResult> = mutants
        .into_iter()
        .zip(statuses.into_iter().map(|(_, status)| status))
        .map(|(mutant, status)| MutantResult { mutant, status })
        .collect();

    println!("{}", serde_json::to_string_pretty(&results).unwrap());
}
