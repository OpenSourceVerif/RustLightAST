use super::{ExpressionPosition, RustCodeGenerator};
use crate::{BindingMode, Member, Pattern, PatternField, PatternPath};

impl RustCodeGenerator {
    /// Render a standalone Rust pattern.
    pub fn generate_pattern_code(&mut self, pattern: &Pattern) -> String {
        self.buffer.clear();
        self.generate_pattern(pattern);
        self.buffer.clone()
    }

    fn generate_pattern_path(&mut self, path: &PatternPath) {
        let segments =
            Self::namespace_segments_with_arguments(&path.segments, &path.arguments, true);
        self.generate_path(&segments);
    }

    pub(super) fn generate_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Ident {
                name,
                mode,
                subpattern,
            } => {
                self.write(match mode {
                    BindingMode::Value => "",
                    BindingMode::Mut => "mut ",
                    BindingMode::Ref => "ref ",
                    BindingMode::RefMut => "ref mut ",
                });
                self.write(name);
                if let Some(subpattern) = subpattern {
                    self.write(" @ ");
                    self.generate_pattern_operand(subpattern);
                }
            }
            Pattern::Path(path) => self.generate_pattern_path(path),
            Pattern::Literal(literal) => self.generate_literal(literal),
            Pattern::Range { start, end } => {
                self.generate_range(start.as_deref(), end, ExpressionPosition::Free);
            }
            Pattern::Opaque(source) => self.write(source),
            Pattern::Wildcard => self.write("_"),
            Pattern::Rest => self.write(".."),
            Pattern::Tuple(patterns) => {
                self.write("(");
                self.generate_pattern_list(patterns, ", ");
                if patterns.len() == 1 {
                    self.write(",");
                }
                self.write(")");
            }
            Pattern::TupleStruct { path, fields } => {
                self.generate_pattern_path(path);
                self.write("(");
                self.generate_pattern_list(fields, ", ");
                self.write(")");
            }
            Pattern::Record { path, fields, rest } => {
                self.generate_pattern_path(path);
                self.write(" {");
                for (index, field) in fields.iter().enumerate() {
                    self.write(if index == 0 { " " } else { ", " });
                    self.generate_pattern_field(field);
                }
                if *rest {
                    self.write(if fields.is_empty() { " .." } else { ", .." });
                }
                self.write(if fields.is_empty() && !rest {
                    "}"
                } else {
                    " }"
                });
            }
            Pattern::Or(alternatives) => self.generate_pattern_list(alternatives, " | "),
            Pattern::Slice(patterns) => {
                self.write("[");
                self.generate_pattern_list(patterns, ", ");
                self.write("]");
            }
            Pattern::Typed { pattern, ty } => {
                self.generate_pattern_operand(pattern);
                self.write(": ");
                self.write(&self.type_to_string(ty));
            }
            Pattern::Reference { pattern, mutable } => {
                self.write(if *mutable { "&mut " } else { "&" });
                self.generate_pattern_operand(pattern);
            }
            Pattern::Parenthesized(pattern) => {
                self.write("(");
                self.generate_pattern(pattern);
                self.write(")");
            }
        }
    }

    fn generate_pattern_list(&mut self, patterns: &[Pattern], separator: &str) {
        for (index, pattern) in patterns.iter().enumerate() {
            if index > 0 {
                self.write(separator);
            }
            self.generate_pattern(pattern);
        }
    }

    fn generate_pattern_operand(&mut self, pattern: &Pattern) {
        if matches!(pattern, Pattern::Or(_) | Pattern::Range { .. }) {
            self.write("(");
            self.generate_pattern(pattern);
            self.write(")");
        } else {
            self.generate_pattern(pattern);
        }
    }

    fn generate_pattern_field(&mut self, field: &PatternField) {
        let shorthand = field.shorthand
            && matches!(
                (&field.member, &field.pattern),
                (Member::Named(member), Pattern::Ident { name, subpattern: None, .. }) if member == name
            );
        if !shorthand {
            self.generate_member(&field.member);
            self.write(": ");
        }
        self.generate_pattern(&field.pattern);
    }
}
