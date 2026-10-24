use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
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
struct VitestSummary {
    success: bool,
}

pub struct VitestRunner {
    project_dir: PathBuf,
    timeout: Duration,
}

/// `beforeEach`/`afterEach` diff of Vitest's own coverage global — which is
/// `globalThis.__VITEST_COVERAGE__`, NOT `__coverage__` like Jest (confirmed
/// by reading `@vitest/coverage-istanbul`'s own `COVERAGE_STORE_KEY`
/// constant). Otherwise identical to Jest's setup script: writes one
/// `MTR_COVERAGE {...}` line per test via `process.stderr.write`.
const VITEST_COVERAGE_SETUP: &str = r#"
import { beforeEach, afterEach } from 'vitest';

let __mtrSnapshot = {};
let __mtrBaseline = null;
beforeEach(() => {
  __mtrSnapshot = JSON.parse(JSON.stringify(globalThis.__VITEST_COVERAGE__ || {}));
  if (__mtrBaseline === null) __mtrBaseline = __mtrSnapshot;
});
afterEach((ctx) => {
  const current = globalThis.__VITEST_COVERAGE__ || {};
  const result = {};
  for (const file in current) {
    const beforeS = __mtrSnapshot[file] ? __mtrSnapshot[file].s : {};
    const baselineS = __mtrBaseline && __mtrBaseline[file] ? __mtrBaseline[file].s : {};
    const afterS = current[file].s;
    const statementMap = current[file].statementMap;
    const touched = [];
    for (const id in afterS) {
      if (afterS[id] > (beforeS[id] || 0) || (baselineS[id] || 0) > 0) {
        touched.push(statementMap[id]);
      }
    }
    if (touched.length) result[file] = touched;
  }
  process.stderr.write('MTR_COVERAGE ' + JSON.stringify({ test: ctx.task.name, coverage: result }) + '\n');
});
"#;

/// Candidate filenames for the project's own Vite/Vitest config, checked in
/// this order. Vitest falls back to a plain `vite.config.*` when it has no
/// `test` block of its own, so both families are worth detecting.
const USER_CONFIG_CANDIDATES: &[&str] = &[
    "vitest.config.ts",
    "vitest.config.mts",
    "vitest.config.js",
    "vitest.config.mjs",
    "vitest.config.cjs",
    "vite.config.ts",
    "vite.config.mts",
    "vite.config.js",
    "vite.config.mjs",
];

/// `std::process::id()` alone collides when multiple `VitestRunner`s run
/// concurrently in the same process — e.g. parallel `cargo test` threads —
/// since they'd all compute the same temp paths and race on write/read/delete
/// of those files. This counter, combined with the pid, keeps every call to
/// `write_coverage_config` unique regardless of threading.
static COVERAGE_CONFIG_COUNTER: AtomicU32 = AtomicU32::new(0);

impl VitestRunner {
    pub fn new(project_dir: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self { project_dir: project_dir.into(), timeout }
    }

    /// Writes two temp files into the project directory (not `std::env::temp_dir()`
    /// — Vite's config loader bundles the config file's imports via esbuild,
    /// which only resolves `node_modules` relative to the config file's own
    /// location, not the project root passed via `--root`) and returns the
    /// generated config's path:
    ///
    /// - the coverage-setup script (referenced by absolute path, so it can
    ///   still live anywhere)
    /// - a `defineConfig`/`mergeConfig` wrapper enabling coverage/singleFork
    ///   on top of the project's own config, when one exists. The import of
    ///   the user's config uses a static, literal specifier (never a computed
    ///   path) — a dynamic `import(variablePath)` forces esbuild to bundle
    ///   the *entire* reachable `node_modules` graph speculatively, which
    ///   reliably fails on real projects (confirmed: it chokes on sourcemap
    ///   files deep in transitive deps that have no configured loader).
    fn write_coverage_config(&self) -> std::io::Result<(PathBuf, PathBuf)> {
        let pid = std::process::id();
        let unique = COVERAGE_CONFIG_COUNTER.fetch_add(1, Ordering::Relaxed);
        let setup_path =
            self.project_dir.join(format!(".mtr-vitest-coverage-setup-{pid}-{unique}.mjs"));
        std::fs::write(&setup_path, VITEST_COVERAGE_SETUP)?;

        let user_config = USER_CONFIG_CANDIDATES
            .iter()
            .find(|name| self.project_dir.join(name).is_file());

        let overrides = format!(
            r#"{{
  test: {{
    setupFiles: ['{}'],
    pool: 'forks',
    poolOptions: {{ forks: {{ singleFork: true }} }},
    coverage: {{ enabled: true, provider: 'istanbul', reporter: ['json'] }},
  }},
}}"#,
            setup_path.display()
        );
        let config_source = match user_config {
            Some(name) => format!(
                "import {{ defineConfig, mergeConfig }} from 'vitest/config';\n\
                 import userConfig from './{name}';\n\
                 export default mergeConfig(userConfig, defineConfig({overrides}));\n"
            ),
            None => format!(
                "import {{ defineConfig }} from 'vitest/config';\n\
                 export default defineConfig({overrides});\n"
            ),
        };
        let config_path =
            self.project_dir.join(format!(".mtr-vitest-coverage-{pid}-{unique}.config.mjs"));
        std::fs::write(&config_path, config_source)?;

        Ok((config_path, setup_path))
    }
}

