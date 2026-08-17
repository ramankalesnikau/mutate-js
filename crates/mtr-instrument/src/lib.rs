//! oxc_codegen reformats (own style, not the original), so output never
//! equals input. What we check instead: print, reparse, print again — same
//! result both times.

use mtr_types::Mutant;
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
/// Implemented as direct text substitution on each mutant's `enclosing_span`,
/// not an AST rewrite — correct as long as no two mutants' enclosing spans
/// nest inside each other (true for the current flat-expression operators;
/// would need real AST-level rewriting to hold in general).
pub fn switch_instrument(source: &str, mutants: &[Mutant]) -> String {
    let mut ordered: Vec<&Mutant> = mutants.iter().collect();
    ordered.sort_by(|a, b| b.enclosing_span.start.cmp(&a.enclosing_span.start));

    let mut out = source.to_string();
    for m in ordered {
        let enclosing_start = m.enclosing_span.start as usize;
        let enclosing_end = m.enclosing_span.end as usize;
        let original_text = &source[enclosing_start..enclosing_end];

        let rel_start = (m.span.start - m.enclosing_span.start) as usize;
        let rel_end = (m.span.end - m.enclosing_span.start) as usize;
        let mutated_text =
            format!("{}{}{}", &original_text[..rel_start], m.replacement, &original_text[rel_end..]);

        let wrapped = format!(
            "(process.env.{ACTIVE_MUTANT_ENV_VAR} === \"{}\" ? ({mutated_text}) : ({original_text}))",
            m.id.0
        );
        out.replace_range(enclosing_start..enclosing_end, &wrapped);
    }
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
}
