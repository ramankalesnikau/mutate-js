use std::collections::{HashMap, HashSet};

use mtr_types::{Mutant, MutantId, MutantKind, Span};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpression, ArrowFunctionBody, ArrowFunctionExpression, AssignmentExpression,
    BinaryExpression, BlockStatement, BooleanLiteral, CallExpression, ComputedMemberExpression,
    Comment, CommentKind, ConditionalExpression, Directive, Expression, ExportAllDeclaration,
    ExportFromDeclaration,
    Function, ImportDeclaration, ImportExpression, JSXAttribute, LogicalExpression,
    ObjectExpression, RegExpLiteral, StaticMemberExpression, StringLiteral, UnaryExpression,
    UpdateExpression,
};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::operator::{
    AssignmentOperator, BinaryOperator, LogicalOperator, UnaryOperator, UpdateOperator,
};

pub struct MutantCandidate {
    pub operator: &'static str,
    pub span: Span,
    pub enclosing_span: Span,
    pub original: String,
    pub replacement: String,
    pub kind: MutantKind,
}

impl MutantCandidate {
    /// An expression-position mutant — the common case, wrappable in a
    /// ternary by the switch instrumenter.
    fn expression(
        operator: &'static str,
        span: Span,
        enclosing_span: Span,
        original: String,
        replacement: String,
    ) -> Self {
        Self { operator, span, enclosing_span, original, replacement, kind: MutantKind::Expression }
    }
}

