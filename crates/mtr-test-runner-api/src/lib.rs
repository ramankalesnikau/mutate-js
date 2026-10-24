use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use mtr_types::{MutantId, MutantStatus};
use serde::Deserialize;

/// `active_mutant`: `None` runs the suite unmutated (or, against an
/// instrumented file, with every mutant switched off); `Some(id)` selects
/// which mutant is switched on.
///
/// `related_to_file`: `None` runs the whole suite; `Some(path)` asks the
/// runner's own dependency resolver to run only tests related to that file
/// (Jest's `--findRelatedTests`, Vitest's `related` subcommand). Ignored
/// when `only_tests` is set.
///
/// `only_tests`: exact test ids (full describe+test name) to run, already
/// resolved from coverage analysis — takes precedence over
/// `related_to_file` when set.
#[derive(Clone, Copy)]
pub struct RunOptions<'a> {
    pub active_mutant: Option<MutantId>,
    pub related_to_file: Option<&'a Path>,
    pub only_tests: Option<&'a [String]>,
}

/// A single statement's line range, as reported by a runner's istanbul-style
/// coverage (`statementMap`). Columns are dropped after parsing — matching
/// the precedent set by `mtr-cache`'s git-diff scoping, which also compares
/// line ranges only, not columns, for the same good-enough-and-robust reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoveredStatement {
    pub start_line: u32,
    pub end_line: u32,
}

/// One test's coverage contribution: which statements, in which files, ran
/// while it executed (captured as a `beforeEach`/`afterEach` diff of the
/// runner's global coverage counters, not a full-suite aggregate).
#[derive(Debug, Clone, Default)]
pub struct TestCoverage {
    pub test_id: String,
    pub touched: HashMap<PathBuf, Vec<CoveredStatement>>,
}

/// Every test's individual coverage contribution from one suite run.
#[derive(Debug, Clone, Default)]
pub struct SuiteCoverage {
    pub tests: Vec<TestCoverage>,
}

pub trait TestRunner {
    fn run(&self, opts: RunOptions) -> MutantStatus;

    /// Runs the whole suite (or, when `related_to_file` is set, the
    /// runner's own related-file prefilter) once, collecting which
    /// statements each individual test touched. Mutants outside every
    /// test's touched statements can then skip test execution entirely
    /// (`MutantStatus::NoCoverage`) instead of running the suite against
    /// them and finding nothing.
    ///
    /// Defaults to an error — same all-default-no-op shape as `Mutator` and
    /// `Reporter` — so a runner that hasn't implemented this gives a clear
    /// message instead of a silent, wrong fallback.
    fn collect_coverage(&self, _related_to_file: Option<&Path>) -> Result<SuiteCoverage, String> {
        Err("this test runner doesn't support coverage-based test filtering".to_string())
    }
}

/// A type-erased [`TestRunner`] — the shape a `TestRunnerRegistry` (in
/// `mtr-core`) hands back as `Box<DynTestRunner>`, so callers that pick a
/// runner by config string rather than a concrete generic type still work
/// with [`run_with_file_swapped`]/[`run_with_instrumented_file`].
pub type DynTestRunner = dyn TestRunner + std::panic::RefUnwindSafe + Send + Sync;

/// Escapes regex metacharacters in a test name for safe use inside a `-t`
/// alternation — test names are free text (can contain `.`, `(`, `[`, etc.),
/// not pre-escaped regex fragments.
pub fn escape_test_name_for_regex(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for c in name.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

#[derive(Deserialize)]
struct CoverageLocation {
    line: u32,
}

#[derive(Deserialize)]
struct CoverageRange {
    start: CoverageLocation,
    end: CoverageLocation,
}

#[derive(Deserialize)]
struct CoverageLine {
    test: String,
    coverage: HashMap<PathBuf, Vec<CoverageRange>>,
}

/// Parses `MTR_COVERAGE {...}` lines (one per test) from a runner's stdout
/// into a [`SuiteCoverage`] — the wire shape both the Jest and Vitest
/// coverage-setup scripts emit, by design, so this parser is shared rather
/// than duplicated per runner. Lines that aren't valid `MTR_COVERAGE`
/// entries (test output, summaries, etc.) are silently skipped.
pub fn parse_coverage_stream(stdout: &str) -> SuiteCoverage {
    let tests = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("MTR_COVERAGE "))
        .filter_map(|json| serde_json::from_str::<CoverageLine>(json).ok())
        .map(|entry| TestCoverage {
            test_id: entry.test,
            touched: entry
                .coverage
                .into_iter()
                .map(|(file, ranges)| {
                    let statements = ranges
                        .into_iter()
                        .map(|r| CoveredStatement { start_line: r.start.line, end_line: r.end.line })
                        .collect();
                    (file, statements)
                })
                .collect(),
        })
        .collect();
    SuiteCoverage { tests }
}

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

    let opts = RunOptions { active_mutant: None, related_to_file, only_tests: None };
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
                let opts = RunOptions { active_mutant: Some(id), related_to_file, only_tests: None };
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

