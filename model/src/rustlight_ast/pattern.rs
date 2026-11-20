use super::{Expr, Literal, Member, PathType, PathTypeArguments, RangeEnd, Type};

/// A Rust pattern used by bindings, parameters, and match arms.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// A bare identifier may name a binding or a constant. Resolution decides
    /// which it denotes when no binding modifier or subpattern is present.
    Ident {
        name: String,
        mode: BindingMode,
        subpattern: Option<Box<Pattern>>,
    },
    Path(PatternPath),
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
        path: PatternPath,
        fields: Vec<Pattern>,
    },
    Record {
        path: PatternPath,
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

/// A constructor or constant path with modeled generic arguments in a pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternPath {
    pub segments: Vec<String>,
    pub arguments: Vec<PathTypeArguments>,
}

impl PatternPath {
    /// Use the same namespace path when resolving an expression constructor.
    pub fn expression(&self) -> Expr {
        let kind = if self.arguments.is_empty() {
            PathType::Namespace
        } else {
            PathType::NamespaceWithTypeArguments(self.arguments.clone())
        };
        Expr::Path(self.segments.clone(), kind)
    }
}

impl From<Vec<String>> for PatternPath {
    fn from(segments: Vec<String>) -> Self {
        Self { segments, arguments: Vec::new() }
    }
}

impl Pattern {
    /// Create an ordinary identifier binding, or a wildcard for `_`.
    pub fn binding(name: impl Into<String>) -> Self {
        Self::binding_with_mode(name, BindingMode::Value)
    }

    /// Create an identifier binding with its written binding modifier.
    pub fn binding_with_mode(name: impl Into<String>, mode: BindingMode) -> Self {
        let name = name.into();
        if name == "_" && mode == BindingMode::Value {
            return Self::Wildcard;
        }
        Self::Ident {
            name,
            mode,
            subpattern: None,
        }
    }

    /// The name of an identifier binding without an `@` subpattern.
    pub fn binding_name(&self) -> Option<&str> {
        match self {
            Self::Ident {
                name,
                subpattern: None,
                ..
            } => Some(name),
            _ => None,
        }
    }

    /// The name of a plain identifier binding with no binding modifier.
    pub fn plain_binding_name(&self) -> Option<&str> {
        match self {
            Self::Ident {
                name,
                mode: BindingMode::Value,
                subpattern: None,
            } => Some(name),
            _ => None,
        }
    }

    /// The name of a simple value binding, with or without `mut`.
    pub fn value_binding_name(&self) -> Option<&str> {
        match self {
            Self::Ident {
                name,
                mode: BindingMode::Value | BindingMode::Mut,
                subpattern: None,
            } => Some(name),
            _ => None,
        }
    }
}

/// The binding modifiers written on an identifier pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingMode {
    Value,
    Mut,
    Ref,
    RefMut,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternField {
    pub member: Member,
    pub pattern: Pattern,
    /// Request shorthand if the pattern still binds the same-named field.
    pub shorthand: bool,
}
