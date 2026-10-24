use mtr_types::MutantResult;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FileMutantResults {
    pub file: String,
    pub mutants: Vec<MutantResult>,
    /// The file's unmutated source text, as it was when scanned. Carried
    /// here (rather than re-read from disk by each reporter) so a source
    /// viewer reporter — e.g. HTML — can highlight each mutant's `span` in
    /// context without depending on the file still existing at that path.
    pub source: String,
}

/// All-default no-op — implementers only override what they need, same
/// pattern already used for `Mutator`. Every parameter is plain data (no
/// closures), so a future out-of-process plugin bridge can carry the same
/// calls over a wire protocol without redesigning the trait.
pub trait Reporter {
    fn on_mutant_tested(&mut self, _file: &str, _result: &MutantResult) {}
    fn on_file_complete(&mut self, _file: &str, _source: &str, _results: &[MutantResult]) {}
    fn on_run_complete(&mut self, _files: &[FileMutantResults], _score: f64) {}
    fn wrap_up(&mut self) {}
}
