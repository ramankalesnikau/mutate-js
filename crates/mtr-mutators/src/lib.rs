use mtr_types::{Mutant, MutantId, Span};
use oxc_allocator::Allocator;
use oxc_ast::ast::{BinaryExpression, BooleanLiteral, JSXAttribute};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::operator::BinaryOperator;

pub struct MutantCandidate {
    pub operator: &'static str,
    pub span: Span,
    pub enclosing_span: Span,
    pub original: String,
    pub replacement: String,
}

pub trait Mutator {
    fn name(&self) -> &'static str;
    fn discover_binary(&self, _expr: &BinaryExpression, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_boolean_literal(&self, _lit: &BooleanLiteral, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_jsx_attribute(&self, _attr: &JSXAttribute, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
}

/// `BinaryExpression::span` covers the whole `left op right` expression, not
/// just the operator token, so applying a mutant means finding the operator's
/// own span within the gap between the operands.
fn operator_span(expr: &BinaryExpression, source: &str) -> Span {
    let gap_start = expr.left.span().end as usize;
    let gap_end = expr.right.span().start as usize;
    let gap = &source[gap_start..gap_end];
    let op = expr.operator.as_str();
    let offset = gap.find(op).expect("operator token must appear in the gap between operands");
    Span { start: (gap_start + offset) as u32, end: (gap_start + offset + op.len()) as u32 }
}

pub struct ArithmeticOperatorMutator;

impl Mutator for ArithmeticOperatorMutator {
    fn name(&self) -> &'static str {
        "ArithmeticOperator"
    }

    fn discover_binary(&self, expr: &BinaryExpression, source: &str) -> Vec<MutantCandidate> {
        let replacement = match expr.operator {
            BinaryOperator::Addition => "-",
            BinaryOperator::Subtraction => "+",
            BinaryOperator::Multiplication => "/",
            BinaryOperator::Division => "*",
            _ => return Vec::new(),
        };
        vec![MutantCandidate {
            operator: self.name(),
            span: operator_span(expr, source),
            enclosing_span: Span { start: expr.span.start, end: expr.span.end },
            original: expr.operator.as_str().to_string(),
            replacement: replacement.to_string(),
        }]
    }
}

pub struct EqualityOperatorMutator;

impl Mutator for EqualityOperatorMutator {
    fn name(&self) -> &'static str {
        "EqualityOperator"
    }

    fn discover_binary(&self, expr: &BinaryExpression, source: &str) -> Vec<MutantCandidate> {
        let replacement = match expr.operator {
            BinaryOperator::Equality => "!=",
            BinaryOperator::Inequality => "==",
            BinaryOperator::StrictEquality => "!==",
            BinaryOperator::StrictInequality => "===",
            _ => return Vec::new(),
        };
        vec![MutantCandidate {
            operator: self.name(),
            span: operator_span(expr, source),
            enclosing_span: Span { start: expr.span.start, end: expr.span.end },
            original: expr.operator.as_str().to_string(),
            replacement: replacement.to_string(),
        }]
    }
}

pub struct BooleanLiteralMutator;

impl Mutator for BooleanLiteralMutator {
    fn name(&self) -> &'static str {
        "BooleanLiteral"
    }

    fn discover_boolean_literal(&self, lit: &BooleanLiteral, _source: &str) -> Vec<MutantCandidate> {
        vec![MutantCandidate {
            operator: self.name(),
            span: Span { start: lit.span.start, end: lit.span.end },
            enclosing_span: Span { start: lit.span.start, end: lit.span.end },
            original: lit.value.to_string(),
            replacement: (!lit.value).to_string(),
        }]
    }
}

/// The core, framework-agnostic operator catalog.
pub fn default_mutators() -> Vec<Box<dyn Mutator>> {
    vec![
        Box::new(ArithmeticOperatorMutator),
        Box::new(EqualityOperatorMutator),
        Box::new(BooleanLiteralMutator),
    ]
}

struct MutantScanner<'m, 's> {
    mutators: &'m [Box<dyn Mutator>],
    source: &'s str,
    next_id: u32,
    mutants: Vec<Mutant>,
}

impl<'m, 's> MutantScanner<'m, 's> {
    fn new(mutators: &'m [Box<dyn Mutator>], source: &'s str) -> Self {
        Self { mutators, source, next_id: 0, mutants: Vec::new() }
    }