pub trait Mutator {
    fn name(&self) -> &'static str;
    fn discover_binary(&self, _expr: &BinaryExpression, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_logical(&self, _expr: &LogicalExpression, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_unary(&self, _expr: &UnaryExpression, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_update(&self, _expr: &UpdateExpression, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_assignment(
        &self,
        _expr: &AssignmentExpression,
        _source: &str,
    ) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_conditional(
        &self,
        _expr: &ConditionalExpression,
        _source: &str,
    ) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_boolean_literal(&self, _lit: &BooleanLiteral, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_string_literal(&self, _lit: &StringLiteral, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_array_expression(
        &self,
        _expr: &ArrayExpression,
        _source: &str,
    ) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_jsx_attribute(&self, _attr: &JSXAttribute, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_block_statement(
        &self,
        _block: &BlockStatement,
        _source: &str,
    ) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_function(&self, _func: &Function, _source: &str) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_arrow_function(
        &self,
        _func: &ArrowFunctionExpression,
        _source: &str,
    ) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_object_expression(
        &self,
        _expr: &ObjectExpression,
        _source: &str,
    ) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_static_member(
        &self,
        _expr: &StaticMemberExpression,
        _source: &str,
    ) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_computed_member(
        &self,
        _expr: &ComputedMemberExpression,
        _source: &str,
    ) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_call_expression(
        &self,
        _expr: &CallExpression,
        _source: &str,
    ) -> Vec<MutantCandidate> {
        Vec::new()
    }
    fn discover_regex_literal(
        &self,
        _lit: &RegExpLiteral,
        _source: &str,
    ) -> Vec<MutantCandidate> {
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
            kind: MutantKind::Expression,
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
            kind: MutantKind::Expression,
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
            kind: MutantKind::Expression,
        }]
    }
}

/// `LogicalExpression::span` covers the whole `left op right` expression, not
/// just the operator token — same gap-search trick as `operator_span`, which
/// only takes a `BinaryExpression`.
fn logical_operator_span(expr: &LogicalExpression, source: &str) -> Span {
    let gap_start = expr.left.span().end as usize;
    let gap_end = expr.right.span().start as usize;
    let gap = &source[gap_start..gap_end];
    let op = expr.operator.as_str();
    let offset = gap.find(op).expect("operator token must appear in the gap between operands");
    Span { start: (gap_start + offset) as u32, end: (gap_start + offset + op.len()) as u32 }
}

pub struct LogicalOperatorMutator;

impl Mutator for LogicalOperatorMutator {
    fn name(&self) -> &'static str {
        "LogicalOperator"
    }

    fn discover_logical(&self, expr: &LogicalExpression, source: &str) -> Vec<MutantCandidate> {
        let replacement = match expr.operator {
            LogicalOperator::And => "||",
            LogicalOperator::Or => "&&",
            // `??` has different short-circuit semantics (nullish, not
            // falsy) than `&&`/`||` — swapping it for either would often
            // change behavior for reasons unrelated to test quality, so it's
            // left out of the catalog rather than mutated to a lookalike.
            LogicalOperator::Coalesce => return Vec::new(),
        };
        vec![MutantCandidate {
            operator: self.name(),
            span: logical_operator_span(expr, source),
            enclosing_span: Span { start: expr.span.start, end: expr.span.end },
            original: expr.operator.as_str().to_string(),
            replacement: replacement.to_string(),
            kind: MutantKind::Expression,
        }]
    }
}

pub struct UnaryOperatorMutator;

impl Mutator for UnaryOperatorMutator {
    fn name(&self) -> &'static str {
        "UnaryOperator"
    }

    fn discover_unary(&self, expr: &UnaryExpression, _source: &str) -> Vec<MutantCandidate> {
        let replacement = match expr.operator {
            UnaryOperator::UnaryPlus => "-",
            UnaryOperator::UnaryNegation => "+",
            UnaryOperator::LogicalNot => "",
            _ => return Vec::new(),
        };
        let operator_span = Span {
            start: expr.span.start,
            end: expr.span.start + expr.operator.as_str().len() as u32,
        };
        vec![MutantCandidate {
            operator: self.name(),
            span: operator_span,
            enclosing_span: Span { start: expr.span.start, end: expr.span.end },
            original: expr.operator.as_str().to_string(),
            replacement: replacement.to_string(),
            kind: MutantKind::Expression,
        }]
    }
}

pub struct UpdateOperatorMutator;

impl Mutator for UpdateOperatorMutator {
    fn name(&self) -> &'static str {
        "UpdateOperator"
    }

    fn discover_update(&self, expr: &UpdateExpression, source: &str) -> Vec<MutantCandidate> {
        let replacement = match expr.operator {
            UpdateOperator::Increment => "--",
            UpdateOperator::Decrement => "++",
        };
        let op = expr.operator.as_str();
        let operator_span = if expr.prefix {
            Span { start: expr.span.start, end: expr.span.start + op.len() as u32 }
        } else {
            let text = &source[expr.span.start as usize..expr.span.end as usize];
            let offset = text.rfind(op).expect("operator token must appear at the end of a postfix update");
            Span {
                start: expr.span.start + offset as u32,
                end: expr.span.start + offset as u32 + op.len() as u32,
            }
        };
        vec![MutantCandidate {
            operator: self.name(),
            span: operator_span,
            enclosing_span: Span { start: expr.span.start, end: expr.span.end },
            original: op.to_string(),
            replacement: replacement.to_string(),
            kind: MutantKind::Expression,
        }]
    }
}

pub struct AssignmentOperatorMutator;

impl Mutator for AssignmentOperatorMutator {
    fn name(&self) -> &'static str {
        "AssignmentOperator"
    }

    fn discover_assignment(&self, expr: &AssignmentExpression, source: &str) -> Vec<MutantCandidate> {
        let replacement = match expr.operator {
            AssignmentOperator::Addition => "-=",
            AssignmentOperator::Subtraction => "+=",
            AssignmentOperator::Multiplication => "/=",
            AssignmentOperator::Division => "*=",
            _ => return Vec::new(),
        };
        let gap_start = expr.left.span().end as usize;
        let gap_end = expr.right.span().start as usize;
        let gap = &source[gap_start..gap_end];
        let op = expr.operator.as_str();
        let offset = gap.find(op).expect("operator token must appear in the gap between operands");
        let operator_span =
            Span { start: (gap_start + offset) as u32, end: (gap_start + offset + op.len()) as u32 };
        vec![MutantCandidate {
            operator: self.name(),
            span: operator_span,
            enclosing_span: Span { start: expr.span.start, end: expr.span.end },
            original: op.to_string(),
            replacement: replacement.to_string(),
            kind: MutantKind::Expression,
        }]
    }
}

pub struct ConditionalExpressionMutator;

impl Mutator for ConditionalExpressionMutator {
    fn name(&self) -> &'static str {
        "ConditionalExpression"
    }

    /// Swaps the ternary's branches: `test ? a : b` becomes `test ? b : a`.
    /// Leaves `test` itself alone — that's covered by whatever operator
    /// mutants the condition's own expression already produces.
    fn discover_conditional(&self, expr: &ConditionalExpression, source: &str) -> Vec<MutantCandidate> {
        let consequent_span = expr.consequent.span();
        let alternate_span = expr.alternate.span();
        let consequent_text =
            &source[consequent_span.start as usize..consequent_span.end as usize];
        let alternate_text = &source[alternate_span.start as usize..alternate_span.end as usize];
        vec![MutantCandidate {
            operator: self.name(),
            span: Span { start: consequent_span.start, end: alternate_span.end },
            enclosing_span: Span { start: expr.span.start, end: expr.span.end },
            original: format!("{consequent_text} : {alternate_text}"),
            replacement: format!("{alternate_text} : {consequent_text}"),
            kind: MutantKind::Expression,
        }]
    }
}

/// Replaces a string literal's contents: an empty string becomes a
/// placeholder, any non-empty string becomes empty. Doesn't visit module
/// specifiers (`import`/`export ... from "..."`, a bare `require("...")`
/// call's own argument), dynamic `import()` sources, or directive prologues
/// (`"use strict"`) — those strings aren't data, they're syntax, and
/// mutating them breaks the module rather than testing anything.
/// `MutantScanner` enforces this by never routing those nodes' string
/// literals through `discover_string_literal`.
pub struct StringLiteralMutator;

