use mtr_mutators::{Mutator, MutantCandidate};
use mtr_types::Span;
use oxc_ast::ast::{JSXAttribute, JSXAttributeValue, JSXExpression};

/// Flips a JSX attribute's boolean value: `<Foo disabled={true} />` becomes
/// `<Foo disabled={false} />`. Doesn't touch shorthand boolean attributes
/// (`<Foo disabled />`, no `={...}` at all) or string-valued attributes.
pub struct JsxBooleanAttributeMutator;

impl Mutator for JsxBooleanAttributeMutator {
    fn name(&self) -> &'static str {
        "JsxBooleanAttribute"
    }

    fn discover_jsx_attribute(&self, attr: &JSXAttribute, _source: &str) -> Vec<MutantCandidate> {
        let Some(JSXAttributeValue::ExpressionContainer(container)) = &attr.value else {
            return Vec::new();
        };
        let JSXExpression::BooleanLiteral(lit) = &container.expression else {
            return Vec::new();
        };

        vec![MutantCandidate {
            operator: self.name(),
            span: Span { start: lit.span.start, end: lit.span.end },
            enclosing_span: Span { start: lit.span.start, end: lit.span.end },
            original: lit.value.to_string(),
            replacement: (!lit.value).to_string(),
        }]
    }
}

/// This crate's own operators only — callers combine with
/// `mtr_mutators::default_mutators()` (or other packs) themselves, so
/// stacking multiple packs together never double-registers the core catalog.
pub fn mutators() -> Vec<Box<dyn Mutator>> {
    vec![Box::new(JsxBooleanAttributeMutator)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_span::SourceType;

    fn combined_mutators() -> Vec<Box<dyn Mutator>> {
        let mut all = mtr_mutators::default_mutators();
        all.extend(mutators());
        all
    }

    #[test]
    fn flips_jsx_boolean_attribute() {
        let source = r#"
const Badge = ({ active }: { active: boolean }) => (
  <span disabled={true} highlighted={active}>text</span>
);
"#;
        let mutants = mtr_mutators::scan_source_with_mutators(
            source,
            SourceType::tsx(),
            &combined_mutators(),
        );

        let jsx_mutants: Vec<_> =
            mutants.iter().filter(|m| m.operator == "JsxBooleanAttribute").collect();
        assert_eq!(jsx_mutants.len(), 1, "only the literal `true`, not `active`, should mutate");
        assert_eq!(jsx_mutants[0].original, "true");
        assert_eq!(jsx_mutants[0].replacement, "false");
    }

    #[test]
    fn ignores_shorthand_and_string_attributes() {
        let source = r#"
const Badge = () => <span disabled title="hello">text</span>;
"#;
        let mutants = mtr_mutators::scan_source_with_mutators(
            source,
            SourceType::tsx(),
            &combined_mutators(),
        );
        assert!(mutants.iter().all(|m| m.operator != "JsxBooleanAttribute"));
    }

    #[test]
    fn stacks_alongside_the_core_catalog_without_duplication() {
        let source = "const x = a + b;";
        let mutants = mtr_mutators::scan_source_with_mutators(
            source,
            SourceType::ts(),
            &combined_mutators(),
        );
        assert_eq!(mutants.len(), 1, "core catalog must not be double-registered");
        assert_eq!(mutants[0].operator, "ArithmeticOperator");
    }

    #[test]
    fn does_not_double_count_a_boolean_literal_the_core_catalog_already_found() {
        // `true` here is caught by both the core BooleanLiteralMutator (any
        // boolean literal) and JsxBooleanAttributeMutator (specifically a
        // JSX attribute value) — same span, same edit, should collapse to one.
        let source = r#"const el = <span disabled={true} />;"#;
        let mutants = mtr_mutators::scan_source_with_mutators(
            source,
            SourceType::tsx(),
            &combined_mutators(),
        );
        assert_eq!(
            mutants.len(),
            1,
            "identical (span, replacement) from two different mutators should dedupe, got {mutants:#?}"
        );
    }
}
