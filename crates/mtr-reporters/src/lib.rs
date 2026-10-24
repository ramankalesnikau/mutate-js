use mtr_reporter_api::{FileMutantResults, Reporter};
use mtr_types::{MutantResult, MutantStatus};

/// Accumulates every file's results and prints one JSON blob at the end —
/// today's original CLI behavior, refactored into the `Reporter` trait
/// rather than replaced.
#[derive(Default)]
pub struct JsonReporter {
    files: Vec<FileMutantResults>,
}

impl Reporter for JsonReporter {
    fn on_file_complete(&mut self, file: &str, source: &str, results: &[MutantResult]) {
        self.files.push(FileMutantResults {
            file: file.to_string(),
            mutants: results.to_vec(),
            source: source.to_string(),
        });
    }

    fn wrap_up(&mut self) {
        println!("{}", serde_json::to_string_pretty(&self.files).unwrap());
    }
}

/// Human-readable: one line per mutant as its file completes, then a final
/// score summary — proves the trait handles incremental-as-it-happens
/// output, not just JsonReporter's batch-at-the-end style.
#[derive(Default)]
pub struct TextReporter;

impl Reporter for TextReporter {
    fn on_mutant_tested(&mut self, file: &str, result: &MutantResult) {
        let label = match result.status {
            MutantStatus::Killed => "killed",
            MutantStatus::Survived => "SURVIVED",
            MutantStatus::Timeout => "timeout",
            MutantStatus::Error => "error",
            MutantStatus::NoCoverage => "NO-COV",
        };
        println!(
            "{label:<9} {file}  {} `{}` -> `{}`",
            result.mutant.operator, result.mutant.original, result.mutant.replacement
        );
    }

    fn on_run_complete(&mut self, files: &[FileMutantResults], score: f64) {
        let total: usize = files.iter().map(|f| f.mutants.len()).sum();
        println!(
            "\n{score:.1}% mutation score ({total} mutant(s) across {} file(s))",
            files.len()
        );
    }
}

/// A self-contained HTML file: a summary (score, kill/survive/timeout/error
/// counts) followed by every file's source with each mutant's span
/// highlighted in place and a tooltip giving the operator and edit. Written
/// to [`HtmlReporter::DEFAULT_OUTPUT_PATH`] on `wrap_up` — one static file,
/// no server or JS runtime needed to view it.
#[derive(Default)]
pub struct HtmlReporter {
    files: Vec<FileMutantResults>,
    score: f64,
}

impl HtmlReporter {
    pub const DEFAULT_OUTPUT_PATH: &'static str = "mutation-report.html";
}

impl Reporter for HtmlReporter {
    fn on_file_complete(&mut self, file: &str, source: &str, results: &[MutantResult]) {
        self.files.push(FileMutantResults {
            file: file.to_string(),
            mutants: results.to_vec(),
            source: source.to_string(),
        });
    }

    fn on_run_complete(&mut self, _files: &[FileMutantResults], score: f64) {
        self.score = score;
    }

    fn wrap_up(&mut self) {
        let html = render_report(&self.files, self.score);
        match std::fs::write(Self::DEFAULT_OUTPUT_PATH, html) {
            Ok(()) => eprintln!("HTML report: {}", Self::DEFAULT_OUTPUT_PATH),
            Err(e) => eprintln!("failed to write {}: {e}", Self::DEFAULT_OUTPUT_PATH),
        }
    }
}

fn escape_html(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&#39;".to_string(),
            other => other.to_string(),
        })
        .collect()
}

fn status_class(status: MutantStatus) -> &'static str {
    match status {
        MutantStatus::Killed => "killed",
        MutantStatus::Survived => "survived",
        MutantStatus::Timeout => "timeout",
        MutantStatus::Error => "error",
        MutantStatus::NoCoverage => "nocoverage",
    }
}

fn status_label(status: MutantStatus) -> &'static str {
    match status {
        MutantStatus::Killed => "Killed",
        MutantStatus::Survived => "Survived",
        MutantStatus::Timeout => "Timeout",
        MutantStatus::Error => "Error",
        MutantStatus::NoCoverage => "No coverage",
    }
}

