use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use mtr_core::{
    switch_one_file, switch_one_file_with_coverage, MutatorRegistry, TestRunnerRegistry,
};
use mtr_types::MutantResult;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// These tests spawn real Jest processes against shared fixture files; keep
// them from competing with each other inside one test binary.
static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

fn runner(project: &Path) -> Box<mtr_test_runner_api::DynTestRunner> {
    TestRunnerRegistry::default()
        .build("jest", &project.display().to_string(), Duration::from_secs(30))
        .unwrap()
}

fn sorted(mut results: Vec<MutantResult>) -> Vec<(u32, String, mtr_types::MutantStatus)> {
    results.sort_by_key(|r| r.mutant.id.0);
    results.into_iter().map(|r| (r.mutant.id.0, r.mutant.operator, r.status)).collect()
}

#[test]
fn parallel_and_sequential_agree_in_related_mode() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-jest-demo");
    let file = project.join("src/math.js");
    let mutators = MutatorRegistry::default().build(&[], &[]).unwrap();

    let sequential_runner = runner(&project);
    let (sequential, _, _) = switch_one_file(
        &file,
        &project.display().to_string(),
        sequential_runner.as_ref(),
        true,
        1,
        &mut None,
        None,
        &mutators,
    );

    let parallel_runner = runner(&project);
    let (parallel, _, _) = switch_one_file(
        &file,
        &project.display().to_string(),
        parallel_runner.as_ref(),
        true,
        4,
        &mut None,
        None,
        &mutators,
    );

    assert_eq!(sorted(sequential), sorted(parallel));
}

#[test]
fn parallel_and_sequential_agree_in_coverage_mode() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-jest-demo");
    let file = project.join("src/math.js");
    let mutators = MutatorRegistry::default().build(&[], &[]).unwrap();

    let sequential_runner = runner(&project);
    let (sequential, _, _) = switch_one_file_with_coverage(
        &file,
        &project.display().to_string(),
        sequential_runner.as_ref(),
        1,
        &mut None,
        None,
        &mutators,
    )
    .unwrap();

    let parallel_runner = runner(&project);
    let (parallel, _, _) = switch_one_file_with_coverage(
        &file,
        &project.display().to_string(),
        parallel_runner.as_ref(),
        4,
        &mut None,
        None,
        &mutators,
    )
    .unwrap();

    assert_eq!(sorted(sequential), sorted(parallel));
}
