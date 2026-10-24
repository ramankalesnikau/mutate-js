use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use mtr_core::{switch_one_file, switch_one_file_with_coverage, MutatorRegistry, TestRunnerRegistry};
use mtr_types::MutantStatus;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// Both `switch_one_file` and `switch_one_file_with_coverage` temporarily
// overwrite the target file on disk (to embed the mutation-switching
// instrumentation) before restoring it — concurrent tests against the same
// fixture file would otherwise race.
static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

fn assert_coverage_matches_related(project: &Path, file: &Path, runner_name: &str) {
    let mutators = MutatorRegistry::default().build(&[], &[]).unwrap();
    let timeout = Duration::from_secs(30);

    let related_runner = TestRunnerRegistry::default()
        .build(runner_name, &project.display().to_string(), timeout)
        .unwrap();
    let (mut related_results, _, _) = switch_one_file(
        file,
        &project.display().to_string(),
        related_runner.as_ref(),
        true,
        &mut None,
        None,
        &mutators,
    );
    related_results.sort_by_key(|r| r.mutant.id.0);

    let coverage_runner = TestRunnerRegistry::default()
        .build(runner_name, &project.display().to_string(), timeout)
        .unwrap();
    let (mut coverage_results, _, _) = switch_one_file_with_coverage(
        file,
        &project.display().to_string(),
        coverage_runner.as_ref(),
        &mut None,
        None,
        &mutators,
    )
    .unwrap();
    coverage_results.sort_by_key(|r| r.mutant.id.0);

    assert_eq!(
        related_results.len(),
        coverage_results.len(),
        "coverage and related modes should discover the same mutants"
    );
    for (related, coverage) in related_results.iter().zip(&coverage_results) {
        assert_eq!(related.mutant.id, coverage.mutant.id);
        // `coverage` mode narrows test selection further than `related` (per
        // statement, not per file), so it can only ever be as precise or
        // more — never report a mutant killed that `related` reported
        // survived. Both covering the file's one test apiece, they should
        // agree exactly on this fixture.
        assert_eq!(
            related.status, coverage.status,
            "mutant {} disagreed: related={:?} coverage={:?}",
            related.mutant.id.0, related.status, coverage.status
        );
    }
}

#[test]
fn jest_coverage_matches_related_on_math_js() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-jest-demo");
    let file = project.join("src/math.js");
    assert_coverage_matches_related(&project, &file, "jest");
}

#[test]
fn vitest_coverage_matches_related_on_math_js() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-vitest-demo");
    let file = project.join("src/math.js");
    assert_coverage_matches_related(&project, &file, "vitest");
}

#[test]
fn jest_coverage_reports_no_coverage_for_an_uncalled_function() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-jest-demo");
    let file = project.join("src/math.js");
    let mutators = MutatorRegistry::default().build(&[], &[]).unwrap();
    let runner = TestRunnerRegistry::default()
        .build("jest", &project.display().to_string(), Duration::from_secs(30))
        .unwrap();

    let (results, _, _) = switch_one_file_with_coverage(
        &file,
        &project.display().to_string(),
        runner.as_ref(),
        &mut None,
        None,
        &mutators,
    )
    .unwrap();

    // `defaultActive` is called by a test in the fixture, so every mutant in
    // this file should resolve to a real status, never NoCoverage — this
    // guards against the coverage plumbing trivially marking everything
    // uncovered by a bug (e.g. a path mismatch) rather than actually
    // resolving coverage correctly.
    assert!(
        results.iter().all(|r| r.status != MutantStatus::NoCoverage),
        "every mutant in math.js is covered by a test in this fixture: {results:#?}"
    );
}
