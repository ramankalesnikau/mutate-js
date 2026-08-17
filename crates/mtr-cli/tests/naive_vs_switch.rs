use std::path::{Path, PathBuf};
use std::time::Duration;

use mtr_instrument::switch_instrument;
use mtr_mutators::apply_mutant;
use mtr_runner_jest::JestRunner;
use mtr_runner_vitest::VitestRunner;
use mtr_test_runner_api::{run_with_file_swapped, run_with_instrumented_file, TestRunner};
use mtr_types::MutantStatus;
use oxc_span::SourceType;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn assert_naive_matches_switch<R: TestRunner + std::panic::RefUnwindSafe>(
    file: &Path,
    make_runner: impl Fn() -> R,
) {
    let source_type = SourceType::from_path(file).unwrap();
    let original = std::fs::read_to_string(file).unwrap();
    let mutants = mtr_mutators::scan_source(&original, source_type);
    assert!(!mutants.is_empty(), "fixture produced no mutants: {}", file.display());

    let mut naive_statuses = Vec::new();
    for mutant in &mutants {
        let mutated = apply_mutant(&original, mutant);
        naive_statuses.push(run_with_file_swapped(file, &mutated, &make_runner()).unwrap());
    }

    let instrumented = switch_instrument(&original, &mutants);
    let ids: Vec<_> = mutants.iter().map(|m| m.id).collect();
    let switch_statuses: Vec<MutantStatus> =
        run_with_instrumented_file(file, &instrumented, &ids, &make_runner())
            .unwrap()
            .into_iter()
            .map(|(_, status)| status)
            .collect();

    assert_eq!(
        naive_statuses,
        switch_statuses,
        "naive (per-mutant rebuild) and switch (single instrumented build) results diverge for {}",
        file.display()
    );
}

#[test]
fn jest_naive_and_switch_agree() {
    let project = workspace_root().join("fixtures/naive-jest-demo");
    let file = project.join("src/math.js");
    assert_naive_matches_switch(&file, || JestRunner::new(&project, Duration::from_secs(30)));
}

#[test]
fn vitest_naive_and_switch_agree() {
    let project = workspace_root().join("fixtures/naive-vitest-demo");
    let file = project.join("src/math.js");
    assert_naive_matches_switch(&file, || VitestRunner::new(&project, Duration::from_secs(30)));
}
