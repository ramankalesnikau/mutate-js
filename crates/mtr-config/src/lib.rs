use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    /// Which test runner to use, e.g. `"jest"` or `"vitest"`. Validated
    /// against `mtr-core`'s `TestRunnerRegistry` at use, not here — this
    /// crate has no opinion on which runners exist.
    #[serde(default = "default_test_runner")]
    pub test_runner: String,
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
    /// Which reporters to activate, e.g. `["json"]` or `["text"]`.
    #[serde(default = "default_reporters")]
    pub reporters: Vec<String>,
    /// Extra mutator packs to include alongside the core catalog, e.g. `["react"]`.
    #[serde(default)]
    pub mutator_packs: Vec<String>,
    /// Operator names (e.g. `["StringLiteral", "ArrayDeclaration"]`) to
    /// exclude from the mutator catalog, core and packs alike. Use this to
    /// silence a chronically noisy operator instead of ignoring every
    /// mutant it produces one by one.
    #[serde(default)]
    pub excluded_mutators: Vec<String>,
}

fn default_mutate() -> Vec<String> {
    vec!["src/**/*.js".to_string()]
}

fn default_project() -> String {
    ".".to_string()
}

fn default_test_runner() -> String {
    "jest".to_string()
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_reporters() -> Vec<String> {
    vec!["json".to_string()]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mutate: default_mutate(),
            ignore_patterns: Vec::new(),
            project: default_project(),
            test_runner: default_test_runner(),
            timeout_secs: default_timeout_secs(),
            related_tests: false,
            cache: None,
            changed_since: None,
            thresholds: None,
            reporters: default_reporters(),
            mutator_packs: Vec::new(),
            excluded_mutators: Vec::new(),
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
        assert_eq!(config.test_runner, "jest");
        assert!(config.thresholds.is_none());
        assert_eq!(config.reporters, vec!["json".to_string()]);
        assert!(config.mutator_packs.is_empty());
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
        assert_eq!(config.test_runner, "vitest");
        let thresholds = config.thresholds.unwrap();
        assert_eq!(thresholds.break_score, Some(50.0));
    }
}
