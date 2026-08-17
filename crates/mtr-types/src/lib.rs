use serde::Serialize;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MutantStatus {
    Killed,
    Survived,
    Timeout,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct MutantResult {
    pub mutant: Mutant,
    pub status: MutantStatus,
}
