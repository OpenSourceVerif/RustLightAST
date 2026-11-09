// crates/rustlightast/src/rustlight_ast.rs

mod pattern;

pub use pattern::{BindingMode, Pattern, PatternField};

/// Lightweight Rust abstract syntax tree (module level)
#[derive(Debug, Clone)]
pub struct RustModule {
    pub name: String,
    pub docs: Vec<String>,
    pub items: Vec<Item>,
    pub attrs: Vec<Attribute>, // #[attributes]
    pub vis: Visibility,       // controls the visibility of the module
}

/// Module item definitions
#[derive(Debug, Clone)]
pub enum Item {
    Raw(String), // raw Rust item or crate-level directive
    Struct(StructDef),
    Enum(EnumDef),
    Union(UnionDef),
    Function(FunctionDef),
    Impl(ImplBlock),
    Const(ConstDef),
    TypeAlias(TypeAlias),
    Use(UseStatement),
    Mod(Box<RustModule>),      // nested module
    LazyStatic(LazyStaticDef), // lazy_static! macro
}

/// Struct definition
#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<Field>,
    pub generics: Vec<GenericParam>,
    pub derives: Vec<String>, // #[derive(...)]
    pub docs: Vec<String>,
    pub vis: Visibility, // controls struct visibility
}

/// Union definition
#[derive(Debug, Clone)]
pub struct UnionDef {
    pub name: String,
    pub fields: Vec<Field>, // union fields
    pub generics: Vec<GenericParam>,
    pub derives: Vec<String>, // #[derive(...)]
    pub docs: Vec<String>,
    pub vis: Visibility, // controls union visibility
}

/// Enum definition
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<Variant>,
    pub generics: Vec<GenericParam>,
    pub derives: Vec<String>,
    pub docs: Vec<String>,
    pub vis: Visibility,
}

/// Function definition
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub generics: Vec<GenericParam>,
    pub body: Block,
    pub asyncness: bool,
    pub vis: Visibility,
    pub docs: Vec<String>,
    pub attrs: Vec<Attribute>,
}

/// Implementation block
#[derive(Debug, Clone)]
pub struct ImplBlock {
    pub target: Type,
    pub generics: Vec<GenericParam>,
    pub items: Vec<ImplItem>,
    pub trait_impl: Option<Type>, // implemented for which trait
}

/// Constant definition
#[derive(Debug, Clone)]
pub struct ConstDef {
    pub name: String,
    pub ty: Type,
    pub value: Expr,
    pub vis: Visibility,
    pub docs: Vec<String>,
}

/// Type alias
#[derive(Debug, Clone)]
pub struct TypeAlias {
    pub name: String,
    pub target: Type,
    pub generics: Vec<GenericParam>,
    pub vis: Visibility,
    pub docs: Vec<String>,
}

// ========== Basic type definitions ========== //

/// Type representation
#[derive(Debug, Clone)]
pub enum Type {
    Path(Vec<String>),                // std::vec::Vec
    Named(String),                    // i32, String
    Generic(String, Vec<Type>),       // HashMap<K, V>
    CallableTrait(CallableTraitType), // dyn Fn(A) -> B / impl Fn(A) -> B
    /// Reference target, optional explicit lifetime, and mutability.
    Reference(Box<Type>, Option<String>, bool),
    Tuple(Vec<Type>),                 // (T1, T2)
    Slice(Box<Type>),                 // [T]
    Array(Box<Type>, usize),          // [T; N] fixed-size array
    Unit,                             // ()
    Never,                            // !
    Infer,                            // _
}

/// Callable trait type such as `dyn Fn(A) -> B` or `impl Fn(A) -> B`.
#[derive(Debug, Clone)]
pub struct CallableTraitType {
    pub qualifier: CallableTraitQualifier,
    /// Namespace segments of the callable trait, with an empty first segment
    /// for an absolute path.
    pub trait_path: Vec<String>,
    pub args: Vec<Type>,
    pub return_type: Box<Type>,
    pub leading_bounds: Vec<CallableBound>,
    pub trailing_bounds: Vec<CallableBound>,
    pub higher_ranked: Vec<LifetimeParameter>,
}

