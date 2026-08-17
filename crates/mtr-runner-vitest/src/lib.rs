use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use mtr_test_runner_api::TestRunner;
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

impl VitestRunner {
    pub fn new(project_dir: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self { project_dir: project_dir.into(), timeout }
    }
}

impl TestRunner for VitestRunner {
    fn run(&self) -> MutantStatus {
        let mut child = match Command::new("npx")
            .args(["vitest", "run", "--reporter=json"])
            .current_dir(&self.project_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
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
            Ok(Some(_)) => {
                let stdout = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
                classify(&stdout)
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

fn classify(stdout: &str) -> MutantStatus {
    match serde_json::from_str::<VitestSummary>(stdout) {
        Ok(summary) if summary.success => MutantStatus::Survived,
        Ok(_) => MutantStatus::Killed,
        Err(_) => MutantStatus::Error,
    }
}
