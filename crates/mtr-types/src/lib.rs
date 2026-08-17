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
    pub span: Span,
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
