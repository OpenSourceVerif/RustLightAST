//! Declaration facts available without consulting function or implementation bodies.

use proc_macro2::{Delimiter, TokenStream, TokenTree};

use crate::{
    Attribute, ImplBlock, ImplItem, Item, RetainedDeclaration, RetainedImplementation,
    RetainedTraitImplementation, RetainedVariantBinding, RustModule, Type,
};

/// Read a declaration-only source file through the same projection.
pub fn parse_file_declaration_view(source: &str, name: String) -> syn::Result<RustModule> {
    let parsed = super::ParsedSourceFile::parse(source)
        .map_err(|error| syn::Error::new(proc_macro2::Span::call_site(), error))?;
    Ok(parsed.declaration_view(name))
}

pub(super) fn model_declaration_view(model: &RustModule, name: String) -> RustModule {
    fn project_module(model: &RustModule) -> RustModule {
        RustModule {
            name: model.name.clone(),
            docs: model.docs.clone(),
            attrs: model.attrs.clone(),
            vis: model.vis.clone(),
            items: model
                .items
                .iter()
                .map(|item| match item {
                    Item::Mod(module) if !super::source_file::module_attrs_conditional(module) => {
                        Item::Mod(Box::new(project_module(module)))
                    }
                    Item::Raw(_) | Item::Retained(_) | Item::Macro(_) => item.clone(),
                    _ => Item::Retained(Box::new(super::source_file::retained_model_item(item))),
                })
                .collect(),
        }
    }
    let mut result = project_module(model);
    result.name = name;
    result
}

pub(super) fn modeled_attribute_changes_binding(attribute: &Attribute, module_decl: bool) -> bool {
    let name = attribute.unqualified_name();
    if module_decl && name == Some("path") {
        return false;
    }
    if let Attribute::Source { meta, .. } = attribute {
        if name == Some("cfg_attr") {
            return cfg_attr_changes_binding(meta);
        }
    }
    !known_binding_attribute(name)
}

// Only comma-separated cfg_attr arguments need examination. Token groups keep
// nested predicates and attribute arguments from splitting the outer list.
fn cfg_attr_changes_binding(source: &str) -> bool {
    let Ok(tokens) = source.parse::<TokenStream>() else {
        return true;
    };
    let mut arguments = tokens.into_iter();
    let Some(TokenTree::Group(group)) = arguments.nth(1) else {
        return true;
    };
    if group.delimiter() != Delimiter::Parenthesis || arguments.next().is_some() {
        return true;
    }
    let mut parts = vec![TokenStream::new()];
    for token in group.stream() {
        if matches!(&token, TokenTree::Punct(punct) if punct.as_char() == ',') {
            parts.push(TokenStream::new());
        } else {
            parts.last_mut().unwrap().extend(std::iter::once(token));
        }
    }
    if parts.last().is_some_and(TokenStream::is_empty) {
        parts.pop();
    }
    parts.len() < 2
        || parts.iter().skip(1).any(|part| {
            modeled_attribute_changes_binding(
                &Attribute::Source {
                    meta: part.to_string(),
                    inner: false,
                },
                false,
            )
        })
}

fn known_binding_attribute(name: Option<&str>) -> bool {
    matches!(
        name,
        Some(
            "derive"
                | "doc"
                | "allow"
                | "warn"
                | "deny"
                | "forbid"
                | "inline"
                | "cold"
                | "must_use"
                | "repr"
                | "non_exhaustive"
                | "test"
                | "ignore"
                | "should_panic"
                | "deprecated"
                | "no_std"
                | "no_implicit_prelude"
                | "recursion_limit"
        )
    )
}

pub(super) fn modeled_declaration(item: &Item) -> Option<RetainedDeclaration> {
    let data =
        |generics: &crate::Generics, fields: Vec<Type>, variants| RetainedDeclaration::Data {
            generics: Some(generics.clone()),
            fields: Some(fields),
            variants,
        };
    Some(match item {
        Item::Struct(item) => data(
            &item.generics,
            item.fields
                .as_slice()
                .iter()
                .map(|field| field.ty.clone())
                .collect(),
            Vec::new(),
        ),
        Item::Union(item) => data(
            &item.generics,
            item.fields.iter().map(|field| field.ty.clone()).collect(),
            Vec::new(),
        ),
        Item::Enum(item) => data(
            &item.generics,
            item.variants
                .iter()
                .flat_map(|variant| variant.fields.as_slice())
                .map(|field| field.ty.clone())
                .collect(),
            item.variants
                .iter()
                .map(|variant| RetainedVariantBinding {
                    name: variant.name.clone(),
                    conditional_identity: variant.attrs.iter().any(|attr| {
                        super::source_file::written_attribute(attr)
                            .is_none_or(|attr| modeled_attribute_changes_binding(&attr, false))
                    }),
                })
                .collect(),
        ),
        Item::TypeAlias(item) => RetainedDeclaration::Alias {
            generics: Some(item.generics.clone()),
            target: Some(item.target.clone()),
        },
        Item::Function(item) => RetainedDeclaration::Function {
            signature: Some(item.signature_only()),
        },
        Item::Static(item) => RetainedDeclaration::Static {
            ty: Some(item.ty.clone()),
            initializer: item.value.expression().cloned(),
        },
        Item::Impl(item) => {
            let trait_identity = item.trait_impl.as_ref().and_then(|trait_ty| {
                let trait_path = match trait_ty {
                    Type::Named(name) => vec![name.clone()],
                    Type::Path(path) | Type::PathWithArguments(path, _) => path.clone(),
                    Type::Generic(name, _) => name.split("::").map(str::to_owned).collect(),
                    _ => return None,
                };
                Some(RetainedTraitImplementation {
                    trait_path,
                    target: item.target.clone(),
                    generics: item.generics.clone(),
                })
            });
            RetainedDeclaration::Implementation(RetainedImplementation {
                header: Some(ImplBlock {
                    unsafety: item.unsafety,
                    attrs: item.attrs.clone(),
                    target: item.target.clone(),
                    generics: item.generics.clone(),
                    trait_impl: item.trait_impl.clone(),
                    items: item
                        .items
                        .iter()
                        .map(|item| match item {
                            ImplItem::Method(method) => {
                                ImplItem::Method(method.signature_only())
                            }
                            _ => item.clone(),
                        })
                        .collect(),
                }),
                unsupported_methods: Vec::new(),
                retained_method_signatures: Vec::new(),
                is_trait: item.trait_impl.is_some(),
                trait_identity,
            })
        }
        Item::Mod(item) => {
            RetainedDeclaration::Module(model_declaration_view(item, item.name.clone()))
        }
        _ => return None,
    })
}
