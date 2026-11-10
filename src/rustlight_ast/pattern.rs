use super::{Expr, Literal, Member, RangeEnd, Type};

/// A Rust pattern used by bindings, parameters, and match arms.
#[derive(Debug, Clone)]
pub enum Pattern {
    /// A bare identifier may name a binding or a constant. Resolution decides
    /// which it denotes when no binding modifier or subpattern is present.
    Ident {
        name: String,
        mode: BindingMode,
        subpattern: Option<Box<Pattern>>,
    },
    Path(Vec<String>),
    Literal(Literal),
    Range {
        start: Option<Box<Expr>>,
        end: RangeEnd,
    },
    /// Source retained when a supported expression contains an unmodeled pattern.
    Opaque(String),
    Wildcard,
    Rest,
    Tuple(Vec<Pattern>),
    TupleStruct {
        path: Vec<String>,
        fields: Vec<Pattern>,
    },
    Record {
        path: Vec<String>,
        fields: Vec<PatternField>,
        rest: bool,
    },
    Or(Vec<Pattern>),
    Slice(Vec<Pattern>),
    Typed {
        pattern: Box<Pattern>,
        ty: Type,
    },
    Reference {
        pattern: Box<Pattern>,
        mutable: bool,
    },
    Parenthesized(Box<Pattern>),
}

/// The binding modifiers written on an identifier pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingMode {
    Value,
    Mut,
    Ref,
    RefMut,
}

#[derive(Debug, Clone)]
pub struct PatternField {
    pub member: Member,
    pub pattern: Pattern,
    /// Request shorthand if the pattern still binds the same-named field.
    pub shorthand: bool,
}