impl Mutator for StringLiteralMutator {
    fn name(&self) -> &'static str {
        "StringLiteral"
    }

    fn discover_string_literal(&self, lit: &StringLiteral, source: &str) -> Vec<MutantCandidate> {
        let text = &source[lit.span.start as usize..lit.span.end as usize];
        let quote = text.chars().next().unwrap_or('"');
        let replacement = if lit.value.as_str().is_empty() {
            format!("{quote}Stryker was here!{quote}")
        } else {
            format!("{quote}{quote}")
        };
        vec![MutantCandidate {
            operator: self.name(),
            span: Span { start: lit.span.start, end: lit.span.end },
            enclosing_span: Span { start: lit.span.start, end: lit.span.end },
            original: text.to_string(),
            replacement,
            kind: MutantKind::Expression,
        }]
    }
}

/// Empties an array literal: `[1, 2, 3]` becomes `[]`. Skipped when already
/// empty (nothing to mutate) or containing a spread element (`[...xs]`) —
/// clearing it would also delete whatever `xs` contributes, which is a much
/// bigger behavior change than the operator intends to model.
pub struct ArrayDeclarationMutator;

impl Mutator for ArrayDeclarationMutator {
    fn name(&self) -> &'static str {
        "ArrayDeclaration"
    }

    fn discover_array_expression(&self, expr: &ArrayExpression, source: &str) -> Vec<MutantCandidate> {
        if expr.elements.is_empty() {
            return Vec::new();
        }
        if expr.elements.iter().any(|e| matches!(e, oxc_ast::ast::ArrayExpressionElement::SpreadElement(_)))
        {
            return Vec::new();
        }
        let text = &source[expr.span.start as usize..expr.span.end as usize];
        vec![MutantCandidate {
            operator: self.name(),
            span: Span { start: expr.span.start, end: expr.span.end },
            enclosing_span: Span { start: expr.span.start, end: expr.span.end },
            original: text.to_string(),
            replacement: "[]".to_string(),
            kind: MutantKind::Expression,
        }]
    }
}

/// Empties a standalone block statement: `{ ... }` becomes `{}`. This is a
/// block-position mutant (`MutantKind::Block`) — the instrumenter wraps it in
/// `if/else`, not a ternary. Function/method/arrow bodies are `FunctionBody`
/// nodes, not `BlockStatement`, and are covered by `MethodExpressionMutator`.
pub struct BlockStatementMutator;

impl Mutator for BlockStatementMutator {
    fn name(&self) -> &'static str {
        "BlockStatement"
    }

    fn discover_block_statement(&self, block: &BlockStatement, source: &str) -> Vec<MutantCandidate> {
        if block.body.is_empty() {
            return Vec::new();
        }
        let span = Span { start: block.span.start, end: block.span.end };
        let text = &source[span.start as usize..span.end as usize];
        vec![MutantCandidate {
            operator: self.name(),
            span,
            enclosing_span: span,
            original: text.to_string(),
            replacement: "{}".to_string(),
            kind: MutantKind::Block,
        }]
    }
}

/// Replaces a function/method/arrow body with an empty block. `Function`
/// covers function declarations, function expressions, and class/object
/// method bodies (the scanner visits methods' `Function` through
/// `walk_method_definition`); `ArrowFunctionExpression` is a separate node in
/// oxc. Bodies with no statements (or none at all, e.g. overload signatures)
/// are skipped — there is nothing to empty.
pub struct MethodExpressionMutator;

impl MethodExpressionMutator {
    fn from_body(body_span: Span, source: &str) -> Vec<MutantCandidate> {
        let text = &source[body_span.start as usize..body_span.end as usize];
        vec![MutantCandidate {
            operator: "MethodExpression",
            span: body_span,
            enclosing_span: body_span,
            original: text.to_string(),
            replacement: "{}".to_string(),
            kind: MutantKind::Block,
        }]
    }
}

impl Mutator for MethodExpressionMutator {
    fn name(&self) -> &'static str {
        "MethodExpression"
    }

    fn discover_function(&self, func: &Function, source: &str) -> Vec<MutantCandidate> {
        let Some(body) = &func.body else { return Vec::new() };
        if body.statements.is_empty() {
            return Vec::new();
        }
        Self::from_body(Span { start: body.span.start, end: body.span.end }, source)
    }

    fn discover_arrow_function(
        &self,
        func: &ArrowFunctionExpression,
        source: &str,
    ) -> Vec<MutantCandidate> {
        let ArrowFunctionBody::FunctionBody(body) = &func.body else { return Vec::new() };
        if body.statements.is_empty() {
            return Vec::new();
        }
        Self::from_body(Span { start: body.span.start, end: body.span.end }, source)
    }
}