/// Like [`run_with_instrumented_file`], but each mutant id in `targeted`
/// runs against only its own pre-resolved covering tests (from coverage
/// analysis) instead of the whole suite or a related-file prefilter.
pub fn run_with_instrumented_file_targeted<R: TestRunner + std::panic::RefUnwindSafe + ?Sized>(
    file: &Path,
    instrumented_source: &str,
    targeted: &[(MutantId, Vec<String>)],
    runner: &R,
) -> std::io::Result<Vec<(MutantId, MutantStatus)>> {
    let original = std::fs::read_to_string(file)?;
    std::fs::write(file, instrumented_source)?;

    let result = std::panic::catch_unwind(|| {
        targeted
            .iter()
            .map(|(id, tests)| {
                let opts = RunOptions {
                    active_mutant: Some(*id),
                    related_to_file: None,
                    only_tests: Some(tests.as_slice()),
                };
                (*id, runner.run(opts))
            })
            .collect::<Vec<_>>()
    });

    std::fs::write(file, &original)?;

    match result {
        Ok(results) => Ok(results),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Runs `count` jobs, at most `concurrency` at a time, returning
/// `(index, status)` in index order. Workers pull indices from a shared
/// counter and buffer their own results — no mutex, so no poisoning; the
/// parent sorts by index, keeping output deterministic.
fn run_jobs<'a, R, F>(
    count: usize,
    concurrency: usize,
    runner: &R,
    make_opts: F,
) -> Vec<(usize, MutantStatus)>
where
    R: TestRunner + std::panic::RefUnwindSafe + Sync + ?Sized,
    F: Fn(usize) -> RunOptions<'a> + Sync,
{
    let concurrency = concurrency.max(1).min(count.max(1));
    if concurrency <= 1 || count <= 1 {
        return (0..count).map(|index| (index, runner.run(make_opts(index)))).collect();
    }

    let next = AtomicUsize::new(0);
    let mut results: Vec<(usize, MutantStatus)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..concurrency)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        if index >= count {
                            break;
                        }
                        local.push((index, runner.run(make_opts(index))));
                    }
                    local
                })
            })
            .collect();

        let mut all = Vec::with_capacity(count);
        for handle in handles {
            all.extend(handle.join().expect("mutant worker thread panicked"));
        }
        all
    });
    results.sort_by_key(|(index, _)| *index);
    results
}

/// Installs `instrumented_source`, runs `body`, then restores the original —
/// on panic too. `body`'s value is discarded if it panicked, matching the
/// sequential helpers' behavior.
fn with_instrumented_file<T>(
    file: &Path,
    instrumented_source: &str,
    body: impl FnOnce() -> T,
) -> std::io::Result<T> {
    let original = std::fs::read_to_string(file)?;
    std::fs::write(file, instrumented_source)?;

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));

    let restore = std::fs::write(file, &original);
    match (result, restore) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(error),
        (Err(payload), _) => std::panic::resume_unwind(payload),
    }
}

/// Like [`run_with_instrumented_file`], but runs the mutants concurrently.
/// Safe because the instrumented file is written once and only read by the
/// child processes, each of which receives its own `MTR_ACTIVE_MUTANT` via
/// the environment. `concurrency <= 1` delegates to the sequential path.
pub fn run_with_instrumented_file_parallel<'a, R>(
    file: &Path,
    instrumented_source: &str,
    mutant_ids: &'a [MutantId],
    related_to_file: Option<&'a Path>,
    runner: &R,
    concurrency: usize,
) -> std::io::Result<Vec<(MutantId, MutantStatus)>>
where
    R: TestRunner + std::panic::RefUnwindSafe + Sync + ?Sized,
{
    if concurrency <= 1 || mutant_ids.len() <= 1 {
        return run_with_instrumented_file(
            file,
            instrumented_source,
            mutant_ids,
            related_to_file,
            runner,
        );
    }

    with_instrumented_file(file, instrumented_source, || {
        run_jobs(mutant_ids.len(), concurrency, runner, |index| RunOptions {
            active_mutant: Some(mutant_ids[index]),
            related_to_file,
            only_tests: None,
        })
        .into_iter()
        .map(|(index, status)| (mutant_ids[index], status))
        .collect()
    })
}

