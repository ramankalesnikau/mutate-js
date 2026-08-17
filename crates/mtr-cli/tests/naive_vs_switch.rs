use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use mtr_cache::{mutant_signature, MutantCache};
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

// cargo test runs tests in parallel by default, but every test here mutates
// shared fixture files on disk — without this, two tests racing on the same
// file corrupt each other's swaps.
static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

fn assert_naive_matches_switch<R: TestRunner + std::panic::RefUnwindSafe>(
    file: &Path,
    related_to_file: Option<&Path>,
    make_runner: impl Fn() -> R,
) {
    let source_type = SourceType::from_path(file).unwrap();
    let original = std::fs::read_to_string(file).unwrap();
    let mutants = mtr_mutators::scan_source(&original, source_type);
    assert!(!mutants.is_empty(), "fixture produced no mutants: {}", file.display());

    let mut naive_statuses = Vec::new();
    for mutant in &mutants {
        let mutated = apply_mutant(&original, mutant);
        naive_statuses.push(
            run_with_file_swapped(file, &mutated, related_to_file, &make_runner()).unwrap(),
        );
    }

    let instrumented = switch_instrument(&original, &mutants);
    let ids: Vec<_> = mutants.iter().map(|m| m.id).collect();
    let switch_statuses: Vec<MutantStatus> =
        run_with_instrumented_file(file, &instrumented, &ids, related_to_file, &make_runner())
            .unwrap()
            .into_iter()
            .map(|(_, status)| status)
            .collect();

    assert_eq!(
        naive_statuses,
        switch_statuses,
        "naive and switch results diverge for {} (related_to_file: {:?})",
        file.display(),
        related_to_file
    );
}

/// Proves `related_to_file` is actually doing something, not silently
/// ignored: the same mutant should be Killed when we ask for tests related
/// to its own file, and Survived when we (wrongly) ask only for tests
/// related to a completely unrelated file.
fn assert_filtering_actually_filters<R: TestRunner + std::panic::RefUnwindSafe>(
    mutated_file: &Path,
    unrelated_file: &Path,
    make_runner: impl Fn() -> R,
) {
    let source_type = SourceType::from_path(mutated_file).unwrap();
    let original = std::fs::read_to_string(mutated_file).unwrap();
    let mutants = mtr_mutators::scan_source(&original, source_type);
    let arithmetic_mutant = mutants
        .iter()
        .find(|m| m.operator == "ArithmeticOperator")
        .expect("fixture should contain an arithmetic mutant");
    let mutated = apply_mutant(&original, arithmetic_mutant);

    let correctly_scoped =
        run_with_file_swapped(mutated_file, &mutated, Some(mutated_file), &make_runner()).unwrap();
    assert_eq!(correctly_scoped, MutantStatus::Killed);

    let wrongly_scoped =
        run_with_file_swapped(mutated_file, &mutated, Some(unrelated_file), &make_runner())
            .unwrap();
    assert_eq!(
        wrongly_scoped,
        MutantStatus::Survived,
        "mutant appeared killed even when only an unrelated file's tests ran — \
         related-file filtering isn't actually narrowing the test selection"
    );
}

#[test]
fn jest_naive_and_switch_agree() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-jest-demo");
    let file = project.join("src/math.js");
    let make_runner = || JestRunner::new(&project, Duration::from_secs(30));
    assert_naive_matches_switch(&file, None, make_runner);
    assert_naive_matches_switch(&file, Some(file.as_path()), make_runner);
}

#[test]
fn vitest_naive_and_switch_agree() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-vitest-demo");
    let file = project.join("src/math.js");
    let make_runner = || VitestRunner::new(&project, Duration::from_secs(30));
    assert_naive_matches_switch(&file, None, make_runner);
    assert_naive_matches_switch(&file, Some(file.as_path()), make_runner);
}

#[test]
fn jest_related_tests_filtering_is_real() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-jest-demo");
    assert_filtering_actually_filters(
        &project.join("src/math.js"),
        &project.join("src/greeting.js"),
        || JestRunner::new(&project, Duration::from_secs(30)),
    );
}

#[test]
fn vitest_related_tests_filtering_is_real() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-vitest-demo");
    assert_filtering_actually_filters(
        &project.join("src/math.js"),
        &project.join("src/greeting.js"),
        || VitestRunner::new(&project, Duration::from_secs(30)),
    );
}

#[test]
fn cache_signature_is_reused_across_runs_for_unchanged_mutants() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-jest-demo");
    let file = project.join("src/math.js");
    let cache_path = std::env::temp_dir().join(format!("mtr-cache-test-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&cache_path);

    let source_type = SourceType::from_path(&file).unwrap();
    let original = std::fs::read_to_string(&file).unwrap();
    let mutants = mtr_mutators::scan_source(&original, source_type);
    let file_key = file.to_str().unwrap();

    let instrumented = switch_instrument(&original, &mutants);
    let ids: Vec<_> = mutants.iter().map(|m| m.id).collect();
    let statuses = run_with_instrumented_file(
        &file,
        &instrumented,
        &ids,
        None,
        &JestRunner::new(&project, Duration::from_secs(30)),
    )
    .unwrap();

    let mut cache = MutantCache::load(&cache_path);
    for (mutant, (_, status)) in mutants.iter().zip(&statuses) {
        cache.set(mutant_signature(file_key, mutant, &original), *status);
    }
    cache.save(&cache_path).unwrap();

    // Reload as a fresh run would, and confirm every mutant's signature is
    // still a hit with the correct recorded status — no test re-execution
    // needed to know this.
    let reloaded = MutantCache::load(&cache_path);
    for (mutant, (_, expected_status)) in mutants.iter().zip(&statuses) {
        let signature = mutant_signature(file_key, mutant, &original);
        assert_eq!(
            reloaded.get(&signature),
            Some(*expected_status),
            "mutant {} should be a cache hit with its previously recorded status",
            mutant.id.0
        );
    }

    let _ = std::fs::remove_file(&cache_path);
}

#[test]
fn changed_since_scopes_to_only_the_modified_mutant() {
    let _guard = FIXTURE_LOCK.lock().unwrap();
    let project = workspace_root().join("fixtures/naive-jest-demo");
    let file = project.join("src/math.js");

    let original = std::fs::read_to_string(&file).unwrap();
    let modified = original.replace("return a === b;", "return (a) === b;");
    assert_ne!(original, modified, "fixture no longer contains the expected line to modify");
    std::fs::write(&file, &modified).unwrap();

    let source_type = SourceType::from_path(&file).unwrap();
    let all_mutants = mtr_mutators::scan_source(&modified, source_type);
    let changed = mtr_cache::changed_line_ranges(&project, "HEAD", &file);
    let scoped: Vec<_> = all_mutants
        .iter()
        .filter(|m| mtr_cache::mutant_touches_changed_lines(m, &modified, &changed))
        .collect();

    std::fs::write(&file, &original).unwrap();

    assert_eq!(
        scoped.len(),
        1,
        "only the mutant on the changed line should survive diff scoping, got {scoped:#?}"
    );
    assert_eq!(scoped[0].operator, "EqualityOperator");
}