/// Empties an object literal: `{ a: 1, b: 2 }` becomes `{}`.
pub struct ObjectLiteralMutator;

impl Mutator for ObjectLiteralMutator {
    fn name(&self) -> &'static str {
        "ObjectLiteral"
    }

    fn discover_object_expression(
        &self,
        expr: &ObjectExpression,
        source: &str,
    ) -> Vec<MutantCandidate> {
        if expr.properties.is_empty() {
            return Vec::new();
        }
        let span = Span { start: expr.span.start, end: expr.span.end };
        let text = &source[span.start as usize..span.end as usize];
        vec![MutantCandidate::expression(
            self.name(),
            span,
            span,
            text.to_string(),
            "{}".to_string(),
        )]
    }
}

/// Finds the `?.` token that makes a member/call optional. `from` is the end
/// of the receiver (object/callee) and `to` the end of the whole expression;
/// the optional token sits in that gap.
fn optional_chain_span(from: u32, to: u32, source: &str) -> Option<Span> {
    let gap = &source[from as usize..to as usize];
    let offset = gap.find("?.")?;
    Some(Span { start: from + offset as u32, end: from + offset as u32 + 2 })
}

/// Removes optional chaining: `a?.b` becomes `a.b`, `f?.()` becomes `f()`.
/// Each `?.` in a chain is its own mutant.
pub struct OptionalChainingMutator;

impl Mutator for OptionalChainingMutator {
    fn name(&self) -> &'static str {
        "OptionalChaining"
    }

    fn discover_static_member(
        &self,
        expr: &StaticMemberExpression,
        source: &str,
    ) -> Vec<MutantCandidate> {
        if !expr.optional {
            return Vec::new();
        }
        Self::candidate(
            Span { start: expr.span.start, end: expr.span.end },
            expr.object.span().end,
            expr.span.end,
            source,
            ".",
        )
    }

    fn discover_computed_member(
        &self,
        expr: &ComputedMemberExpression,
        source: &str,
    ) -> Vec<MutantCandidate> {
        if !expr.optional {
            return Vec::new();
        }
        Self::candidate(
            Span { start: expr.span.start, end: expr.span.end },
            expr.object.span().end,
            expr.span.end,
            source,
            "",
        )
    }

    fn discover_call_expression(
        &self,
        expr: &CallExpression,
        source: &str,
    ) -> Vec<MutantCandidate> {
        if !expr.optional {
            return Vec::new();
        }
        Self::candidate(
            Span { start: expr.span.start, end: expr.span.end },
            expr.callee.span().end,
            expr.span.end,
            source,
            "",
        )
    }
}

impl OptionalChainingMutator {
    /// `replacement` is "." for `a?.b` (the `.` is part of the removed token)
    /// but "" for `a?.[k]` / `a?.()` (the `[` or `(` that stays is the
    /// separator) — otherwise the result would be `a.b` vs invalid `a.[k]`.
    fn candidate(
        expr_span: Span,
        from: u32,
        to: u32,
        source: &str,
        replacement: &str,
    ) -> Vec<MutantCandidate> {
        let Some(span) = optional_chain_span(from, to, source) else { return Vec::new() };
        vec![MutantCandidate::expression(
            "OptionalChaining",
            span,
            expr_span,
            "?.".to_string(),
            replacement.to_string(),
        )]
    }
}

/// Replaces a regular expression literal with `/x/`.
pub struct RegexMutator;

impl Mutator for RegexMutator {
    fn name(&self) -> &'static str {
        "Regex"
    }

    fn discover_regex_literal(&self, lit: &RegExpLiteral, source: &str) -> Vec<MutantCandidate> {
        let span = Span { start: lit.span.start, end: lit.span.end };
        let text = &source[span.start as usize..span.end as usize];
        if lit.regex.pattern.text.is_empty() || text == "/x/" {
            return Vec::new();
        }
        vec![MutantCandidate::expression(
            self.name(),
            span,
            span,
            text.to_string(),
            "/x/".to_string(),
        )]
    }
}