/// A bound on the callable in addition to its call signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallableBound {
    Lifetime(String),
    Trait(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum CallableTraitQualifier {
    Dyn,
    Impl,
}

/// A closure parameter with its pattern and optional type kept separately.
///
/// Keeping the type structured lets optimization passes inspect and rewrite
/// closure annotations without reparsing a printed `pattern: Type` string.
#[derive(Debug, Clone)]
pub struct ClosureParam {
    pub pattern: String,
    pub ty: Option<Type>,
}

impl ClosureParam {
    pub fn untyped(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            ty: None,
        }
    }

    pub fn typed(pattern: impl Into<String>, ty: Type) -> Self {
        Self {
            pattern: pattern.into(),
            ty: Some(ty),
        }
    }
}

impl From<&str> for ClosureParam {
    fn from(pattern: &str) -> Self {
        Self::untyped(pattern)
    }
}

impl From<String> for ClosureParam {
    fn from(pattern: String) -> Self {
        Self::untyped(pattern)
    }
}

#[derive(Debug, Clone)]
pub enum PathType {
    Namespace, // separated by :: (e.g., std::thread)
}

/// Expression
#[derive(Debug, Clone)]
pub enum Expr {
    Ident(String),
    /// An opaque macro invocation preserved as Rust source, e.g. `panic!("msg")`.
    Macro(String),
    Path(Vec<String>, PathType),
    Field {
        base: Box<Expr>,
        member: Member,
    },
    Record {
        path: Vec<String>,
        fields: Vec<RecordField>,
        rest: Option<Box<Expr>>,
    },
    Literal(Literal),
    Array(Vec<Expr>),
    Repeat {
        value: Box<Expr>,
        len: Box<Expr>,
    },
    Range {
        start: Option<Box<Expr>>,
        end: RangeEnd,
    },
    Tuple(Vec<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    MethodCall(Box<Expr>, String, Vec<Expr>),
    Block(Block),
    Loop(Box<Block>),
    While {
        condition: Box<Expr>,
        body: Block,
    },
    For {
        pattern: Pattern,
        iter: Box<Expr>,
        body: Block,
    },
    Let {
        pattern: Pattern,
        value: Box<Expr>,
    },
    Return(Option<Box<Expr>>),
    Break(Option<Box<Expr>>),
    Continue,
    Await(Box<Expr>),
    Try(Box<Expr>),
    /// `(params, body, is_move)` — third field is `true` for `move |…| …` closures.
    Closure(Vec<ClosureParam>, Box<Expr>, bool),
    /// A closure whose explicit Rust return type must survive parse/print.
    TypedClosure(Vec<ClosureParam>, Type, Box<Expr>, bool),
    BuilderChain(Vec<BuilderMethod>), // represents builder-style chained calls
    Unsafe(Box<Block>),               // unsafe expression support
    If {
        condition: Box<Expr>,
        then_branch: Block,
        else_branch: Option<Block>,
    }, // conditional expression
    IfLet {
        pattern: String,
        value: Box<Expr>,
        then_branch: Block,
        else_branch: Option<Block>,
    },
    Match {
        expr: Box<Expr>,
        arms: Vec<MatchArm>,
    }, // match expression
    Reference(Box<Expr>, bool, bool), // &expr, first flag for &, second flag for mut
    BinaryOp(Box<Expr>, String, Box<Expr>), // left op right, e.g., a != 0
    UnaryOp(String, Box<Expr>),       // op expr, e.g., !x, -y
    Index(Box<Expr>, Box<Expr>),      // expr[index], e.g., array[i]
    Parenthesized(Box<Expr>),         // (expr), e.g., (a + b)
    Cast(Box<Expr>, Type),            // expr as Type
    Assign(Box<Expr>, Box<Expr>), // left = right, e.g., state = State::S0 // lazy_static! macro support
}

/// A half-open range can omit its end. An inclusive range requires one.
#[derive(Debug, Clone)]
pub enum RangeEnd {
    Exclusive(Option<Box<Expr>>),
    Inclusive(Box<Expr>),
}

impl RangeEnd {
    pub fn expression(&self) -> Option<&Expr> {
        match self {
            Self::Exclusive(end) => end.as_deref(),
            Self::Inclusive(end) => Some(end),
        }
    }

    pub fn expression_mut(&mut self) -> Option<&mut Expr> {
        match self {
            Self::Exclusive(end) => end.as_deref_mut(),
            Self::Inclusive(end) => Some(end),
        }
    }

