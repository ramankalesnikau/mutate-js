use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use mtr_instrument::ACTIVE_MUTANT_ENV_VAR;
use mtr_test_runner_api::{
    escape_test_name_for_regex, parse_coverage_stream, RunOptions, SuiteCoverage, TestRunner,
};
use mtr_types::MutantStatus;
use serde::Deserialize;
use wait_timeout::ChildExt;

#[derive(Deserialize)]
struct JestSummary {
    success: bool,
}

/// `beforeEach`/`afterEach` diff of Jest's own `global.__coverage__`
/// (istanbul format, live-updated during the run — no extra npm dependency
/// needed, coverage ships with Jest). Written as one `MTR_COVERAGE {...}`
/// line per test via `process.stderr.write` directly — NOT `console.error`,
/// which Jest intercepts and reformats (wraps in a decorative "console.error"
/// block with a stack trace) rather than passing through verbatim, breaking
/// the simple one-line-per-test wire format. `.cjs` extension forces
/// CommonJS regardless of the project's own `"type": "module"` setting.
const JEST_COVERAGE_SETUP: &str = r#"
let __mtrSnapshot = {};
beforeEach(() => {
  __mtrSnapshot = JSON.parse(JSON.stringify(global.__coverage__ || {}));
});
afterEach(() => {
  const current = global.__coverage__ || {};
  const result = {};
  for (const file in current) {
    const beforeS = __mtrSnapshot[file] ? __mtrSnapshot[file].s : {};
    const afterS = current[file].s;
    const statementMap = current[file].statementMap;
    const touched = [];
    for (const id in afterS) {
      if (afterS[id] > (beforeS[id] || 0)) {
        touched.push(statementMap[id]);
      }
    }
    if (touched.length) result[file] = touched;
  }
  const testName = expect.getState().currentTestName || '';
  process.stderr.write('MTR_COVERAGE ' + JSON.stringify({ test: testName, coverage: result }) + '\n');
});
"#;

pub struct JestRunner {
    project_dir: PathBuf,
    timeout: Duration,
}

impl JestRunner {
    pub fn new(project_dir: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self { project_dir: project_dir.into(), timeout }
    }

    fn write_coverage_setup(&self) -> std::io::Result<PathBuf> {
        let path =
            std::env::temp_dir().join(format!("mtr-jest-coverage-{}.cjs", std::process::id()));
        std::fs::write(&path, JEST_COVERAGE_SETUP)?;
        Ok(path)
    }
}

impl TestRunner for JestRunner {
    fn run(&self, opts: RunOptions) -> MutantStatus {
        let mut cmd = Command::new("npx");
        cmd.args(["jest", "--json", "--silent", "--passWithNoTests"])
            .current_dir(&self.project_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(id) = opts.active_mutant {
            cmd.env(ACTIVE_MUTANT_ENV_VAR, id.0.to_string());
        }
        if let Some(tests) = opts.only_tests {
            let pattern = format!(
                "^(?:{})$",
                tests.iter().map(|t| escape_test_name_for_regex(t)).collect::<Vec<_>>().join("|")
            );
            cmd.arg("-t").arg(pattern);
        } else if let Some(path) = opts.related_to_file {
            cmd.arg("--findRelatedTests").arg(path);
        }

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(_) => return MutantStatus::Error,
        };

        let mut stdout_pipe = child.stdout.take().expect("stdout was piped");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = String::new();
            let _ = stdout_pipe.read_to_string(&mut buf);
            let _ = tx.send(buf);
        });

        match child.wait_timeout(self.timeout) {
            Ok(Some(status)) => {
                let stdout = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
                classify(&stdout, status.success())
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                MutantStatus::Timeout
            }
            Err(_) => MutantStatus::Error,
        }
    }

    fn collect_coverage(&self, related_to_file: Option<&Path>) -> Result<SuiteCoverage, String> {
        let script_path =
            self.write_coverage_setup().map_err(|e| format!("failed to write coverage setup: {e}"))?;

        let mut cmd = Command::new("npx");
        cmd.arg("jest")
            .arg("--coverage")
            .arg("--passWithNoTests")
            .arg(format!("--setupFilesAfterEnv={}", script_path.display()))
            .current_dir(&self.project_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(path) = related_to_file {
            cmd.arg("--findRelatedTests").arg(path);
        }

        let spawn_result = cmd.spawn();
        let cleanup = || {
            let _ = std::fs::remove_file(&script_path);
        };

        let mut child = match spawn_result {
            Ok(child) => child,
            Err(e) => {
                cleanup();
                return Err(format!("failed to spawn jest: {e}"));
            }
        };

        let mut stderr_pipe = child.stderr.take().expect("stderr was piped");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = String::new();
            let _ = stderr_pipe.read_to_string(&mut buf);
            let _ = tx.send(buf);
        });

        let result = match child.wait_timeout(self.timeout) {
            Ok(Some(_status)) => {
                let stderr = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
                Ok(parse_coverage_stream(&stderr))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                Err("timed out collecting coverage".to_string())
            }
            Err(e) => Err(format!("failed to wait for jest: {e}")),
        };

        cleanup();
        result
    }
}

