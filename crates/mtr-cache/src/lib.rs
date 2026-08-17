use std::collections::HashMap;
use std::path::Path;

use mtr_types::{Mutant, MutantStatus};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod diff;
pub use diff::{changed_line_ranges, mutant_touches_changed_lines};

/// Identifies a mutant by what it actually changes, not where — hashing the
/// unmutated text at `enclosing_span` (not byte offsets, which shift when
/// unrelated code elsewhere in the file changes) means the same edit gets
/// the same signature run to run, even if its position in the file moved.
pub fn mutant_signature(relative_file: &str, mutant: &Mutant, source: &str) -> String {
    let start = mutant.enclosing_span.start as usize;
    let end = mutant.enclosing_span.end as usize;
    let enclosing_text = &source[start..end];

    let mut hasher = Sha256::new();
    hasher.update(relative_file.as_bytes());
    hasher.update(b"\0");
    hasher.update(mutant.operator.as_bytes());
    hasher.update(b"\0");
    hasher.update(enclosing_text.as_bytes());
    hasher.update(b"\0");
    hasher.update(mutant.replacement.as_bytes());

    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Default, Serialize, Deserialize)]
pub struct MutantCache {
    entries: HashMap<String, MutantStatus>,
}

impl MutantCache {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_json::from_str(&contents).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).expect("MutantCache always serializes");
        std::fs::write(path, json)
    }

    pub fn get(&self, signature: &str) -> Option<MutantStatus> {
        self.entries.get(signature).copied()
    }

    pub fn set(&mut self, signature: String, status: MutantStatus) {
        self.entries.insert(signature, status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtr_types::{MutantId, Span};

    fn mutant(operator: &str, span: (u32, u32), replacement: &str) -> Mutant {
        Mutant {
            id: MutantId(0),
            operator: operator.to_string(),
            span: Span { start: span.0, end: span.1 },
            enclosing_span: Span { start: span.0, end: span.1 },
            original: String::new(),
            replacement: replacement.to_string(),
        }
    }

    #[test]
    fn signature_is_stable_when_unrelated_code_shifts_offsets() {
        let before = "const a = 1;\nconst x = alpha + beta;";
        let after = "const a = 1;\nconst extra = 2;\nconst x = alpha + beta;";

        let needle = "alpha + beta";
        let start_before = before.find(needle).unwrap() as u32;
        let start_after = after.find(needle).unwrap() as u32;
        assert_ne!(start_before, start_after, "offsets should genuinely differ for this test to mean anything");

        let m_before = mutant("ArithmeticOperator", (start_before, start_before + needle.len() as u32), "-");
        let m_after = mutant("ArithmeticOperator", (start_after, start_after + needle.len() as u32), "-");

        let sig_before = mutant_signature("src/math.js", &m_before, before);
        let sig_after = mutant_signature("src/math.js", &m_after, after);
        assert_eq!(sig_before, sig_after, "same edit at a different offset should hash the same");
    }

    #[test]
    fn signature_changes_when_the_mutated_code_actually_changes() {
        let source_a = "const x = alpha + beta;";
        let source_b = "const x = alpha + gamma;";
        let m_a = mutant("ArithmeticOperator", (10, 23), "-");
        let m_b = mutant("ArithmeticOperator", (10, 23), "-");

        let sig_a = mutant_signature("src/math.js", &m_a, source_a);
        let sig_b = mutant_signature("src/math.js", &m_b, source_b);
        assert_ne!(sig_a, sig_b);
    }

    #[test]
    fn cache_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("mtr-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.json");

        let mut cache = MutantCache::load(&path);
        assert_eq!(cache.get("abc"), None);
        cache.set("abc".to_string(), MutantStatus::Killed);
        cache.save(&path).unwrap();

        let reloaded = MutantCache::load(&path);
        assert_eq!(reloaded.get("abc"), Some(MutantStatus::Killed));

        std::fs::remove_dir_all(&dir).ok();
    }
}