    pub fn map_expression(&self, rewrite: impl FnOnce(&Expr) -> Expr) -> Self {
        match self {
            Self::Exclusive(end) => {
                Self::Exclusive(end.as_deref().map(|end| Box::new(rewrite(end))))
            }
            Self::Inclusive(end) => Self::Inclusive(Box::new(rewrite(end))),
        }
    }
}

/// A named field or the position of a tuple field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Member {
    Named(String),
    Index(u32),
}

/// The value of a field initializer, in source evaluation order.
#[derive(Debug, Clone)]
pub struct RecordField {
    pub member: Member,
    pub value: Expr,
    /// Preserve `field` when the value is still the same-named binding.
    pub shorthand: bool,
}

// Match expression arm
#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Block,
}

// Distinguish different builder methods such as .name(), .stack_size(), etc.
#[derive(Debug, Clone)]
pub enum BuilderMethod {
    Named(String), // e.g., .name("thread_name")
    //StackSize(Box<Expr>), // TODO .stack_size(expr)
    Spawn {
        closure: Box<Expr>,
        move_kw: bool, // move semantics handled in BuilderMethod
    },
}

/// Literal
#[derive(Debug, Clone)]
pub enum Literal {
    /// Literal source preserved verbatim when its spelling carries Rust type or
    /// escaping information that must survive a parse/print round trip.
    Raw(String),
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Char(char),
}

/// Code block
#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Statement>,
    pub expr: Option<Box<Expr>>,
}

/// Statement
#[derive(Debug, Clone)]
pub enum Statement {
    Let(LetStmt),
    Expr(Expr),
    Item(Box<Item>),
    Comment(String), // comment statement
}

/// let binding
#[derive(Debug, Clone)]
pub struct LetStmt {
    pub ifmut: bool,
    pub name: String,
    pub ty: Option<Type>,
    pub init: Option<Expr>,
}

// ========== Auxiliary types ========== //

#[derive(Debug, Clone)]
pub enum Visibility {
    Public,
    Private,
    Restricted(Vec<String>), // pub(in path)
    None,                    // no visibility needed for trait implementations, follows the trait
}

#[derive(Debug, Clone)]
pub struct Attribute {
    pub name: String,
    pub args: Vec<AttributeArg>,
}

#[derive(Debug, Clone)]
pub enum AttributeArg {
    Ident(String),
    Literal(Literal),
    KeyValue(String, Literal),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub vis: Visibility,
    pub docs: Vec<String>,
    pub attrs: Vec<Attribute>,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub data: Option<Vec<Type>>, // Some for tuple variant
    pub docs: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum GenericParam {
    Type(TypeParameter),
    Lifetime(LifetimeParameter),
}

#[derive(Debug, Clone)]
pub struct TypeParameter {
    pub name: String,
    pub bounds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifetimeParameter {
    /// Spelling including the apostrophe.
    pub name: String,
    pub bounds: Vec<String>,
}

impl GenericParam {
    pub fn name(&self) -> &str {
        match self {
            Self::Type(param) => &param.name,
            Self::Lifetime(param) => &param.name,
        }
    }

    pub fn bounds(&self) -> &[String] {
        match self {
            Self::Type(param) => &param.bounds,
            Self::Lifetime(param) => &param.bounds,
        }
    }

    pub fn type_parameter(&self) -> Option<&TypeParameter> {
        match self {
            Self::Type(param) => Some(param),
            Self::Lifetime(_) => None,
        }
    }

    pub fn type_parameter_mut(&mut self) -> Option<&mut TypeParameter> {
        match self {
            Self::Type(param) => Some(param),
            Self::Lifetime(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    /// Mutability of the binding, independently of a reference type's mutability.
    pub mutable: bool,
}

#[derive(Debug, Clone)]
pub enum ImplItem {
    Method(FunctionDef),
    AssocConst(String, Type, Expr),
    AssocType(String, Type),
}

#[derive(Debug, Clone)]
pub struct UseStatement {
    pub path: Vec<String>,
    pub kind: UseKind,
}

#[derive(Debug, Clone)]
pub enum UseKind {
    Simple,
    Glob,                // {path}::*
    Nested(Vec<String>), // {path}::{a, b}
}

/// lazy_static! macro definition
#[derive(Debug, Clone)]
pub struct LazyStaticDef {
    pub name: String,
    pub ty: Type,
    pub init: Block,
    pub vis: Visibility,
    pub docs: Vec<String>,
}
