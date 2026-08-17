use std::path::Path;

use mtr_types::MutantStatus;

/// Deliberately minimal for now: one whole-suite run, no coverage-aware
/// dry-run/mutant-run split yet — that split only earns its place once
/// coverage-based test filtering exists (a later stage).
pub trait TestRunner {
    fn run(&self) -> MutantStatus;
}

/// Overwrites `file`, runs `runner`, then restores the original content —
/// even if the run panics. Shared by every runner adapter; not runner-specific.
pub fn run_with_file_swapped(
    file: &Path,
    mutated_source: &str,
    runner: &(impl TestRunner + std::panic::RefUnwindSafe),
) -> std::io::Result<MutantStatus> {
    let original = std::fs::read_to_string(file)?;
    std::fs::write(file, mutated_source)?;

    let result = std::panic::catch_unwind(|| runner.run());

    std::fs::write(file, &original)?;

    match result {
        Ok(status) => Ok(status),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
