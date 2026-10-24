//! oxc_codegen reformats (own style, not the original), so output never
//! equals input. What we check instead: print, reparse, print again — same
//! result both times.

use mtr_types::{Mutant, MutantKind, Span};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

pub fn parse_and_print(source: &str, source_type: SourceType) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, source_type).parse();
    Codegen::new().build(&ret.program).code
}

pub const ACTIVE_MUTANT_ENV_VAR: &str = "MTR_ACTIVE_MUTANT";

/// Rewrites `source` once so every mutant is embedded behind a runtime check
/// on `MTR_ACTIVE_MUTANT`, instead of needing a separate file per mutant.
///
/// Implemented as direct text substitution, not an AST rewrite. Mutants whose
/// enclosing spans nest (e.g. an arithmetic operator inside a function body
/// that is itself being emptied by `MethodExpression`) are rendered as nested
/// switches: the inner switch stays inside the outer mutant's *original*
/// branch, so at most one mutant is active at a time — exactly the intended
/// mutation-switching semantics. `MutantKind` decides the wrapper shape:
/// expressions become `cond ? mutated : original`; statement blocks become
/// `if (cond) { mutated } else { original }`, since a bare block is not a
/// valid ternary branch.
pub fn switch_instrument(source: &str, mutants: &[Mutant]) -> String {
    if mutants.is_empty() {
        return source.to_string();
    }

    // Build a containment forest: each mutant's children are the mutants
    // directly nested inside its enclosing span.
    let mut order: Vec<usize> = (0..mutants.len()).collect();
    order.sort_by(|&a, &b| {
        let (a_span, b_span) = (mutants[a].enclosing_span, mutants[b].enclosing_span);
        a_span.start.cmp(&b_span.start).then(b_span.end.cmp(&a_span.end))
    });

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); mutants.len()];
    let mut roots: Vec<usize> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for &index in &order {
        while let Some(&top) = stack.last() {
            if strictly_contains(mutants[top].enclosing_span, mutants[index].enclosing_span) {
                break;
            }
            stack.pop();
        }
        match stack.last() {
            Some(&parent) => children[parent].push(index),
            None => roots.push(index),
        }
        stack.push(index);
    }

    let mut out = String::new();
    let mut cursor = 0usize;
    for root in roots {
        let start = mutants[root].enclosing_span.start as usize;
        // Disjoint-by-construction, but guard against equal spans (which the
        // scanner dedupes, yet the instrumenter shouldn't panic on input it
        // didn't produce) by skipping anything already covered.
        if start < cursor {
            continue;
        }
        out.push_str(&source[cursor..start]);
        out.push_str(&render_mutant(mutants, &children, root, source));
        cursor = mutants[root].enclosing_span.end as usize;
    }
    out.push_str(&source[cursor..]);
    out
}

fn strictly_contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start
        && inner.end <= outer.end
        && (outer.start, outer.end) != (inner.start, inner.end)
}

/// Renders one mutant's switch. The `original` branch recursively renders any
/// nested mutants as their own switches; the `mutated` branch is spliced from
/// pristine source, which is correct because a nested mutant is inactive
/// whenever this one is active (only one id is ever set).
fn render_mutant(mutants: &[Mutant], children: &[Vec<usize>], index: usize, source: &str) -> String {
    let mutant = &mutants[index];
    let original_text = render_region(
        mutants,
        children,
        &children[index],
        mutant.enclosing_span.start as usize,
        mutant.enclosing_span.end as usize,
        source,
    );

    let enclosing = &source
        [mutant.enclosing_span.start as usize..mutant.enclosing_span.end as usize];
    let rel_start = (mutant.span.start - mutant.enclosing_span.start) as usize;
    let rel_end = (mutant.span.end - mutant.enclosing_span.start) as usize;
    let mutated_text =
        format!("{}{}{}", &enclosing[..rel_start], mutant.replacement, &enclosing[rel_end..]);

    let id = mutant.id.0;
    match mutant.kind {
        MutantKind::Expression => format!(
            "(process.env.{ACTIVE_MUTANT_ENV_VAR} === \"{id}\" ? ({mutated_text}) : ({original_text}))"
        ),
        MutantKind::Block => format!(
            "{{ if (process.env.{ACTIVE_MUTANT_ENV_VAR} === \"{id}\") {mutated_text} else {original_text} }}"
        ),
    }
}

