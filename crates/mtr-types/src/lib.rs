use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct MutantId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mutant {
    pub id: MutantId,
    pub operator: String,
    /// Exact span to replace for a direct text substitution.
    pub span: Span,
    /// Nearest span that's a self-contained expression, safe to wrap in a
    /// runtime `cond ? mutated : original` switch. Equal to `span`
    /// when the mutant span already is one (e.g. a boolean literal).
    pub enclosing_span: Span,
    pub original: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutantStatus {
    Killed,
    Survived,
    Timeout,
    Error,
    /// No test touched this mutant's `enclosing_span`, per coverage
    /// analysis — the test process was never even spawned for it. Reported
    /// distinctly from `Survived` (a test suite that ran and missed) for
    /// transparency, though it counts the same way in an aggregate score:
    /// an uncovered mutant is exactly as bad as one the suite missed.
    NoCoverage,
}

#[derive(Debug, Clone, Serialize)]
pub struct MutantResult {
    pub mutant: Mutant,
    pub status: MutantStatus,
}
