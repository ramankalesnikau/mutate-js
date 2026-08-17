use std::path::Path;

use mtr_types::{MutantId, MutantStatus};

/// Deliberately minimal for now: one whole-suite run, no coverage-aware
/// dry-run/mutant-run split yet — that split only earns its place once
/// coverage-based test filtering exists (a later stage).
///
/// `active_mutant`: `None` runs the suite unmutated (or, against an
/// instrumented file, with every mutant switched off); `Some(id)` selects
/// which mutant is switched on.
pub trait TestRunner {
    fn run(&self, active_mutant: Option<MutantId>) -> MutantStatus;
}

/// Overwrites `file`, runs `runner` once, then restores the original content —
/// even if the run panics. For a plain file swap, one mutant per file version.
pub fn run_with_file_swapped(
    file: &Path,
    mutated_source: &str,
    runner: &(impl TestRunner + std::panic::RefUnwindSafe),
) -> std::io::Result<MutantStatus> {
    let original = std::fs::read_to_string(file)?;
    std::fs::write(file, mutated_source)?;

    let result = std::panic::catch_unwind(|| runner.run(None));

    std::fs::write(file, &original)?;

    match result {
        Ok(status) => Ok(status),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Writes `instrumented_source` (all mutants embedded behind a runtime
/// switch) once, runs `runner` once per id in `mutant_ids` with that mutant
/// selected, then restores the original content — even if a run panics.
pub fn run_with_instrumented_file(
    file: &Path,
    instrumented_source: &str,
    mutant_ids: &[MutantId],
    runner: &(impl TestRunner + std::panic::RefUnwindSafe),
) -> std::io::Result<Vec<(MutantId, MutantStatus)>> {
    let original = std::fs::read_to_string(file)?;
    std::fs::write(file, instrumented_source)?;

    let result = std::panic::catch_unwind(|| {
        mutant_ids.iter().map(|&id| (id, runner.run(Some(id)))).collect::<Vec<_>>()
    });

    std::fs::write(file, &original)?;

    match result {
        Ok(results) => Ok(results),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