/// Like [`run_with_instrumented_file_targeted`], but runs each mutant's
/// pre-resolved covering tests concurrently.
pub fn run_with_instrumented_file_targeted_parallel<R>(
    file: &Path,
    instrumented_source: &str,
    targeted: &[(MutantId, Vec<String>)],
    runner: &R,
    concurrency: usize,
) -> std::io::Result<Vec<(MutantId, MutantStatus)>>
where
    R: TestRunner + std::panic::RefUnwindSafe + Sync + ?Sized,
{
    if concurrency <= 1 || targeted.len() <= 1 {
        return run_with_instrumented_file_targeted(file, instrumented_source, targeted, runner);
    }

    with_instrumented_file(file, instrumented_source, || {
        run_jobs(targeted.len(), concurrency, runner, |index| {
            let (id, tests) = &targeted[index];
            RunOptions {
                active_mutant: Some(*id),
                related_to_file: None,
                only_tests: Some(tests.as_slice()),
            }
        })
        .into_iter()
        .map(|(index, status)| (targeted[index].0, status))
        .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct SlowRunner {
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        seen: Mutex<Vec<u32>>,
    }

    impl SlowRunner {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                in_flight: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    impl TestRunner for SlowRunner {
        fn run(&self, opts: RunOptions) -> MutantStatus {
            let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(current, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(15));
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            let id = opts.active_mutant.expect("test runner expects an active mutant").0;
            self.seen.lock().unwrap().push(id);
            MutantStatus::Killed
        }
    }

    #[test]
    fn run_jobs_preserves_index_order_and_actually_runs_in_parallel() {
        let runner = SlowRunner::new();
        let results = run_jobs(8, 4, runner.as_ref(), |index| RunOptions {
            active_mutant: Some(MutantId(index as u32)),
            related_to_file: None,
            only_tests: None,
        });

        assert_eq!(results.len(), 8);
        assert_eq!(results.iter().map(|(index, _)| *index).collect::<Vec<_>>(), (0..8).collect::<Vec<_>>());
        assert!(results.iter().all(|(_, status)| *status == MutantStatus::Killed));
        assert_eq!(runner.seen.lock().unwrap().len(), 8, "every job should have run");
        assert!(
            runner.max_in_flight.load(Ordering::SeqCst) > 1,
            "expected concurrent execution, saw max {} in flight",
            runner.max_in_flight.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn run_jobs_with_concurrency_one_stays_sequential() {
        let runner = SlowRunner::new();
        let results = run_jobs(3, 1, runner.as_ref(), |index| RunOptions {
            active_mutant: Some(MutantId(index as u32)),
            related_to_file: None,
            only_tests: None,
        });
        assert_eq!(results.len(), 3);
        assert_eq!(runner.max_in_flight.load(Ordering::SeqCst), 1);
    }


    #[test]
    fn escape_test_name_for_regex_escapes_metacharacters() {
        assert_eq!(escape_test_name_for_regex("a.b"), "a\\.b");
        assert_eq!(escape_test_name_for_regex("foo (bar)"), "foo \\(bar\\)");
        assert_eq!(escape_test_name_for_regex("a|b"), "a\\|b");
        assert_eq!(escape_test_name_for_regex("plain text"), "plain text");
    }

    #[test]
    fn parse_coverage_stream_parses_one_line_per_test() {
        let stdout = "PASS src/math.test.js\n\
             MTR_COVERAGE {\"test\":\"sum adds\",\"coverage\":{\"/p/math.js\":[{\"start\":{\"line\":2,\"column\":2},\"end\":{\"line\":2,\"column\":15}}]}}\n\
             MTR_COVERAGE {\"test\":\"isEqual\",\"coverage\":{\"/p/math.js\":[{\"start\":{\"line\":6,\"column\":2},\"end\":{\"line\":6,\"column\":17}}]}}\n\
             Test Suites: 1 passed, 1 total\n";

        let coverage = parse_coverage_stream(stdout);
        assert_eq!(coverage.tests.len(), 2);
        assert_eq!(coverage.tests[0].test_id, "sum adds");
        let touched = &coverage.tests[0].touched[&PathBuf::from("/p/math.js")];
        assert_eq!(touched, &vec![CoveredStatement { start_line: 2, end_line: 2 }]);
    }

    #[test]
    fn parse_coverage_stream_ignores_unrelated_lines() {
        let stdout = "random output\nnot json at all\n";
        let coverage = parse_coverage_stream(stdout);
        assert!(coverage.tests.is_empty());
    }
}
