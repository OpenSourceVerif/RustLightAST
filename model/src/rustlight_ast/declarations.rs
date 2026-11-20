use super::{
    Attribute, Expr, FunctionAbi, FunctionDef, FunctionQualifiers, Generics, MacroInvocation,
    Param, Pattern, Type, TypeBound, Visibility,
};

/// A function signature declared without a body.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDeclaration {
    pub name: String,
    pub params: Vec<DeclaredParam>,
    /// A receiver written as `self`, `&self`, or `&mut self` has no type annotation.
    /// Its type and mutability are still represented by the first parameter.
    pub shorthand_receiver: bool,
    pub variadic: Option<FunctionVariadic>,
    pub return_type: Type,
    pub generics: Generics,
    pub asyncness: bool,
    pub qualifiers: FunctionQualifiers,
    pub vis: Visibility,
    pub docs: Vec<String>,
    pub attrs: Vec<Attribute>,
    /// Explicit `safe` on a member of an unsafe extern block.
    pub safe: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredParam {
    pub param: Param,
    pub attrs: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionVariadic {
    pub attrs: Vec<Attribute>,
    pub pattern: Option<Pattern>,
}

/// Default methods share the function and block model used by free functions.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef {
    pub name: String,
    pub vis: Visibility,
    pub attrs: Vec<Attribute>,
    pub generics: Generics,
    pub supertraits: Vec<TypeBound>,
    pub unsafety: bool,
    pub auto: bool,
    pub members: Vec<TraitMember>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TraitMember {
    DefaultMethod(FunctionDef),
    RequiredMethod(FunctionDeclaration),
    AssocConst(AssociatedConstDeclaration),
    AssocType(AssociatedTypeDeclaration),
    /// The signature remains available when the default body's syntax is unsupported.
    RetainedMethod {
        declaration: FunctionDeclaration,
        source: String,
    },
    Macro(MemberMacro),
    Raw(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssociatedConstDeclaration {
    pub name: String,
    pub attrs: Vec<Attribute>,
    pub generics: Generics,
    pub ty: Type,
    pub default: Option<Initializer>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssociatedTypeDeclaration {
    pub name: String,
    pub attrs: Vec<Attribute>,
    pub generics: Generics,
    pub bounds: Vec<TypeBound>,
    pub default: Option<Type>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StaticDef {
    pub name: String,
    pub attrs: Vec<Attribute>,
    pub docs: Vec<String>,
    pub vis: Visibility,
    pub mutable: bool,
    pub ty: Type,
    pub value: Initializer,
}

/// A written initializer whose declaration can still be modeled if its expression cannot.
#[derive(Debug, Clone, PartialEq)]
pub enum Initializer {
    Expr(Expr),
    Raw(String),
}

impl Initializer {
    pub fn expression(&self) -> Option<&Expr> {
        match self {
            Self::Expr(value) => Some(value),
            Self::Raw(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForeignBlock {
    pub abi: FunctionAbi,
    pub unsafety: bool,
    pub attrs: Vec<Attribute>,
    pub members: Vec<ForeignMember>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForeignMember {
    Function(FunctionDeclaration),
    Static(ForeignStatic),
    Type(ForeignType),
    Macro(MemberMacro),
    Raw(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberMacro {
    pub attrs: Vec<Attribute>,
    pub invocation: MacroInvocation,
    pub trailing_semicolon: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForeignSafety {
    #[default]
    Implicit,
    Safe,
    Unsafe,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForeignStatic {
    pub name: String,
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub safety: ForeignSafety,
    pub mutable: bool,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForeignType {
    pub name: String,
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub generics: Generics,
}
