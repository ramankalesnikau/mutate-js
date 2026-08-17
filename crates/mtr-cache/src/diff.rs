use std::path::Path;
use std::process::Command;

use mtr_types::Mutant;

/// Inclusive 1-indexed line ranges added or modified in `file` since
/// `git_ref`, read from `git diff --unified=0` hunk headers.
pub fn changed_line_ranges(repo_dir: &Path, git_ref: &str, file: &Path) -> Vec<(u32, u32)> {
    let output = Command::new("git")
        .args(["diff", "--unified=0", git_ref, "--"])
        .arg(file)
        .current_dir(repo_dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git diff: {e}"));

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("@@ "))
        .filter_map(|hunk| hunk.split_whitespace().find(|token| token.starts_with('+')))
        .filter_map(|new_range| {
            let spec = &new_range[1..];
            let mut parts = spec.splitn(2, ',');
            let start: u32 = parts.next()?.parse().ok()?;
            let count: u32 = parts.next().map_or(Some(1), |c| c.parse().ok())?;
            (count > 0).then_some((start, start + count - 1))
        })
        .collect()
}

fn line_number_at(source: &str, byte_offset: u32) -> u32 {
    source[..byte_offset as usize].matches('\n').count() as u32 + 1
}

/// Whether `mutant`'s enclosing span overlaps any of `changed` (inclusive
/// 1-indexed line ranges).
pub fn mutant_touches_changed_lines(mutant: &Mutant, source: &str, changed: &[(u32, u32)]) -> bool {
    let start_line = line_number_at(source, mutant.enclosing_span.start);
    let end_line = line_number_at(source, mutant.enclosing_span.end);
    changed.iter().any(|&(cs, ce)| start_line <= ce && end_line >= cs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_number_at_counts_newlines() {
        let source = "line1\nline2\nline3";
        assert_eq!(line_number_at(source, 0), 1);
        assert_eq!(line_number_at(source, 6), 2);
        assert_eq!(line_number_at(source, 12), 3);
    }

    #[test]
    fn overlap_detection() {
        let source = "a\nb\nc\nd\ne";
        // byte offset of the start of 1-indexed `line` in `source`
        let offset_of_line = |line: u32| -> u32 {
            source.match_indices('\n').nth(line as usize - 2).map_or(0, |(i, _)| i as u32 + 1)
        };

        assert!(mutant_touches_changed_lines(
            &mutant_spanning(offset_of_line(3), offset_of_line(3) + 1),
            source,
            &[(2, 4)]
        ));
        assert!(!mutant_touches_changed_lines(
            &mutant_spanning(offset_of_line(5), offset_of_line(5) + 1),
            source,
            &[(2, 4)]
        ));
    }

    fn mutant_spanning(start: u32, end: u32) -> Mutant {
        use mtr_types::{MutantId, Span};
        Mutant {
            id: MutantId(0),
            operator: "Test".to_string(),
            span: Span { start, end },
            enclosing_span: Span { start, end },
            original: String::new(),
            replacement: String::new(),
        }
    }
}
