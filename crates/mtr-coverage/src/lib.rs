//! mtr-coverage — pure mapping from a runner's per-test statement coverage
//! (collected by `TestRunner::collect_coverage`) to "which tests actually
//! cover this mutant", with no process spawning or I/O. Separated into its
//! own crate following this workspace's one-crate-per-pipeline-stage
//! convention (`mtr-cache`, `mtr-instrument`, `mtr-mutators`, ...).

use std::path::Path;

use mtr_test_runner_api::SuiteCoverage;
use mtr_types::Mutant;

/// Same logic as `mtr-cache`'s private `line_number_at` — small enough to
/// duplicate here rather than add a cross-crate dependency for one line.
fn byte_offset_to_line(source: &str, byte_offset: u32) -> u32 {
    source[..byte_offset as usize].matches('\n').count() as u32 + 1
}

/// Which tests (by `test_id`) touch `mutant`'s `enclosing_span` in `file`,
/// found by comparing `mutant`'s line range against each test's recorded
/// coverage for that file. A mutant with no overlapping statement in any
/// test's coverage returns an empty list — the caller treats that as
/// `MutantStatus::NoCoverage` without ever spawning a test process for it.
pub fn tests_covering_mutant(
    coverage: &SuiteCoverage,
    file: &Path,
    source: &str,
    mutant: &Mutant,
) -> Vec<String> {
    let mutant_start = byte_offset_to_line(source, mutant.enclosing_span.start);
    let mutant_end = byte_offset_to_line(source, mutant.enclosing_span.end);

    coverage
        .tests
        .iter()
        .filter(|test| {
            test.touched.get(file).is_some_and(|statements| {
                statements
                    .iter()
                    .any(|s| mutant_start <= s.end_line && mutant_end >= s.start_line)
            })
        })
        .map(|test| test.test_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtr_test_runner_api::{CoveredStatement, TestCoverage};
    use mtr_types::{MutantId, Span};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn mutant_spanning(start: u32, end: u32) -> Mutant {
        Mutant {
            id: MutantId(0),
            operator: "Test".to_string(),
            span: Span { start, end },
            enclosing_span: Span { start, end },
            original: String::new(),
            replacement: String::new(),
        }
    }

    fn coverage_with(file: &Path, entries: Vec<(&str, Vec<(u32, u32)>)>) -> SuiteCoverage {
        SuiteCoverage {
            tests: entries
                .into_iter()
                .map(|(test_id, ranges)| {
                    let statements = ranges
                        .into_iter()
                        .map(|(start_line, end_line)| CoveredStatement { start_line, end_line })
                        .collect();
                    let mut touched = HashMap::new();
                    touched.insert(file.to_path_buf(), statements);
                    TestCoverage { test_id: test_id.to_string(), touched }
                })
                .collect(),
        }
    }

    #[test]
    fn resolves_only_the_test_covering_the_mutants_line() {
        let source = "line1\nline2\nline3\n";
        let file = PathBuf::from("/p/math.js");
        let coverage = coverage_with(
            &file,
            vec![("test a", vec![(1, 1)]), ("test b", vec![(2, 2)])],
        );
        // mutant spans byte range covering line 2 only (offset 6..10 == "line2")
        let mutant = mutant_spanning(6, 10);
        let found = tests_covering_mutant(&coverage, &file, source, &mutant);
        assert_eq!(found, vec!["test b".to_string()]);
    }

    #[test]
    fn returns_empty_when_no_test_touches_the_mutants_line() {
        let source = "line1\nline2\nline3\n";
        let file = PathBuf::from("/p/math.js");
        let coverage = coverage_with(&file, vec![("test a", vec![(1, 1)])]);
        let mutant = mutant_spanning(12, 17); // line 3
        let found = tests_covering_mutant(&coverage, &file, source, &mutant);
        assert!(found.is_empty());
    }

    #[test]
    fn ignores_coverage_recorded_for_a_different_file() {
        let source = "line1\nline2\n";
        let file = PathBuf::from("/p/math.js");
        let other = PathBuf::from("/p/other.js");
        let coverage = coverage_with(&other, vec![("test a", vec![(1, 1)])]);
        let mutant = mutant_spanning(0, 5);
        let found = tests_covering_mutant(&coverage, &file, source, &mutant);
        assert!(found.is_empty());
    }

    #[test]
    fn multiple_tests_covering_the_same_line_all_resolve() {
        let source = "line1\n";
        let file = PathBuf::from("/p/math.js");
        let coverage =
            coverage_with(&file, vec![("test a", vec![(1, 1)]), ("test b", vec![(1, 1)])]);
        let mutant = mutant_spanning(0, 5);
        let found = tests_covering_mutant(&coverage, &file, source, &mutant);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&"test a".to_string()));
        assert!(found.contains(&"test b".to_string()));
    }

    #[test]
    fn mutant_spanning_multiple_lines_matches_overlapping_statement() {
        let source = "line1\nline2\nline3\n";
        let file = PathBuf::from("/p/math.js");
        // statement covers lines 2-3; mutant also spans lines 2-3
        let coverage = coverage_with(&file, vec![("test a", vec![(2, 3)])]);
        let mutant = mutant_spanning(6, 17);
        let found = tests_covering_mutant(&coverage, &file, source, &mutant);
        assert_eq!(found, vec!["test a".to_string()]);
    }
}
