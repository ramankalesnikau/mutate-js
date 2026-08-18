use mtr_reporter_api::{FileMutantResults, Reporter};
use mtr_types::{MutantResult, MutantStatus};

/// Accumulates every file's results and prints one JSON blob at the end —
/// today's original CLI behavior, refactored into the `Reporter` trait
/// rather than replaced.
#[derive(Default)]
pub struct JsonReporter {
    files: Vec<FileMutantResults>,
}

impl Reporter for JsonReporter {
    fn on_file_complete(&mut self, file: &str, results: &[MutantResult]) {
        self.files.push(FileMutantResults { file: file.to_string(), mutants: results.to_vec() });
    }

    fn wrap_up(&mut self) {
        println!("{}", serde_json::to_string_pretty(&self.files).unwrap());
    }
}

/// Human-readable: one line per mutant as its file completes, then a final
/// score summary — proves the trait handles incremental-as-it-happens
/// output, not just JsonReporter's batch-at-the-end style.
#[derive(Default)]
pub struct TextReporter;

impl Reporter for TextReporter {
    fn on_mutant_tested(&mut self, file: &str, result: &MutantResult) {
        let label = match result.status {
            MutantStatus::Killed => "killed",
            MutantStatus::Survived => "SURVIVED",
            MutantStatus::Timeout => "timeout",
            MutantStatus::Error => "error",
        };
        println!(
            "{label:<9} {file}  {} `{}` -> `{}`",
            result.mutant.operator, result.mutant.original, result.mutant.replacement
        );
    }

    fn on_run_complete(&mut self, files: &[FileMutantResults], score: f64) {
        let total: usize = files.iter().map(|f| f.mutants.len()).sum();
        println!(
            "\n{score:.1}% mutation score ({total} mutant(s) across {} file(s))",
            files.len()
        );
    }
}