impl TestRunner for VitestRunner {
    fn run(&self, opts: RunOptions) -> MutantStatus {
        let mut cmd = Command::new("npx");
        cmd.arg("vitest");
        if opts.only_tests.is_some() {
            cmd.arg("run");
        } else {
            match opts.related_to_file {
                Some(path) => {
                    cmd.arg("related").arg(path).arg("--run");
                }
                None => {
                    cmd.arg("run");
                }
            }
        }
        cmd.arg("--reporter=json")
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
        let (config_path, setup_path) = self
            .write_coverage_config()
            .map_err(|e| format!("failed to write coverage config: {e}"))?;
        let cleanup = || {
            let _ = std::fs::remove_file(&config_path);
            let _ = std::fs::remove_file(&setup_path);
        };

        let mut cmd = Command::new("npx");
        cmd.arg("vitest");
        match related_to_file {
            Some(path) => {
                cmd.arg("related").arg(path).arg("--run");
            }
            None => {
                cmd.arg("run");
            }
        }
        cmd.arg("--config")
            .arg(&config_path)
            .current_dir(&self.project_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                cleanup();
                return Err(format!("failed to spawn vitest: {e}"));
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
            Ok(Some(status)) => {
                let stderr = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
                let coverage = parse_coverage_stream(&stderr);
                if coverage.tests.is_empty() && !status.success() {
                    // A clean run with zero MTR_COVERAGE lines usually means
                    // zero related tests matched (not an error) — but a
                    // failing exit code alongside zero lines means
                    // coverage collection itself never got off the ground
                    // (e.g. `@vitest/coverage-istanbul` isn't installed,
                    // which Vitest reports as its own `MISSING DEPENDENCY`
                    // message — shown here verbatim rather than inventing
                    // separate wording for it).
                    Err(format!(
                        "vitest exited with a failure while collecting coverage; \
                         is @vitest/coverage-istanbul installed?\n{stderr}"
                    ))
                } else {
                    Ok(coverage)
                }
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                Err("timed out collecting coverage".to_string())
            }
            Err(e) => Err(format!("failed to wait for vitest: {e}")),
        };

        cleanup();
        result
    }
}

/// `exit_success`: falls back on when stdout isn't valid JSON at all — e.g.
/// zero related tests matched, which prints a plain-text message instead of
/// a JSON reporter result. A clean process exit with unparseable output means
/// nothing could have killed the mutant, not a real failure.
fn classify(stdout: &str, exit_success: bool) -> MutantStatus {
    match serde_json::from_str::<VitestSummary>(stdout) {
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
    use std::sync::Mutex;

    fn fixture_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/naive-vitest-demo")
            .canonicalize()
            .expect("fixture dir should exist")
    }

    // `cargo test` runs these in parallel by default, but every test here
    // spawns a real `vitest` process against the same fixture directory —
    // concurrent runs race on its `coverage/` output dir and Vite's cache,
    // corrupting each other (confirmed: one run's generated coverage config
    // got deleted mid-read by another run's cleanup).
    static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn collect_coverage_reports_each_tests_touched_statements() {
        let _guard = FIXTURE_LOCK.lock().unwrap();
        let project = fixture_dir();
        let runner = VitestRunner::new(&project, Duration::from_secs(30));
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
        let _guard = FIXTURE_LOCK.lock().unwrap();
        let project = fixture_dir();
        let file = project.join("src/math.js");
        let runner = VitestRunner::new(&project, Duration::from_secs(30));
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
        let _guard = FIXTURE_LOCK.lock().unwrap();
        let project = fixture_dir();
        let runner = VitestRunner::new(&project, Duration::from_secs(30));
        let tests = vec!["sum adds two numbers".to_string()];
        let opts = RunOptions { active_mutant: None, related_to_file: None, only_tests: Some(&tests) };
        assert_eq!(runner.run(opts), MutantStatus::Survived);
    }

    #[test]
    fn run_with_only_tests_accepts_multiple_names() {
        let _guard = FIXTURE_LOCK.lock().unwrap();
        let project = fixture_dir();
        let runner = VitestRunner::new(&project, Duration::from_secs(30));
        let tests =
            vec!["sum adds two numbers".to_string(), "isEqual compares strictly".to_string()];
        let opts = RunOptions { active_mutant: None, related_to_file: None, only_tests: Some(&tests) };
        assert_eq!(runner.run(opts), MutantStatus::Survived);
    }
}