/// Mutants sharing an identical `span` (e.g. two operators independently
/// matching the same site with different replacements) render as one
/// highlighted region listing every one of them in its tooltip, rather than
/// nesting `<mark>` tags the browser would otherwise have to untangle.
fn group_by_span(results: &[MutantResult]) -> Vec<(mtr_types::Span, Vec<&MutantResult>)> {
    let mut sorted: Vec<&MutantResult> = results.iter().collect();
    sorted.sort_by_key(|r| (r.mutant.span.start, r.mutant.span.end));

    let mut groups: Vec<(mtr_types::Span, Vec<&MutantResult>)> = Vec::new();
    for result in sorted {
        match groups.last_mut() {
            Some((span, group)) if *span == result.mutant.span => group.push(result),
            _ => groups.push((result.mutant.span, vec![result])),
        }
    }
    groups
}

/// The worst (most test-quality-relevant) status in a group drives the
/// highlight color: a survived mutant is the signal worth seeing first, so
/// it outranks a killed one at the same site. `NoCoverage` outranks even
/// `Survived` — nothing even tried is a stronger signal than "ran and missed."
fn worst_status(group: &[&MutantResult]) -> MutantStatus {
    group
        .iter()
        .map(|r| r.status)
        .max_by_key(|s| match s {
            MutantStatus::Killed => 0,
            MutantStatus::Timeout => 1,
            MutantStatus::Error => 2,
            MutantStatus::Survived => 3,
            MutantStatus::NoCoverage => 4,
        })
        .unwrap_or(MutantStatus::Killed)
}