/// The core, framework-agnostic operator catalog.
pub fn default_mutators() -> Vec<Box<dyn Mutator>> {
    vec![
        Box::new(ArithmeticOperatorMutator),
        Box::new(EqualityOperatorMutator),
        Box::new(BooleanLiteralMutator),
        Box::new(LogicalOperatorMutator),
        Box::new(UnaryOperatorMutator),
        Box::new(UpdateOperatorMutator),
        Box::new(AssignmentOperatorMutator),
        Box::new(ConditionalExpressionMutator),
        Box::new(StringLiteralMutator),
        Box::new(ArrayDeclarationMutator),
        Box::new(BlockStatementMutator),
        Box::new(MethodExpressionMutator),
        Box::new(ObjectLiteralMutator),
        Box::new(OptionalChainingMutator),
        Box::new(RegexMutator),
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
                kind: c.kind,
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

    fn visit_logical_expression(&mut self, it: &LogicalExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_logical(it, self.source);
            self.push(candidates);
        }
        walk::walk_logical_expression(self, it);
    }

    fn visit_unary_expression(&mut self, it: &UnaryExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_unary(it, self.source);
            self.push(candidates);
        }
        walk::walk_unary_expression(self, it);
    }

    fn visit_update_expression(&mut self, it: &UpdateExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_update(it, self.source);
            self.push(candidates);
        }
        walk::walk_update_expression(self, it);
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_assignment(it, self.source);
            self.push(candidates);
        }
        walk::walk_assignment_expression(self, it);
    }

    fn visit_conditional_expression(&mut self, it: &ConditionalExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_conditional(it, self.source);
            self.push(candidates);
        }
        walk::walk_conditional_expression(self, it);
    }

    fn visit_boolean_literal(&mut self, it: &BooleanLiteral) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_boolean_literal(it, self.source);
            self.push(candidates);
        }
    }

    fn visit_string_literal(&mut self, it: &StringLiteral<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_string_literal(it, self.source);
            self.push(candidates);
        }
    }

    fn visit_array_expression(&mut self, it: &ArrayExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_array_expression(it, self.source);
            self.push(candidates);
        }
        walk::walk_array_expression(self, it);
    }

    fn visit_jsx_attribute(&mut self, it: &JSXAttribute<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_jsx_attribute(it, self.source);
            self.push(candidates);
        }
        walk::walk_jsx_attribute(self, it);
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_block_statement(it, self.source);
            self.push(candidates);
        }
        walk::walk_block_statement(self, it);
    }

    fn visit_function(&mut self, it: &Function<'ast>, flags: oxc_syntax::scope::ScopeFlags) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_function(it, self.source);
            self.push(candidates);
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_arrow_function(it, self.source);
            self.push(candidates);
        }
        walk::walk_arrow_function_expression(self, it);
    }

    fn visit_object_expression(&mut self, it: &ObjectExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_object_expression(it, self.source);
            self.push(candidates);
        }
        walk::walk_object_expression(self, it);
    }

    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_static_member(it, self.source);
            self.push(candidates);
        }
        walk::walk_static_member_expression(self, it);
    }

    fn visit_computed_member_expression(&mut self, it: &ComputedMemberExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_computed_member(it, self.source);
            self.push(candidates);
        }
        walk::walk_computed_member_expression(self, it);
    }

    fn visit_reg_exp_literal(&mut self, it: &RegExpLiteral<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_regex_literal(it, self.source);
            self.push(candidates);
        }
        walk::walk_reg_exp_literal(self, it);
    }

    // The overrides below walk everything a module specifier's or directive's
    // string literal node normally would, except the call into
    // `visit_string_literal` itself — those strings are syntax (what to
    // import/export, "use strict"), not data, so `StringLiteralMutator` never
    // sees them.
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'ast>) {
        if let Some(specifiers) = &it.specifiers {
            self.visit_import_declaration_specifiers(specifiers);
        }
        if let Some(with_clause) = &it.with_clause {
            self.visit_with_clause(with_clause);
        }
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'ast>) {
        if let Some(exported) = &it.exported {
            self.visit_module_export_name(exported);
        }
        if let Some(with_clause) = &it.with_clause {
            self.visit_with_clause(with_clause);
        }
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'ast>) {
        self.visit_export_specifiers(&it.specifiers);
        if let Some(with_clause) = &it.with_clause {
            self.visit_with_clause(with_clause);
        }
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'ast>) {
        // `source` is an arbitrary `Expression`, not always a plain string
        // literal (`import(dynamicPath)`), but it's still a module
        // specifier — skip it rather than special-case the common
        // string-literal shape only.
        if let Some(options) = &it.options {
            self.visit_expression(options);
        }
    }

    fn visit_directive(&mut self, _it: &Directive<'ast>) {
        // `"use strict"` and friends: syntax, not a value to mutate.
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'ast>) {
        let mutators = self.mutators;
        for m in mutators {
            let candidates = m.discover_call_expression(it, self.source);
            self.push(candidates);
        }
        let Expression::Identifier(callee) = &it.callee else {
            return walk::walk_call_expression(self, it);
        };
        if callee.name.as_str() != "require" {
            return walk::walk_call_expression(self, it);
        }
        // `require("./x")`: the argument is a module specifier, syntax, not
        // data — same reasoning as `ImportDeclaration`'s `source`. Visit
        // every argument except a leading bare string literal.
        for (i, arg) in it.arguments.iter().enumerate() {
            if i == 0 && matches!(arg, Argument::StringLiteral(_)) {
                continue;
            }
            match arg {
                Argument::SpreadElement(spread) => self.visit_spread_element(spread),
                _ => self.visit_expression(arg.to_expression()),
            }
        }
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

    let directives = IgnoreDirectives::from_comments(source, &ret.program.comments);
    let mut mutants = scanner.mutants;
    mutants.retain(|m| !directives.suppressed(line_at(source, m.span.start), &m.operator));
    mutants
}