/// `exit_success`: falls back on when stdout isn't valid JSON at all — e.g.
/// zero related tests matched. A clean process exit with unparseable output
/// means nothing could have killed the mutant, not a real failure.
fn classify(stdout: &str, exit_success: bool) -> MutantStatus {
    match serde_json::from_str::<JestSummary>(stdout) {
        Ok(summary) if summary.success => MutantStatus::Survived,
        Ok(_) => MutantStatus::Killed,
        Err(_) if exit_success => MutantStatus::Survived,
        Err(_) => MutantStatus::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Duration;

    fn fixture_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/naive-jest-demo")
            .canonicalize()
            .expect("fixture dir should exist")
    }

    #[test]
    fn collect_coverage_reports_each_tests_touched_statements() {
        let project = fixture_dir();
        let runner = JestRunner::new(&project, Duration::from_secs(30));
        let coverage = runner.collect_coverage(None).unwrap();
        assert_eq!(coverage.tests.len(), 4, "fixture has 4 tests total: {:?}", coverage.tests);

        let sum_test = coverage
            .tests
            .iter()
            .find(|t| t.test_id == "sum adds two numbers")
            .expect("`sum adds two numbers` should be in the collected coverage");
        let math_js = project.join("src/math.js");
        assert!(
            sum_test.touched.contains_key(&math_js),
            "the `sum` test should touch math.js: {:?}",
            sum_test.touched.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn collect_coverage_respects_related_to_file_as_a_prefilter() {
        let project = fixture_dir();
        let file = project.join("src/math.js");
        let runner = JestRunner::new(&project, Duration::from_secs(30));
        let coverage = runner.collect_coverage(Some(&file)).unwrap();
        assert_eq!(
            coverage.tests.len(),
            3,
            "only math.test.js's 3 tests should run when scoped to math.js: {:?}",
            coverage.tests.iter().map(|t| &t.test_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn run_with_only_tests_runs_exactly_the_named_test() {
        let project = fixture_dir();
        let runner = JestRunner::new(&project, Duration::from_secs(30));
        let tests = vec!["sum adds two numbers".to_string()];
        let opts = RunOptions { active_mutant: None, related_to_file: None, only_tests: Some(&tests) };
        assert_eq!(runner.run(opts), MutantStatus::Survived);
    }

    #[test]
    fn run_with_only_tests_accepts_multiple_names() {
        let project = fixture_dir();
        let runner = JestRunner::new(&project, Duration::from_secs(30));
        let tests =
            vec!["sum adds two numbers".to_string(), "isEqual compares strictly".to_string()];
        let opts = RunOptions { active_mutant: None, related_to_file: None, only_tests: Some(&tests) };
        assert_eq!(runner.run(opts), MutantStatus::Survived);
    }
}