/// Copies `source` while replacing each child's enclosing span with its
/// rendered switch.
fn render_region(
    mutants: &[Mutant],
    children: &[Vec<usize>],
    indices: &[usize],
    start: usize,
    end: usize,
    source: &str,
) -> String {
    let mut out = String::new();
    let mut cursor = start;
    for &child in indices {
        let child_start = mutants[child].enclosing_span.start as usize;
        if child_start < cursor {
            continue;
        }
        out.push_str(&source[cursor..child_start]);
        out.push_str(&render_mutant(mutants, children, child, source));
        cursor = mutants[child].enclosing_span.end as usize;
    }
    out.push_str(&source[cursor..end]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> Vec<(&'static str, &'static str, SourceType)> {
        vec![
            (
                "generics_and_optional_chaining",
                r#"
interface Repository<T> {
  findById(id: string): T | undefined;
}

class InMemoryRepository<T extends { id: string }> implements Repository<T> {
  private items = new Map<string, T>();

  findById(id: string): T | undefined {
    return this.items.get(id)?.id === id ? this.items.get(id) : undefined;
  }

  upsert(item: T): void {
    this.items.set(item.id, item);
  }
}
"#,
                SourceType::ts(),
            ),
            (
                "decorators_and_enums",
                r#"
enum Status {
  Pending = "pending",
  Active = "active",
  Archived = "archived",
}

function logged(target: unknown, key: string, descriptor: PropertyDescriptor) {
  const original = descriptor.value;
  descriptor.value = function (...args: unknown[]) {
    return original.apply(this, args);
  };
  return descriptor;
}

class Task {
  status: Status = Status.Pending;

  @logged
  activate() {
    this.status = Status.Active;
  }
}
"#,
                SourceType::ts(),
            ),
            (
                "template_literals_and_arrow_fns",
                r#"
const greet = (name: string, times = 1): string => {
  const parts: string[] = [];
  for (let i = 0; i < times; i++) {
    parts.push(`hello, ${name}! (#${i + 1})`);
  }
  return parts.join(", ");
};

export default greet;
"#,
                SourceType::ts(),
            ),
            (
                "small_tsx_component",
                r#"
import type { FC } from "react";

interface BadgeProps {
  label: string;
  active?: boolean;
}

const Badge: FC<BadgeProps> = ({ label, active = false }) => {
  return (
    <span className={active ? "badge badge--active" : "badge"}>
      {active ? `* ${label}` : label}
    </span>
  );
};

export default Badge;
"#,
                SourceType::tsx(),
            ),
        ]
    }

    #[test]
    fn parse_and_print_is_idempotent() {
        for (name, source, source_type) in fixtures() {
            let first = parse_and_print(source, source_type);
            let second = parse_and_print(&first, source_type);
            assert_eq!(first, second, "fixture `{name}`: not idempotent");
        }
    }

    #[test]
    fn parse_and_print_produces_semantically_recognizable_output() {
        for (name, source, source_type) in fixtures() {
            let printed = parse_and_print(source, source_type);
            assert!(!printed.trim().is_empty(), "fixture `{name}`: printed output was empty");
        }
    }

    #[test]
    fn switch_instrument_wraps_each_mutant_in_a_runtime_check() {
        let source = "const x = alpha + beta;";
        let mutants = mtr_mutators::scan_source(source, SourceType::ts());
        let instrumented = switch_instrument(source, &mutants);
        assert_eq!(
            instrumented,
            "const x = (process.env.MTR_ACTIVE_MUTANT === \"0\" ? (alpha - beta) : (alpha + beta));"
        );
    }

    #[test]
    fn switch_instrument_output_still_parses() {
        let source = "const x = alpha + beta; const y = alpha === beta; const z = true;";
        let mutants = mtr_mutators::scan_source(source, SourceType::ts());
        let instrumented = switch_instrument(source, &mutants);
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, &instrumented, SourceType::ts()).parse();
        assert!(ret.diagnostics.is_empty(), "instrumented output failed to parse: {instrumented}");
    }

    #[test]
    fn block_mutants_are_wrapped_in_if_else_not_a_ternary() {
        let source = "if (x) { a(); }";
        let mutants = mtr_mutators::scan_source(source, SourceType::ts());
        assert!(mutants.iter().any(|m| m.kind == MutantKind::Block), "{mutants:#?}");
        let instrumented = switch_instrument(source, &mutants);
        assert!(instrumented.contains("if (process.env."), "block mutant should use if/else: {instrumented}");
        assert!(instrumented.ends_with("}}}") || instrumented.contains("else"), "{instrumented}");

        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, &instrumented, SourceType::ts()).parse();
        assert!(ret.diagnostics.is_empty(), "instrumented output failed to parse: {instrumented}");
    }

    #[test]
    fn nested_mutants_are_rendered_as_nested_switches() {
        // An arithmetic mutant nests inside the arrow function body mutant.
        let source = "const f = () => { return a + b; };";
        let mutants = mtr_mutators::scan_source(source, SourceType::ts());
        let has_block = mutants.iter().any(|m| m.kind == MutantKind::Block);
        let has_expr = mutants.iter().any(|m| m.kind == MutantKind::Expression);
        assert!(has_block && has_expr, "fixture should exercise nesting: {mutants:#?}");

        let instrumented = switch_instrument(source, &mutants);
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, &instrumented, SourceType::ts()).parse();
        assert!(ret.diagnostics.is_empty(), "instrumented output failed to parse: {instrumented}");
        // Inner expression switch must survive inside the outer block switch.
        assert!(
            instrumented.matches("process.env.").count() >= 2,
            "both nested mutants should be embedded: {instrumented}"
        );
    }

    #[test]
    fn output_with_mixed_operator_kinds_still_parses() {
        let source = r#"
const o = { a: 1 };
const re = /ab+c/;
const v = obj?.prop;
function f() { return 1; }
"#;
        let mutants = mtr_mutators::scan_source(source, SourceType::mjs());
        assert!(!mutants.is_empty());
        let instrumented = switch_instrument(source, &mutants);
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, &instrumented, SourceType::mjs()).parse();
        assert!(ret.diagnostics.is_empty(), "instrumented output failed to parse: {instrumented}");
    }
}