fn line_at(source: &str, offset: u32) -> u32 {
    source[..offset as usize].matches('\n').count() as u32 + 1
}

#[derive(Debug, Clone)]
enum BlockState {
    Disabled(Option<HashSet<String>>),
    Enabled,
}

/// `// mutate-js disable-line`, `disable-next-line`, `disable` / `restore`,
/// each with an optional operator-name allowlist. Parsed from the file's own
/// comments once, then queried per mutant. Directives are matched only when
/// the comment body starts with exactly `mutate-js`, so comments aimed at
/// other tools are left alone.
#[derive(Debug, Default)]
struct IgnoreDirectives {
    /// Target line -> operators to suppress (`None` = all).
    per_line: HashMap<u32, Option<HashSet<String>>>,
    /// Ordered `disable`/`restore` markers; the last one at or before a line
    /// decides that line's state.
    blocks: Vec<(u32, BlockState)>,
}

impl IgnoreDirectives {
    fn from_comments(source: &str, comments: &[Comment]) -> Self {
        let mut directives = Self::default();
        let mut ordered: Vec<&Comment> = comments.iter().collect();
        ordered.sort_by_key(|c| c.span.start);

        for comment in ordered {
            let raw = &source[comment.span.start as usize..comment.span.end as usize];
            let body = match comment.kind {
                CommentKind::Line => raw.strip_prefix("//").unwrap_or(raw),
                _ => raw
                    .strip_prefix("/*")
                    .and_then(|rest| rest.strip_suffix("*/"))
                    .unwrap_or(raw),
            };

            let mut parts = body.split_whitespace();
            if parts.next() != Some("mutate-js") {
                continue;
            }
            let Some(command) = parts.next() else { continue };
            let operators: HashSet<String> = parts.map(str::to_string).collect();
            let set = if operators.is_empty() { None } else { Some(operators) };
            let line = line_at(source, comment.span.start);

            match command {
                "disable-next-line" => directives.add_line(line + 1, set),
                "disable-line" => directives.add_line(line, set),
                "disable" => directives.blocks.push((line, BlockState::Disabled(set))),
                "restore" => directives.blocks.push((line, BlockState::Enabled)),
                // Unknown/malformed directives are ignored rather than
                // treated as "disable everything" — a typo must not silently
                // hide mutants.
                _ => {}
            }
        }
        directives
    }

    fn add_line(&mut self, line: u32, set: Option<HashSet<String>>) {
        match self.per_line.entry(line) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(set);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                // Any unscoped directive wins; otherwise union the names.
                match (entry.get_mut(), set) {
                    (slot, None) => *slot = None,
                    (Some(existing), Some(names)) => existing.extend(names),
                    (None, Some(_)) => {}
                }
            }
        }
    }

    fn suppressed(&self, line: u32, operator: &str) -> bool {
        if let Some(set) = self.per_line.get(&line) {
            if matches_operator(set, operator) {
                return true;
            }
        }
        let mut state: Option<&BlockState> = None;
        for (marker_line, block_state) in &self.blocks {
            if *marker_line <= line {
                state = Some(block_state);
            } else {
                break;
            }
        }
        matches!(state, Some(BlockState::Disabled(set)) if matches_operator(set, operator))
    }
}

