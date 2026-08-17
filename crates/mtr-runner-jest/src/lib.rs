use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use mtr_instrument::ACTIVE_MUTANT_ENV_VAR;
use mtr_test_runner_api::{RunOptions, TestRunner};
use mtr_types::MutantStatus;
use serde::Deserialize;
use wait_timeout::ChildExt;

#[derive(Deserialize)]
struct JestSummary {
    success: bool,
}

pub struct JestRunner {
    project_dir: PathBuf,
    timeout: Duration,
}

impl JestRunner {
    pub fn new(project_dir: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self { project_dir: project_dir.into(), timeout }
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
        if let Some(path) = opts.related_to_file {
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
