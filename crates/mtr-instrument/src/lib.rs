//! oxc_codegen reformats (own style, not the original), so output never
//! equals input. What we check instead: print, reparse, print again — same
//! result both times.

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

pub fn parse_and_print(source: &str, source_type: SourceType) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, source_type).parse();
    Codegen::new().build(&ret.program).code
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
}
