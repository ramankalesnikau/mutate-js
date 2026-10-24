use std::path::Path;

use mtr_types::{MutantId, MutantStatus};

/// `active_mutant`: `None` runs the suite unmutated (or, against an
/// instrumented file, with every mutant switched off); `Some(id)` selects
/// which mutant is switched on.
///
/// `related_to_file`: `None` runs the whole suite; `Some(path)` asks the
/// runner's own dependency resolver to run only tests related to that file
/// (Jest's `--findRelatedTests`, Vitest's `related` subcommand).
#[derive(Clone, Copy)]
pub struct RunOptions<'a> {
    pub active_mutant: Option<MutantId>,
    pub related_to_file: Option<&'a Path>,
}

pub trait TestRunner {
    fn run(&self, opts: RunOptions) -> MutantStatus;
}

/// A type-erased [`TestRunner`] — the shape a `TestRunnerRegistry` (in
/// `mtr-core`) hands back as `Box<DynTestRunner>`, so callers that pick a
/// runner by config string rather than a concrete generic type still work
/// with [`run_with_file_swapped`]/[`run_with_instrumented_file`] (both
/// `?Sized` over their `runner` parameter for exactly this reason).
pub type DynTestRunner = dyn TestRunner + std::panic::RefUnwindSafe;

/// Overwrites `file`, runs `runner` once, then restores the original content —
/// even if the run panics. For a plain file swap, one mutant per file version.
pub fn run_with_file_swapped<R: TestRunner + std::panic::RefUnwindSafe + ?Sized>(
    file: &Path,
    mutated_source: &str,
    related_to_file: Option<&Path>,
    runner: &R,
) -> std::io::Result<MutantStatus> {
    let original = std::fs::read_to_string(file)?;
    std::fs::write(file, mutated_source)?;

    let opts = RunOptions { active_mutant: None, related_to_file };
    let result = std::panic::catch_unwind(|| runner.run(opts));

    std::fs::write(file, &original)?;

    match result {
        Ok(status) => Ok(status),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Writes `instrumented_source` (all mutants embedded behind a runtime
/// switch) once, runs `runner` once per id in `mutant_ids` with that mutant
/// selected, then restores the original content — even if a run panics.
pub fn run_with_instrumented_file<R: TestRunner + std::panic::RefUnwindSafe + ?Sized>(
    file: &Path,
    instrumented_source: &str,
    mutant_ids: &[MutantId],
    related_to_file: Option<&Path>,
    runner: &R,
) -> std::io::Result<Vec<(MutantId, MutantStatus)>> {
    let original = std::fs::read_to_string(file)?;
    std::fs::write(file, instrumented_source)?;

    let result = std::panic::catch_unwind(|| {
        mutant_ids
            .iter()
            .map(|&id| {
                let opts = RunOptions { active_mutant: Some(id), related_to_file };
                (id, runner.run(opts))
            })
            .collect::<Vec<_>>()
    });

    std::fs::write(file, &original)?;

    match result {
        Ok(results) => Ok(results),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
