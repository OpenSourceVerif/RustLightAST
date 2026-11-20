use super::{
    Attribute, Expr, FunctionDeclaration, FunctionDef, Generics, ImplBlock, Item, ItemMacroSyntax,
    RustModule, Type, TypeBound, UseStatement, Visibility,
};

/// A parsed declaration kept frozen in its original spelling.
///
/// Visitors leave the frozen body alone. Analyses can explicitly inspect its
/// header and any modeled conditional declaration established at ingestion.
#[derive(Debug, Clone, PartialEq)]
pub struct RetainedItem {
    source: String,
    header: RetainedItemHeader,
    attrs: Vec<Attribute>,
    conditional_item: Option<Item>,
    macro_syntax: Option<ItemMacroSyntax>,
    may_emit_declarations: bool,
    body_model: Option<Box<Item>>,
    declaration: Option<Box<RetainedDeclaration>>,
}

impl RetainedItem {
    pub fn new(source: String, header: RetainedItemHeader, conditional_item: Option<Item>) -> Self {
        Self {
            source,
            header,
            attrs: Vec::new(),
            conditional_item,
            macro_syntax: None,
            may_emit_declarations: true,
            body_model: None,
            declaration: None,
        }
    }

    /// Keep a modeled declaration available for read-only queries.
    pub fn set_readonly_model(&mut self, item: Item) {
        self.body_model = Some(Box::new(item));
    }

    /// Borrow a declaration whose body was modeled while reading the source.
    pub fn modeled_body_item(&self) -> Option<&Item> {
        self.body_model.as_deref()
    }

    pub fn take_modeled_body_item(&mut self) -> Option<Item> {
        self.body_model.take().map(|item| *item)
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Set the original spelling after collecting the declaration's source span.
    pub fn with_source(mut self, source: String) -> Self {
        self.source = source;
        self
    }

    pub fn header(&self) -> &RetainedItemHeader {
        &self.header
    }

    pub fn attrs(&self) -> &[Attribute] {
        &self.attrs
    }

    pub fn with_attrs(mut self, attrs: Vec<Attribute>) -> Self {
        self.attrs = attrs;
        self
    }

    /// A read-only view of an allowlisted cfg-guarded function, import, or inline module.
    pub fn conditional_item(&self) -> Option<&Item> {
        self.conditional_item.as_ref()
    }



    /// The syntax of an item macro established from the original declaration.
    pub fn macro_syntax(&self) -> Option<&ItemMacroSyntax> {
        self.macro_syntax.as_ref()
    }

    /// Replace the spelling and modeled input of a retained macro.
    pub fn replace_macro_input(&mut self, source: String, tokens: super::MacroInput) {
        if let Some(ItemMacroSyntax::Invocation(invocation)) = &mut self.macro_syntax {
            invocation.tokens = tokens;
        }
        self.source = source;
    }

    /// Declaration facts available without opening the retained body.
    pub fn declaration(&self) -> Option<&RetainedDeclaration> {
        self.declaration.as_deref()
    }

    pub fn with_declaration(mut self, declaration: Option<RetainedDeclaration>) -> Self {
        self.declaration = declaration.map(Box::new);
        self
    }

    /// Whether the spelling may contain an attribute, an impl, or a macro call.
    pub fn may_emit_declarations(&self) -> bool {
        self.may_emit_declarations
    }

    pub fn with_macro_syntax(
        mut self,
        macro_syntax: Option<ItemMacroSyntax>,
        may_emit_declarations: bool,
    ) -> Self {
        self.macro_syntax = macro_syntax;
        self.may_emit_declarations = may_emit_declarations;
        self
    }
}

/// Semantic declarations whose complete source remains frozen.
#[derive(Debug, Clone, PartialEq)]
pub enum RetainedDeclaration {
    Data {
        generics: Option<Generics>,
        fields: Option<Vec<Type>>,
        variants: Vec<RetainedVariantBinding>,
    },
    Alias {
        generics: Option<Generics>,
        target: Option<Type>,
    },
    TraitAlias {
        generics: Option<Generics>,
        bounds: Vec<TypeBound>,
    },
    Function {
        signature: Option<FunctionDef>,
    },
    /// ABI signatures can include syntax that an editable function cannot represent.
    ForeignFunction(FunctionDeclaration),
    Static {
        ty: Option<Type>,
        initializer: Option<Expr>,
    },
    Implementation(RetainedImplementation),
    Module(RustModule),
}

/// Dispatch information survives incomplete lowering of a method signature.
#[derive(Debug, Clone, PartialEq)]
pub struct RetainedImplementation {
    pub header: Option<ImplBlock>,
    pub unsupported_methods: Vec<String>,
    pub retained_method_signatures: Vec<FunctionDeclaration>,
    pub is_trait: bool,
    pub trait_identity: Option<RetainedTraitImplementation>,
}

/// The written trait and target also identify impls with unsupported bodies
/// or modifiers. The full header determines whether the impl can be modeled.
#[derive(Debug, Clone, PartialEq)]
pub struct RetainedTraitImplementation {
    pub trait_path: Vec<String>,
    pub target: Type,
    pub generics: Generics,
}

/// Namespace facts for variants remain available if their field types fail to lower.
#[derive(Debug, Clone, PartialEq)]
pub struct RetainedVariantBinding {
    pub name: String,
    pub conditional_identity: bool,
}

/// The name and visibility introduced by an item, independently of its body.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemHeader {
    pub name: Option<String>,
    pub visibility: Option<Visibility>,
    pub kind: ItemHeaderKind,
}

/// Declaration and conditional attributes of a retained source item.
#[derive(Debug, Clone, PartialEq)]
pub struct RetainedItemHeader {
    pub header: ItemHeader,
    conditional_attributes: bool,
    conditional_identity: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemHeaderKind {
    Function,
    Nominal,
    Enum,
    Type,
    Trait,
    TraitAlias,
    Module,
    MacroRules,
    MacroInvocation,
    ExternCrate {
        crate_name: String,
    },
    ForeignBlock {
        names: Vec<String>,
        has_unmodeled_member: bool,
    },
    Import(UseStatement),
    Unmodeled,
    Other,
}

impl RetainedItemHeader {
    pub fn new(header: ItemHeader, conditional_attributes: bool) -> Self {
        Self {
            header,
            conditional_attributes,
            conditional_identity: conditional_attributes,
        }
    }

    pub fn with_conditional_identity(mut self, conditional: bool) -> Self {
        self.conditional_identity = conditional;
        self
    }

    /// Whether outer attributes can change the name or existence of this item.
    pub fn has_conditional_identity(&self) -> bool {
        self.conditional_identity
    }

    /// Whether a retained source item contains `cfg` or `cfg_attr`.
    pub fn has_conditional_attributes(&self) -> bool {
        self.conditional_attributes
    }
}
