//! Declaration metadata derived from the RustLight model.

use std::fmt;
use crate::{
    visit::{self, Visit},
    Attribute, DataAttribute, Item, ItemHeader, ItemHeaderKind,
    RetainedDeclaration, RetainedItem, RetainedItemHeader, RustModule,
};

/// Failure to read Rust syntax or lower it to the model.
#[derive(Debug, Clone)]
pub struct SourceParseError(String);

impl SourceParseError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SourceParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SourceParseError {}

impl From<syn::Error> for SourceParseError {
    fn from(error: syn::Error) -> Self {
        let start = error.span().start();
        Self(format!("{error} (line {}, column {})", start.line, start.column + 1))
    }
}

/// Whether an out-of-line or inline module placement is explicitly written.
pub(super) fn model_has_module_path_attribute(model: &RustModule) -> bool {
    #[derive(Default)]
    struct ModulePath(bool);
    impl<'model> Visit<'model> for ModulePath {
        fn visit_item(&mut self, item: &'model Item) {
            self.0 |= match item {
                Item::Mod(item) => item
                    .attrs
                    .iter()
                    .any(|attr| attr.unqualified_name() == Some("path")),
                Item::ModuleDecl(item) => item
                    .attrs
                    .iter()
                    .any(|attr| Attribute::source_unqualified_name(attr) == Some("path")),
                Item::Retained(item)
                    if matches!(item.header().header.kind, ItemHeaderKind::Module) =>
                {
                    item.attrs()
                        .iter()
                        .any(|attr| attr.unqualified_name() == Some("path"))
                }
                _ => false,
            };
            if !self.0 {
                if let Item::Retained(item) = item {
                    if let Some(modeled) =
                        item.modeled_body_item().or_else(|| item.conditional_item())
                    {
                        self.visit_item(modeled);
                    } else if let Some(RetainedDeclaration::Module(module)) = item.declaration() {
                        self.visit_module(module);
                    }
                } else {
                    visit::walk_item(self, item);
                }
            }
        }
    }
    let mut check = ModulePath::default();
    check.visit_module(model);
    check.0
}

pub(super) fn retained_model_item(item: &Item) -> RetainedItem {
    let (name, vis, kind, attrs) = match item {
        Item::Function(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Function,
            item.attrs.clone(),
        ),
        Item::Struct(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Nominal,
            data_attrs(&item.attrs),
        ),
        Item::Enum(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Enum,
            data_attrs(&item.attrs),
        ),
        Item::Union(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Nominal,
            data_attrs(&item.attrs),
        ),
        Item::Trait(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Trait,
            item.attrs.clone(),
        ),
        Item::TypeAlias(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Type,
            item.attrs.clone(),
        ),
        Item::Const(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Other,
            item.attrs.clone(),
        ),
        Item::Static(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Other,
            item.attrs.clone(),
        ),
        Item::LazyStatic(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Other,
            Vec::new(),
        ),
        Item::Mod(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Module,
            item.attrs.clone(),
        ),
        Item::ModuleDecl(item) => (
            Some(&item.name),
            Some(&item.vis),
            ItemHeaderKind::Module,
            written_attrs(&item.attrs),
        ),
        Item::Use(item) => (
            None,
            Some(&item.vis),
            ItemHeaderKind::Import(item.clone()),
            written_attrs(&item.attrs),
        ),
        Item::Impl(item) => (None, None, ItemHeaderKind::Other, item.attrs.clone()),
        Item::Foreign(item) => {
            let mut names = Vec::new();
            let mut has_unmodeled_member = false;
            for member in &item.members {
                use crate::ForeignMember::*;
                match member {
                    Function(item) => names.push(item.name.clone()),
                    Static(item) => names.push(item.name.clone()),
                    Type(item) => names.push(item.name.clone()),
                    Macro(_) | Raw(_) => has_unmodeled_member = true,
                }
            }
            (
                None,
                None,
                ItemHeaderKind::ForeignBlock {
                    names,
                    has_unmodeled_member,
                },
                item.attrs.clone(),
            )
        }
        Item::Raw(_) | Item::Retained(_) | Item::Macro(_) => unreachable!("already preserved by the converter"),
    };
    let header = RetainedItemHeader::new(
        ItemHeader {
            name: name.cloned(),
            visibility: vis.cloned(),
            kind,
        },
        declaration_has_conditional_attributes(item),
    )
    .with_conditional_identity(attrs.iter().any(|attr| {
        super::declaration_view::modeled_attribute_changes_binding(
            attr,
            matches!(item, Item::ModuleDecl(_)),
        )
    }));
    RetainedItem::new(String::new(), header, None)
        .with_attrs(attrs)
        .with_macro_syntax(None, modeled_item_may_emit(item))
        .with_declaration(super::declaration_view::modeled_declaration(item))
}

