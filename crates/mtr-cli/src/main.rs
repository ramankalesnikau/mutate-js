use clap::{Parser as ClapParser, Subcommand};
use mtr_types::Mutant;
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