fn matches_operator(set: &Option<HashSet<String>>, operator: &str) -> bool {
    match set {
        None => true,
        Some(names) => names.is_empty() || names.contains(operator),
    }
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
        // Relational operators (`<`, `>`) aren't covered by any mutator yet.
        let source = "const x = a < b; const y = a > b;";
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

    fn only<'a>(mutants: &'a [Mutant], operator: &str) -> Vec<&'a Mutant> {
        mutants.iter().filter(|m| m.operator == operator).collect()
    }

    #[test]
    fn logical_operator_swaps_and_and_or() {
        let source = "const x = a && b; const y = a || b;";
        let mutants = scan_source(source, SourceType::ts());
        let found = only(&mutants, "LogicalOperator");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].original, "&&");
        assert_eq!(found[0].replacement, "||");
        assert_eq!(found[1].original, "||");
        assert_eq!(found[1].replacement, "&&");
    }

    #[test]
    fn logical_operator_ignores_nullish_coalescing() {
        let source = "const x = a ?? b;";
        let mutants = scan_source(source, SourceType::ts());
        assert!(only(&mutants, "LogicalOperator").is_empty());
    }

    #[test]
    fn unary_operator_mutates_sign_and_negation() {
        let source = "const a = -x; const b = +x; const c = !x;";
        let mutants = scan_source(source, SourceType::ts());
        let found = only(&mutants, "UnaryOperator");
        assert_eq!(found.len(), 3);
        assert_eq!((found[0].original.as_str(), found[0].replacement.as_str()), ("-", "+"));
        assert_eq!((found[1].original.as_str(), found[1].replacement.as_str()), ("+", "-"));
        assert_eq!((found[2].original.as_str(), found[2].replacement.as_str()), ("!", ""));
    }

    #[test]
    fn update_operator_mutates_prefix_and_postfix() {
        let source = "x++; x--; ++x; --x;";
        let mutants = scan_source(source, SourceType::ts());
        let found = only(&mutants, "UpdateOperator");
        assert_eq!(found.len(), 4);
        assert_eq!(found[0].original, "++");
        assert_eq!(found[0].replacement, "--");
        assert_eq!(found[1].original, "--");
        assert_eq!(found[1].replacement, "++");
    }

    #[test]
    fn update_operator_only_touches_the_operator_in_a_postfix_expression() {
        let source = "x++;";
        let mutants = scan_source(source, SourceType::ts());
        let mutated = apply_mutant(source, &only(&mutants, "UpdateOperator")[0]);
        assert_eq!(mutated, "x--;");
    }

    #[test]
    fn assignment_operator_mutates_compound_arithmetic_assignment() {
        let source = "x += 1; x -= 1; x *= 2; x /= 2; x = 1;";
        let mutants = scan_source(source, SourceType::ts());
        let found = only(&mutants, "AssignmentOperator");
        assert_eq!(found.len(), 4, "`x = 1` (plain assign) must not be mutated: {found:#?}");
        assert_eq!(found[0].original, "+=");
        assert_eq!(found[0].replacement, "-=");
    }

    #[test]
    fn conditional_expression_swaps_branches() {
        let source = "const x = test ? a : b;";
        let mutants = scan_source(source, SourceType::ts());
        let found = only(&mutants, "ConditionalExpression");
        assert_eq!(found.len(), 1);
        let mutated = apply_mutant(source, found[0]);
        assert_eq!(mutated, "const x = test ? b : a;");
    }

    #[test]
    fn string_literal_empties_a_non_empty_string_and_fills_an_empty_one() {
        let source = r#"const a = "hello"; const b = "";"#;
        let mutants = scan_source(source, SourceType::ts());
        let found = only(&mutants, "StringLiteral");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].replacement, "\"\"");
        assert_eq!(found[1].replacement, "\"Stryker was here!\"");
    }

    #[test]
    fn string_literal_does_not_mutate_import_export_or_directive_sources() {
        let source = r#""use strict";
import { x } from "./x";
export { y } from "./y";
export * from "./z";
"#;
        let mutants = scan_source(source, SourceType::ts());
        assert!(
            only(&mutants, "StringLiteral").is_empty(),
            "module specifiers and directives must never be mutated: {mutants:#?}"
        );
    }

    #[test]
    fn string_literal_does_not_mutate_a_bare_require_specifier() {
        let source = r#"const { x } = require("./x");"#;
        let mutants = scan_source(source, SourceType::ts());
        assert!(
            only(&mutants, "StringLiteral").is_empty(),
            "require()'s module specifier must never be mutated: {mutants:#?}"
        );
    }

    #[test]
    fn string_literal_still_mutates_non_specifier_call_arguments() {
        let source = r#"log("hello"); require(computedPath);"#;
        let mutants = scan_source(source, SourceType::ts());
        assert_eq!(
            only(&mutants, "StringLiteral").len(),
            1,
            "only `require`'s own first string-literal argument is special-cased"
        );
    }

    #[test]
    fn array_declaration_empties_a_non_empty_array() {
        let source = "const xs = [1, 2, 3];";
        let mutants = scan_source(source, SourceType::ts());
        let found = only(&mutants, "ArrayDeclaration");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].replacement, "[]");
    }

    #[test]
    fn array_declaration_ignores_already_empty_arrays() {
        let source = "const xs = [];";
        let mutants = scan_source(source, SourceType::ts());
        assert!(only(&mutants, "ArrayDeclaration").is_empty());
    }

    #[test]
    fn array_declaration_ignores_arrays_with_a_spread_element() {
        let source = "const xs = [...ys];";
        let mutants = scan_source(source, SourceType::ts());
        assert!(only(&mutants, "ArrayDeclaration").is_empty());
    }

    #[test]
    fn block_statement_empties_a_non_empty_block() {
        // A standalone block (not a function body — that's MethodExpression).
        let source = "if (x) { a(); b(); }";
        let mutants = scan_source(source, SourceType::ts());
        let found = only(&mutants, "BlockStatement");
        assert_eq!(found.len(), 1, "{mutants:#?}");
        assert_eq!(found[0].replacement, "{}");
        assert_eq!(found[0].kind, mtr_types::MutantKind::Block);
    }

    #[test]
    fn block_statement_ignores_an_empty_block() {
        let mutants = scan_source("if (x) {}", SourceType::ts());
        assert!(only(&mutants, "BlockStatement").is_empty());
    }

    #[test]
    fn method_expression_empties_function_method_and_arrow_bodies() {
        let source = r#"
function f() { return 1; }
class C { m() { return 2; } }
const g = () => { return 3; };
const h = () => 4;
"#;
        let mutants = scan_source(source, SourceType::ts());
        let found = only(&mutants, "MethodExpression");
        assert_eq!(found.len(), 3, "f, C.m and g have block bodies; h is expression-bodied: {mutants:#?}");
        assert!(found.iter().all(|m| m.replacement == "{}"));
    }

    #[test]
    fn object_literal_empties_a_non_empty_object() {
        let mutants = scan_source("const o = { a: 1, b: 2 };", SourceType::ts());
        let found = only(&mutants, "ObjectLiteral");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].replacement, "{}");
    }

    #[test]
    fn object_literal_ignores_an_empty_object() {
        let mutants = scan_source("const o = {};", SourceType::ts());
        assert!(only(&mutants, "ObjectLiteral").is_empty());
    }

    #[test]
    fn optional_chaining_removes_the_question_mark_dot() {
        let source = "const a = obj?.prop;";
        let mutants = scan_source(source, SourceType::ts());
        let found = only(&mutants, "OptionalChaining");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].original, "?.");
        // The `.` is part of the replaced token, so `a?.b` becomes `a.b`.
        assert_eq!(found[0].replacement, ".");
        assert_eq!(apply_mutant(source, found[0]), "const a = obj.prop;");
    }

    #[test]
    fn optional_chaining_handles_computed_access_and_calls() {
        let source = "obj?.[k]; fn?.();";
        let mutants = scan_source(source, SourceType::ts());
        assert_eq!(only(&mutants, "OptionalChaining").len(), 2);
    }

    #[test]
    fn optional_chaining_ignores_plain_member_access() {
        let mutants = scan_source("obj.prop;", SourceType::ts());
        assert!(only(&mutants, "OptionalChaining").is_empty());
    }

    #[test]
    fn regex_literal_is_replaced_with_x() {
        let source = r#"const re = /ab+c/g;"#;
        let mutants = scan_source(source, SourceType::mjs());
        let found = only(&mutants, "Regex");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].replacement, "/x/");
        assert_eq!(apply_mutant(source, found[0]), "const re = /x/;");
    }

    #[test]
    fn regex_literal_ignores_an_already_mutated_regex() {
        let mutants = scan_source("const re = /x/;", SourceType::mjs());
        assert!(only(&mutants, "Regex").is_empty());
    }

    #[test]
    fn disable_next_line_suppresses_the_following_line_only() {
        let source = "const a = 1 + 2;\n// mutate-js disable-next-line\nconst b = 1 + 2;\nconst c = 1 + 2;\n";
        let mutants = scan_source(source, SourceType::ts());
        assert_eq!(only(&mutants, "ArithmeticOperator").len(), 2, "{mutants:#?}");
    }

    #[test]
    fn disable_line_suppresses_mutants_on_the_same_line() {
        let source = "const a = 1 + 2; // mutate-js disable-line\nconst b = 1 + 2;\n";
        let mutants = scan_source(source, SourceType::ts());
        assert_eq!(only(&mutants, "ArithmeticOperator").len(), 1, "{mutants:#?}");
    }

    #[test]
    fn block_disable_and_restore_scope_a_region() {
        let source = "// mutate-js disable\nconst a = 1 + 2;\n// mutate-js restore\nconst b = 1 + 2;\n";
        let mutants = scan_source(source, SourceType::ts());
        assert_eq!(only(&mutants, "ArithmeticOperator").len(), 1, "{mutants:#?}");
    }

    #[test]
    fn directive_can_be_scoped_to_named_operators() {
        let source = "// mutate-js disable-next-line StringLiteral\nconst s = \"x\" + 1;\n";
        let mutants = scan_source(source, SourceType::ts());
        assert!(only(&mutants, "StringLiteral").is_empty(), "{mutants:#?}");
        assert_eq!(only(&mutants, "ArithmeticOperator").len(), 1, "{mutants:#?}");
    }

    #[test]
    fn restore_without_a_matching_disable_is_a_no_op() {
        let source = "// mutate-js restore\nconst a = 1 + 2;\n";
        let mutants = scan_source(source, SourceType::ts());
        assert_eq!(only(&mutants, "ArithmeticOperator").len(), 1);
    }

    #[test]
    fn comments_aimed_at_other_tools_are_ignored() {
        let source = "// Stryker disable-next-line\nconst a = 1 + 2;\n";
        let mutants = scan_source(source, SourceType::ts());
        assert_eq!(only(&mutants, "ArithmeticOperator").len(), 1);
    }
}