fn data_attrs(attrs: &[DataAttribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter_map(|attr| match attr {
            DataAttribute::Other(attr) => Some(attr.clone()),
            DataAttribute::Derive { .. } => None,
        })
        .collect()
}

fn written_attrs(attrs: &[String]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter_map(|attr| written_attribute(attr))
        .collect()
}

pub(super) fn written_attribute(attr: &str) -> Option<Attribute> {
    let attr = attr.trim().strip_prefix('#')?.trim_start();
    let (inner, attr) = attr
        .strip_prefix('!')
        .map_or((false, attr), |attr| (true, attr.trim_start()));
    let meta = attr.strip_prefix('[')?.strip_suffix(']')?.to_owned();
    Some(Attribute::Source { meta, inner })
}

fn modeled_item_may_emit(item: &Item) -> bool {
    #[derive(Default)]
    struct Expansion(bool);
    impl<'model> Visit<'model> for Expansion {
        fn visit_macro_invocation(&mut self, _: &'model crate::MacroInvocation) {
            self.0 = true;
        }
        fn visit_attribute(&mut self, _: &'model Attribute) {
            self.0 = true;
        }
        fn visit_written_attribute(&mut self, _: &'model str) {
            self.0 = true;
        }
        fn visit_opaque(&mut self, _: &'model str) {
            self.0 = true;
        }
        fn visit_item(&mut self, item: &'model Item) {
            self.0 |= matches!(item, Item::Impl(_))
                || matches!(item, Item::Retained(retained) if retained.may_emit_declarations());
            if !self.0 {
                visit::walk_item(self, item);
            }
        }
    }
    let mut expansion = Expansion::default();
    expansion.visit_item(item);
    expansion.0
}

fn conditional_name(name: Option<&str>) -> bool {
    matches!(name, Some("cfg" | "cfg_attr"))
}

pub(super) fn module_attrs_conditional(module: &RustModule) -> bool {
    module.attrs.iter().any(|attr| conditional_name(attr.unqualified_name()))
        || module.items.iter().any(|item| matches!(item, Item::Raw(source)
            if source.trim_start().starts_with("#!") && conditional_name(Attribute::source_unqualified_name(source))))
}

fn declaration_has_conditional_attributes(item: &Item) -> bool {
    match item {
        Item::Mod(module) => module_attrs_conditional(module),
        _ => item_has_conditional_attributes(item),
    }
}

pub(super) fn model_has_conditional_attributes(model: &RustModule) -> bool {
    model
        .attrs
        .iter()
        .any(|attr| conditional_name(attr.unqualified_name()))
        || model.items.iter().any(item_has_conditional_attributes)
}

fn item_has_conditional_attributes(item: &Item) -> bool {
    #[derive(Default)]
    struct Conditional(bool);
    impl<'model> Visit<'model> for Conditional {
        fn visit_attribute(&mut self, attr: &'model Attribute) {
            self.0 |= conditional_name(attr.unqualified_name());
        }
        fn visit_written_attribute(&mut self, attr: &'model str) {
            self.0 |= conditional_name(Attribute::source_unqualified_name(attr));
        }
        fn visit_item(&mut self, item: &'model Item) {
            if self.0 {
                return;
            }
            match item {
                Item::Retained(item) => self.0 |= item.header().has_conditional_attributes(),
                Item::Raw(source) => self.visit_written_attribute(source),
                _ => visit::walk_item(self, item),
            }
        }
    }
    let mut conditional = Conditional::default();
    conditional.visit_item(item);
    conditional.0
}