    fn push(&mut self, candidates: Vec<MutantCandidate>) {
        for c in candidates {
            // Different mutators (e.g. a core operator and a framework-pack
            // one) can independently match the same site and produce an
            // identical edit — dedupe here rather than coupling the core
            // catalog to framework-specific AST shapes to avoid it upstream.
            let is_duplicate = self
                .mutants
                .iter()
                .any(|m| m.span == c.span && m.replacement == c.replacement);
            if is_duplicate {
                continue;
            }
            self.mutants.push(Mutant {
                id: MutantId(self.next_id),
                operator: c.operator.to_string(),
                span: c.span,
                enclosing_span: c.enclosing_span,
                original: c.original,
                replacement: c.replacement,
            });
            self.next_id += 1;
        }
    }
}

impl<'ast, 'm, 's> Visit<'ast> for MutantScanner<'m, 's> {
    fn visit_binary_expression(&mut self, it: &BinaryExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_binary(it, self.source);
            self.push(candidates);
        }
        walk::walk_binary_expression(self, it);
    }

    fn visit_boolean_literal(&mut self, it: &BooleanLiteral) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_boolean_literal(it, self.source);
            self.push(candidates);
        }
    }

    fn visit_jsx_attribute(&mut self, it: &JSXAttribute<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_jsx_attribute(it, self.source);
            self.push(candidates);
        }
        walk::walk_jsx_attribute(self, it);
    }
}

/// Parse `source` and return every mutant the built-in operator catalog
/// finds, in source order.
pub fn scan_source(source: &str, source_type: SourceType) -> Vec<Mutant> {
    scan_source_with_mutators(source, source_type, &default_mutators())
}

/// Like [`scan_source`], but with an explicit mutator list — lets a
/// framework-extension crate (e.g. a JSX mutator pack) supply an extended
/// set instead of only ever getting the built-in catalog.
pub fn scan_source_with_mutators(
    source: &str,
    source_type: SourceType,
    mutators: &[Box<dyn Mutator>],
) -> Vec<Mutant> {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, source_type).parse();
    let mut scanner = MutantScanner::new(mutators, source);
    scanner.visit_program(&ret.program);
    scanner.mutants
}

/// Replace `mutant`'s span in `source` with its replacement text — the exact
/// text substitution that reproduces the mutant when the file is written out.
pub fn apply_mutant(source: &str, mutant: &Mutant) -> String {
    let start = mutant.span.start as usize;
    let end = mutant.span.end as usize;
    format!("{}{}{}", &source[..start], mutant.replacement, &source[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_arithmetic_equality_and_boolean_mutants() {
        let source = "const x = a + b; const y = a === b; const z = true;";
        let mutants = scan_source(source, SourceType::ts());

        assert_eq!(mutants.len(), 3);

        assert_eq!(mutants[0].operator, "ArithmeticOperator");
        assert_eq!(mutants[0].original, "+");
        assert_eq!(mutants[0].replacement, "-");

        assert_eq!(mutants[1].operator, "EqualityOperator");
        assert_eq!(mutants[1].original, "===");
        assert_eq!(mutants[1].replacement, "!==");

        assert_eq!(mutants[2].operator, "BooleanLiteral");
        assert_eq!(mutants[2].original, "true");
        assert_eq!(mutants[2].replacement, "false");
    }

    #[test]
    fn ignores_operators_outside_the_catalog() {
        let source = "const x = a < b && a > b;";
        let mutants = scan_source(source, SourceType::ts());
        assert!(mutants.is_empty());
    }

    #[test]
    fn assigns_stable_increasing_ids() {
        let source = "const a = true; const b = false;";
        let mutants = scan_source(source, SourceType::ts());
        assert_eq!(mutants[0].id, MutantId(0));
        assert_eq!(mutants[1].id, MutantId(1));
    }

    #[test]
    fn apply_mutant_only_replaces_the_operator_token() {
        let source = "const x = alpha + beta;";
        let mutants = scan_source(source, SourceType::ts());
        let mutated = apply_mutant(source, &mutants[0]);
        assert_eq!(mutated, "const x = alpha - beta;");
    }
}
