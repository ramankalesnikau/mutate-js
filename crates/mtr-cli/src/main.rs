use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser as ClapParser, Subcommand, ValueEnum};
use mtr_mutators::apply_mutant;
use mtr_runner_jest::JestRunner;
use mtr_runner_vitest::VitestRunner;
use mtr_test_runner_api::run_with_file_swapped;
use mtr_types::{Mutant, MutantResult};
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
        Command::Run { file, project, runner, timeout_secs } => {
            run(&file, &project, runner, timeout_secs)
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

fn run(file: &str, project: &str, runner: RunnerKind, timeout_secs: u64) {
    let file_path = PathBuf::from(file);
    let source_type = SourceType::from_path(&file_path).unwrap_or_default();
    let original = std::fs::read_to_string(&file_path)
        .unwrap_or_else(|e| panic!("failed to read {file}: {e}"));
    let mutants = mtr_mutators::scan_source(&original, source_type);
    let timeout = Duration::from_secs(timeout_secs);

    let mut results = Vec::new();
    for mutant in mutants {
        let mutated = apply_mutant(&original, &mutant);
        let status = match runner {
            RunnerKind::Jest => {
                run_with_file_swapped(&file_path, &mutated, &JestRunner::new(project, timeout))
            }
            RunnerKind::Vitest => {
                run_with_file_swapped(&file_path, &mutated, &VitestRunner::new(project, timeout))
            }
        }
        .unwrap_or_else(|e| panic!("failed to run mutant {}: {e}", mutant.id.0));
        results.push(MutantResult { mutant, status });
    }

    println!("{}", serde_json::to_string_pretty(&results).unwrap());
}
