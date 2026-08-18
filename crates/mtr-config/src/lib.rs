use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TestRunner {
    #[default]
    Jest,
    Vitest,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Thresholds {
    pub high: f64,
    pub low: f64,
    /// Exit non-zero if the mutation score falls below this. `null` (the
    /// default) means never fail the build on score alone.
    #[serde(rename = "break", default)]
    pub break_score: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Glob patterns for files to mutate.
    #[serde(default = "default_mutate")]
    pub mutate: Vec<String>,
    /// Glob patterns to exclude from `mutate`.
    #[serde(default)]
    pub ignore_patterns: Vec<String>,
    /// Directory the test runner is invoked from.
    #[serde(default = "default_project")]
    pub project: String,
    #[serde(default)]
    pub test_runner: TestRunner,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Only run each mutant's related tests (the runner's own dependency
    /// resolver) instead of the whole suite.
    #[serde(default)]
    pub related_tests: bool,
    /// Path to a content-hash cache file; omit to disable caching.
    #[serde(default)]
    pub cache: Option<String>,
    /// Only test mutants touching lines changed since this git ref.
    #[serde(default)]
    pub changed_since: Option<String>,
    #[serde(default)]
    pub thresholds: Option<Thresholds>,
}

fn default_mutate() -> Vec<String> {
    vec!["src/**/*.js".to_string()]
}

fn default_project() -> String {
    ".".to_string()
}

fn default_timeout_secs() -> u64 {
    30
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mutate: default_mutate(),
            ignore_patterns: Vec::new(),
            project: default_project(),
            test_runner: TestRunner::default(),
            timeout_secs: default_timeout_secs(),
            related_tests: false,
            cache: None,
            changed_since: None,
            thresholds: None,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        json5::from_str(&contents).map_err(|e| format!("failed to parse {}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_apply_to_missing_fields() {
        let config: Config = json5::from_str("{}").unwrap();
        assert_eq!(config.mutate, vec!["src/**/*.js".to_string()]);
        assert_eq!(config.project, ".");
        assert_eq!(config.timeout_secs, 30);
        assert!(!config.related_tests);
        assert_eq!(config.test_runner, TestRunner::Jest);
        assert!(config.thresholds.is_none());
    }

    #[test]
    fn parses_jsonc_style_comments_and_trailing_commas() {
        let source = r#"{
            // which files to mutate
            mutate: ["src/**/*.js"],
            testRunner: "vitest",
            thresholds: { high: 80, low: 60, break: 50 },
        }"#;
        let config: Config = json5::from_str(source).unwrap();
        assert_eq!(config.test_runner, TestRunner::Vitest);
        let thresholds = config.thresholds.unwrap();
        assert_eq!(thresholds.break_score, Some(50.0));
    }
}