fn render_source(source: &str, results: &[MutantResult]) -> String {
    let groups = group_by_span(results);
    let mut out = String::new();
    let mut cursor = 0usize;

    for (span, group) in groups {
        let start = span.start as usize;
        let end = span.end as usize;
        // Defensive: current operators only ever produce equal or disjoint
        // spans (never partially overlapping), but a future operator that
        // broke that invariant should degrade to "skip the overlap" rather
        // than panic on an out-of-order slice index.
        if start < cursor {
            continue;
        }
        out.push_str(&escape_html(&source[cursor..start]));

        let status = worst_status(&group);
        let tooltip = group
            .iter()
            .map(|r| {
                format!(
                    "{}: {} -> {} ({})",
                    r.mutant.operator,
                    r.mutant.original,
                    r.mutant.replacement,
                    status_label(r.status)
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        out.push_str(&format!(
            "<mark class=\"mutant {}\" title=\"{}\">{}</mark>",
            status_class(status),
            escape_html(&tooltip),
            escape_html(&source[start..end])
        ));
        cursor = end;
    }
    out.push_str(&escape_html(&source[cursor..]));
    out
}

fn render_report(files: &[FileMutantResults], score: f64) -> String {
    let (killed, survived, timeout, error, no_coverage) = files.iter().flat_map(|f| &f.mutants).fold(
        (0u32, 0u32, 0u32, 0u32, 0u32),
        |(k, s, t, e, n), r| match r.status {
            MutantStatus::Killed => (k + 1, s, t, e, n),
            MutantStatus::Survived => (k, s + 1, t, e, n),
            MutantStatus::Timeout => (k, s, t + 1, e, n),
            MutantStatus::Error => (k, s, t, e + 1, n),
            MutantStatus::NoCoverage => (k, s, t, e, n + 1),
        },
    );
    let total = killed + survived + timeout + error + no_coverage;

    let mut files_html = String::new();
    for file in files {
        files_html.push_str(&format!(
            "<section class=\"file\"><h2>{}</h2><pre>{}</pre></section>\n",
            escape_html(&file.file),
            render_source(&file.source, &file.mutants)
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Mutation report</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, sans-serif; margin: 2rem; color: #1a1a1a; }}
  h1 {{ margin-bottom: 0.25rem; }}
  .summary {{ margin-bottom: 1.5rem; color: #444; }}
  .file h2 {{ font-family: monospace; font-size: 0.95rem; background: #f3f3f3; padding: 0.4rem 0.6rem; border-radius: 4px; }}
  pre {{ background: #fafafa; border: 1px solid #e0e0e0; border-radius: 4px; padding: 1rem; overflow-x: auto; white-space: pre-wrap; }}
  mark.mutant {{ border-radius: 3px; padding: 0 2px; cursor: help; }}
  mark.killed {{ background: #c8f7c5; }}
  mark.survived {{ background: #f7c5c5; }}
  mark.timeout {{ background: #f7e8a4; }}
  mark.error {{ background: #d9d9d9; }}
  mark.nocoverage {{ background: #d6e4ff; }}
</style>
</head>
<body>
<h1>Mutation report</h1>
<p class="summary">Score: <strong>{score:.1}%</strong> — {killed} killed, {survived} survived, {timeout} timeout, {error} error, {no_coverage} no coverage ({total} mutant(s) across {file_count} file(s))</p>
{files_html}
</body>
</html>
"#,
        file_count = files.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtr_types::{Mutant, MutantId, Span};

    fn mutant_result(
        id: u32,
        operator: &str,
        span: (u32, u32),
        original: &str,
        replacement: &str,
        status: MutantStatus,
    ) -> MutantResult {
        MutantResult {
            mutant: Mutant {
                id: MutantId(id),
                operator: operator.to_string(),
                span: Span { start: span.0, end: span.1 },
                enclosing_span: Span { start: span.0, end: span.1 },
                original: original.to_string(),
                replacement: replacement.to_string(),
            },
            status,
        }
    }

    #[test]
    fn render_source_highlights_a_mutant_span_and_escapes_the_rest() {
        let source = "const x = a < b;";
        let results = vec![mutant_result(0, "Op", (10, 11), "a", "b", MutantStatus::Survived)];
        let html = render_source(source, &results);
        assert!(html.contains("<mark class=\"mutant survived\""));
        assert!(html.contains("&lt;"), "the unrelated `<` must still be escaped: {html}");
    }

    #[test]
    fn render_source_groups_mutants_sharing_an_identical_span() {
        let source = "const x = true;";
        let results = vec![
            mutant_result(0, "A", (10, 14), "true", "false", MutantStatus::Killed),
            mutant_result(1, "B", (10, 14), "true", "false", MutantStatus::Survived),
        ];
        let html = render_source(source, &results);
        assert_eq!(html.matches("<mark").count(), 1, "identical spans must render as one mark");
        assert!(html.contains("class=\"mutant survived\""), "worst status should drive the class");
    }

    #[test]
    fn render_report_includes_score_and_counts() {
        let files = vec![FileMutantResults {
            file: "src/a.js".to_string(),
            source: "const a = 1 + 2;".to_string(),
            mutants: vec![mutant_result(0, "Arith", (12, 13), "+", "-", MutantStatus::Killed)],
        }];
        let html = render_report(&files, 100.0);
        assert!(html.contains("100.0%"));
        assert!(html.contains("src/a.js"));
        assert!(html.contains("1 killed"));
    }

    #[test]
    fn render_report_counts_no_coverage_separately() {
        let files = vec![FileMutantResults {
            file: "src/a.js".to_string(),
            source: "const a = 1 + 2;".to_string(),
            mutants: vec![mutant_result(0, "Arith", (12, 13), "+", "-", MutantStatus::NoCoverage)],
        }];
        let html = render_report(&files, 0.0);
        assert!(html.contains("1 no coverage"));
    }

    #[test]
    fn no_coverage_outranks_survived_in_a_shared_span_group() {
        let source = "const x = true;";
        let results = vec![
            mutant_result(0, "A", (10, 14), "true", "false", MutantStatus::Survived),
            mutant_result(1, "B", (10, 14), "true", "false", MutantStatus::NoCoverage),
        ];
        let html = render_source(source, &results);
        assert!(html.contains("class=\"mutant nocoverage\""));
    }
}
