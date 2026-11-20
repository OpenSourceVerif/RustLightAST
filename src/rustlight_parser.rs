//! Rust source parsing for the RustLight subset.
//!
//! Unsupported syntax returns a `syn::Error`.

/// Parse and conversion errors returned by the public entry points.
pub use syn::Error as ParseError;

mod declaration_view;
pub use declaration_view::parse_file_declaration_view;

mod parsed_source;
pub use parsed_source::{ParsedSourceFile, SourceSnapshotId};

pub mod model_origins;
use model_origins as origins;
use model_origins::Kind as Origin;

mod source_file;
pub use source_file::SourceParseError;

mod pattern;
mod macro_syntax;

pub use crate::{ItemHeader, ItemHeaderKind, RetainedItemHeader};

pub use pattern::{convert_pattern, parse_rust_pattern};

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use syn::{
    parse_file, AngleBracketedGenericArguments, ExprArray, ExprAssign, ExprBinary, ExprBlock,
    ExprCall, ExprCast, ExprClosure, ExprField, ExprGroup, ExprIf, ExprIndex, ExprLit, ExprMatch,
    ExprMethodCall, ExprParen, ExprPath, ExprReference, ExprTuple, ExprUnary, File,
    GenericArgument as SynGenericArgument, ImplItem as SynImplItem, Item as SynItem, Lit,
    LocalInit, Meta, ParenthesizedGenericArguments, Pat, PathArguments, ReturnType, Stmt,
    TraitBound, Type as SynType, TypeArray, TypeGroup, TypeImplTrait, TypeParamBound, TypeParen,
    TypeReference, TypeSlice, TypeTraitObject, TypeTuple, Visibility as SynVisibility,
};

use crate::{
    AssociatedTypeBinding, Attribute, Block, CallableBound, CallableTraitQualifier,
    CallableTraitType, ClosureParam, ConstDef, EnumDef, Expr, Field, FunctionAbi, FunctionDef,
    FunctionQualifiers, GenericArgument, GenericParam, Generics, ImplBlock, ImplItem, Item,
    LetStmt, LifetimeParameter, Literal, MatchArm, Member, ModuleDecl, Param, PathType,
    RecordField, RecordPath, RustModule, Statement, StructDef, Type, TypeAlias, TypeParameter,
    UnionDef, UseStatement, UseTree, Variant, Visibility, WherePredicate,
};

/// Parse a Rust file into the supported RustLight subset.
///
/// `module_name` is caller-provided metadata. No files are read or written.
/// Syntax outside the converter's supported subset returns an error; this is
/// not a lossless parser for arbitrary Rust source.
pub fn parse_rust_source(source: &str, module_name: impl Into<String>) -> syn::Result<RustModule> {
    let file = parse_file(source)?;
    convert_file(&file, module_name.into())
}

/// Lower a parsed file at the Rust source boundary.
pub fn convert_file(file: &File, module_name: String) -> syn::Result<RustModule> {
    let items = file
        .items
        .iter()
        .map(|item| origins::try_node(Origin::Item, || convert_item(item)))
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(file_module(file, module_name, items))
}

fn file_module(
    file: &File,
    module_name: String,
    items: impl IntoIterator<Item = Item>,
) -> RustModule {
    origins::inner_attributes(file.attrs.len());
    RustModule {
        name: module_name,
        docs: Vec::new(),
        items: file
            .attrs
            .iter()
            .map(|attr| Item::Raw(attribute_to_raw_source(attr)))
            .chain(items)
            .collect(),
        attrs: Vec::new(),
        vis: Visibility::Private,
    }
}

/// Construct an inline module from its parsed header and lowered declarations.
pub fn convert_inline_module(module: &syn::ItemMod, mut items: Vec<Item>) -> RustModule {
    let mut outer = Vec::new();
    let mut inner = Vec::new();
    for attr in &module.attrs {
        match attr.style {
            syn::AttrStyle::Outer => outer.push(attr.clone()),
            syn::AttrStyle::Inner(_) => inner.push(Item::Raw(attribute_to_raw_source(attr))),
        }
    }
    origins::inner_attributes(inner.len());
    items.splice(..0, inner);

    RustModule {
        name: module.ident.to_string(),
        docs: Vec::new(),
        items,
        attrs: convert_attributes(&outer),
        vis: convert_visibility(&module.vis),
    }
}

/// Lower a parsed item without reparsing its source text.
pub fn convert_item(item: &SynItem) -> syn::Result<Item> {
    origins::attempt(|| lower_item(item))
}

fn lower_item(item: &SynItem) -> syn::Result<Item> {
    match item {
        SynItem::Enum(item_enum) => {
            let generics = convert_generics(&item_enum.generics)?;
            Ok(Item::Enum(EnumDef {
                name: item_enum.ident.to_string(),
                variants: item_enum
                    .variants
                    .iter()
                    .map(convert_variant)
                    .collect::<syn::Result<Vec<_>>>()?,
                generics,
                docs: Vec::new(),
                attrs: convert_data_attributes(&item_enum.attrs),
                vis: convert_visibility(&item_enum.vis),
            }))
        }
        SynItem::Fn(item_fn) => Ok(Item::Function(convert_function_body(
            &item_fn.sig, &item_fn.block, &item_fn.vis, &item_fn.attrs,
        )?)),
        SynItem::Struct(item_struct) => convert_struct(item_struct).map(Item::Struct),
        SynItem::Union(item_union) => {
            let generics = convert_generics(&item_union.generics)?;
            Ok(Item::Union(UnionDef {
                name: item_union.ident.to_string(),
                fields: item_union
                    .fields
                    .named
                    .iter()
                    .map(convert_field)
                    .collect::<syn::Result<Vec<_>>>()?,
                generics,
                docs: Vec::new(),
                attrs: convert_data_attributes(&item_union.attrs),
                vis: convert_visibility(&item_union.vis),
            }))
        }
        SynItem::Type(item_type) => Ok(Item::TypeAlias(TypeAlias {
            name: item_type.ident.to_string(),
            generics: convert_generics(&item_type.generics)?,
            target: convert_type(&item_type.ty)?,
            vis: convert_visibility(&item_type.vis),
            docs: Vec::new(),
            attrs: convert_attributes(&item_type.attrs),
        })),
        SynItem::Const(item_const) => Ok(Item::Const(ConstDef {
            name: item_const.ident.to_string(),
            ty: convert_type(&item_const.ty)?,
            value: convert_expr(&item_const.expr)?,
            vis: convert_visibility(&item_const.vis),
            docs: Vec::new(),
            attrs: convert_attributes(&item_const.attrs),
        })),
        SynItem::Static(item_static) => Ok(Item::Static(crate::StaticDef {
            name: item_static.ident.to_string(),
            ty: convert_type(&item_static.ty)?,
            value: crate::Initializer::Expr(convert_expr(&item_static.expr)?),
            mutable: matches!(item_static.mutability, syn::StaticMutability::Mut(_)),
            vis: convert_visibility(&item_static.vis),
            docs: Vec::new(),
            attrs: convert_attributes(&item_static.attrs),
        })),
        SynItem::ForeignMod(item) => Ok(Item::Foreign(crate::ForeignBlock {
            abi: convert_abi(&item.abi),
            unsafety: item.unsafety.is_some(),
            attrs: convert_attributes(&item.attrs),
            members: item.items.iter().map(lower_foreign_member).collect::<syn::Result<_>>()?,
        })),
        SynItem::Use(item_use) => Ok(Item::Use(convert_use(item_use))),
        SynItem::Impl(item_impl) => Ok(Item::Impl(convert_implementation(item_impl)?)),
        SynItem::Mod(item_mod) if item_mod.content.is_none() => Ok(Item::ModuleDecl(ModuleDecl {
            name: item_mod.ident.to_string(),
            vis: convert_visibility(&item_mod.vis),
            attrs: item_mod.attrs.iter().map(attribute_to_raw_source).collect(),
        })),
        SynItem::Mod(item_mod) => {
            let nested_items = item_mod
                .content
                .as_ref()
                .map(|(_, items)| items.iter().map(|item| {
                    origins::try_node(Origin::Item, || convert_item(item))
                }).collect())
                .transpose()?
                .unwrap_or_default();

            Ok(Item::Mod(Box::new(convert_inline_module(
                item_mod,
                nested_items,
            ))))
        }
        SynItem::Trait(item) => origins::try_node(Origin::Trait, || convert_trait(item)),
        SynItem::Macro(item) => Ok(Item::Macro(Box::new(convert_item_macro(item)))),
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported item kind in RustLightAST parser",
        )),
    }
}

fn convert_variant(variant: &syn::Variant) -> syn::Result<Variant> {
    Ok(Variant {
        name: variant.ident.to_string(),
        fields: convert_fields(&variant.fields)?,
        discriminant: variant
            .discriminant
            .as_ref()
            .map(|(_, value)| convert_expr(value))
            .transpose()?,
        attrs: variant.attrs.iter().map(attribute_to_raw_source).collect(),
    })
}

/// Lower a parsed struct declaration.
pub fn convert_struct(item: &syn::ItemStruct) -> syn::Result<StructDef> {
    Ok(StructDef {
        name: item.ident.to_string(),
        generics: convert_generics(&item.generics)?,
        fields: convert_fields(&item.fields)?,
        docs: Vec::new(),
        attrs: convert_data_attributes(&item.attrs),
        vis: convert_visibility(&item.vis),
    })
}

fn convert_fields(fields: &syn::Fields) -> syn::Result<crate::Fields> {
    let converted = fields
        .iter()
        .map(convert_field)
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(match fields {
        syn::Fields::Unit => crate::Fields::Unit,
        syn::Fields::Unnamed(_) => crate::Fields::Tuple(converted),
        syn::Fields::Named(_) => crate::Fields::Named(converted),
    })
}

fn convert_field(field: &syn::Field) -> syn::Result<Field> {
    Ok(Field {
        name: field
            .ident
            .as_ref()
            .map(|ident| ident.to_string())
            .unwrap_or_default(),
        ty: convert_type(&field.ty)?,
        vis: convert_visibility(&field.vis),
        docs: Vec::new(),
        attrs: convert_attributes(&field.attrs),
    })
}

fn convert_trait(item: &syn::ItemTrait) -> syn::Result<Item> {
    if item.restriction.is_some() {
        return Err(syn::Error::new_spanned(item, "unsupported trait restriction"));
    }
    Ok(Item::Trait(crate::TraitDef {
        name: item.ident.to_string(),
        vis: convert_visibility(&item.vis),
        attrs: convert_attributes(&item.attrs),
        generics: convert_generics(&item.generics)?,
        supertraits: item.supertraits.iter().map(convert_type_bound).collect::<syn::Result<_>>()?,
        unsafety: item.unsafety.is_some(),
        auto: item.auto_token.is_some(),
        members: item.items.iter().map(lower_trait_member).collect::<syn::Result<_>>()?,
    }))
}

fn lower_trait_member(member: &syn::TraitItem) -> syn::Result<crate::TraitMember> {
    use crate::TraitMember;

    Ok(match member {
        syn::TraitItem::Fn(method) => {
            let visibility = &syn::Visibility::Inherited;
            if let Some(default) = &method.default {
                TraitMember::DefaultMethod(convert_function_body(&method.sig, default, visibility, &method.attrs)?)
            } else {
                TraitMember::RequiredMethod(convert_function_declaration(
                    &method.sig, visibility, Vec::new(), convert_attributes(&method.attrs),
                )?)
            }
        }
        syn::TraitItem::Const(associated) => TraitMember::AssocConst(crate::AssociatedConstDeclaration {
            name: associated.ident.to_string(),
            attrs: convert_attributes(&associated.attrs),
            generics: convert_generics(&associated.generics)?,
            ty: convert_type(&associated.ty)?,
            default: associated.default.as_ref()
                .map(|(_, value)| convert_expr(value).map(crate::Initializer::Expr))
                .transpose()?,
        }),
        syn::TraitItem::Type(associated) => TraitMember::AssocType(crate::AssociatedTypeDeclaration {
            name: associated.ident.to_string(),
            attrs: convert_attributes(&associated.attrs),
            generics: convert_generics(&associated.generics)?,
            bounds: associated.bounds.iter().map(convert_type_bound).collect::<syn::Result<_>>()?,
            default: associated.default.as_ref().map(|(_, ty)| convert_type(ty)).transpose()?,
        }),
        syn::TraitItem::Macro(mac) => TraitMember::Macro(convert_member_macro(&mac.attrs, &mac.mac, mac.semi_token.is_some())),
        _ => return Err(syn::Error::new_spanned(member, "unsupported trait member")),
    })
}

fn convert_member_macro(attrs: &[syn::Attribute], mac: &syn::Macro, trailing_semicolon: bool) -> crate::MemberMacro {
    crate::MemberMacro {
        attrs: convert_attributes(attrs),
        invocation: convert_macro_invocation_with_attributes(mac, !attrs.is_empty()),
        trailing_semicolon,
    }
}

fn lower_foreign_member(member: &syn::ForeignItem) -> syn::Result<crate::ForeignMember> {
    use crate::{ForeignMember, ForeignSafety};

    Ok(match member {
        syn::ForeignItem::Fn(function) => ForeignMember::Function(convert_function_declaration(
            &function.sig, &function.vis, Vec::new(), convert_attributes(&function.attrs),
        )?),
        syn::ForeignItem::Static(item) => ForeignMember::Static(convert_foreign_static(item, ForeignSafety::Implicit)?),
        syn::ForeignItem::Type(item) => ForeignMember::Type(crate::ForeignType {
            name: item.ident.to_string(),
            attrs: convert_attributes(&item.attrs),
            vis: convert_visibility(&item.vis),
            generics: convert_generics(&item.generics)?,
        }),
        syn::ForeignItem::Macro(mac) => ForeignMember::Macro(convert_member_macro(&mac.attrs, &mac.mac, mac.semi_token.is_some())),
        syn::ForeignItem::Verbatim(tokens) => {
            // syn preserves explicit safety markers on extern items as tokens.
            use syn::parse::Parser;
            return Parser::parse2(lower_foreign_safety_member, tokens.clone());
        }
        _ => return Err(syn::Error::new_spanned(member, "unsupported foreign declaration")),
    })
}

fn convert_foreign_static(item: &syn::ForeignItemStatic, safety: crate::ForeignSafety) -> syn::Result<crate::ForeignStatic> {
    Ok(crate::ForeignStatic {
        name: item.ident.to_string(),
        attrs: convert_attributes(&item.attrs),
        vis: convert_visibility(&item.vis),
        safety,
        mutable: matches!(item.mutability, syn::StaticMutability::Mut(_)),
        ty: convert_type(&item.ty)?,
    })
}

fn lower_foreign_safety_member(input: syn::parse::ParseStream<'_>) -> syn::Result<crate::ForeignMember> {
    use crate::{ForeignMember, ForeignSafety};
    use syn::Token;
    syn::custom_keyword!(safe);

    let attrs = input.call(syn::Attribute::parse_outer)?;
    let vis = input.parse()?;
    let lookahead = input.lookahead1();
    let safety = if lookahead.peek(safe) {
        input.parse::<safe>()?;
        ForeignSafety::Safe
    } else if lookahead.peek(Token![unsafe]) && input.peek2(Token![static]) {
        input.parse::<Token![unsafe]>()?;
        ForeignSafety::Unsafe
    } else {
        return Err(lookahead.error());
    };
    if input.peek(Token![static]) {
        let mut item = input.parse::<syn::ForeignItemStatic>()?;
        item.vis = vis;
        item.attrs = attrs;
        return convert_foreign_static(&item, safety).map(ForeignMember::Static);
    }
    let signature = input.parse()?;
    input.parse::<Token![;]>()?;
    let mut declaration = convert_function_declaration(&signature, &vis, Vec::new(), convert_attributes(&attrs))?;
    declaration.safe = true;
    Ok(ForeignMember::Function(declaration))
}

fn convert_function_body(
    signature: &syn::Signature,
    body: &syn::Block,
    visibility: &syn::Visibility,
    attrs: &[syn::Attribute],
) -> syn::Result<FunctionDef> {
    origins::try_node(Origin::Function, || {
        convert_function(signature, convert_block(body)?, visibility, Vec::new(), convert_attributes(attrs))
    })
}

/// Lower a parsed function signature around an already converted RustLight body.
pub fn convert_function(
    signature: &syn::Signature,
    body: Block,
    visibility: &syn::Visibility,
    docs: Vec<String>,
    attrs: Vec<Attribute>,
) -> syn::Result<FunctionDef> {
    if let Some(variadic) = &signature.variadic {
        return Err(syn::Error::new_spanned(
            variadic,
            "unsupported variadic parameter",
        ));
    }
    if let Some(attribute) = signature.inputs.iter().flat_map(fn_arg_attributes).next() {
        return Err(syn::Error::new_spanned(
            attribute,
            "unsupported attributes on function parameter",
        ));
    }
    let crate::FunctionDeclaration {
        name, params, return_type, generics, asyncness, qualifiers, vis, docs, attrs, ..
    } = convert_function_declaration(signature, visibility, docs, attrs)?;
    let params = params.into_iter().map(|parameter| parameter.param).collect();
    Ok(FunctionDef { name, params, return_type, generics, body, asyncness, qualifiers, vis, docs, attrs })
}

fn convert_function_declaration(
    signature: &syn::Signature,
    visibility: &syn::Visibility,
    docs: Vec<String>,
    attrs: Vec<Attribute>,
) -> syn::Result<crate::FunctionDeclaration> {
    let generics = convert_generics(&signature.generics)?;
    Ok(crate::FunctionDeclaration {
        name: signature.ident.to_string(),
        params: signature
            .inputs
            .iter()
            .map(|arg| {
                Ok(crate::DeclaredParam {
                    param: convert_fn_arg(arg)?,
                    attrs: convert_attributes(fn_arg_attributes(arg)),
                })
            })
            .collect::<syn::Result<_>>()?,
        return_type: convert_return_type(&signature.output)?,
        generics,
        shorthand_receiver: signature.receiver().is_some_and(|receiver| receiver.colon_token.is_none()),
        variadic: signature.variadic.as_ref().map(|argument| -> syn::Result<_> {
            Ok(crate::FunctionVariadic {
                attrs: convert_attributes(&argument.attrs),
                pattern: argument.pat.as_ref().map(|(pattern, _)| convert_pattern(pattern)).transpose()?,
            })
        }).transpose()?,
        asyncness: source_signature_is_async(signature),
        qualifiers: FunctionQualifiers {
            constness: signature.constness.is_some(),
            unsafety: signature.unsafety.is_some(),
            abi: signature.abi.as_ref().map(convert_abi),
            explicit_return_type: matches!(signature.output, syn::ReturnType::Type(..)),
        },
        vis: convert_visibility(visibility),
        docs,
        attrs,
        safe: false,
    })
}

/// Read the async marker independently of parameter and type conversion.
pub fn source_signature_is_async(signature: &syn::Signature) -> bool {
    signature.asyncness.is_some()
}

fn convert_fn_arg(arg: &syn::FnArg) -> syn::Result<Param> {
    origins::try_node(Origin::Parameter, || lower_fn_arg(arg))
}

fn fn_arg_attributes(arg: &syn::FnArg) -> &[syn::Attribute] {
    match arg {
        syn::FnArg::Receiver(receiver) => &receiver.attrs,
        syn::FnArg::Typed(parameter) => &parameter.attrs,
    }
}

fn lower_fn_arg(arg: &syn::FnArg) -> syn::Result<Param> {
    match arg {
        syn::FnArg::Receiver(receiver) => {
            let ty = if receiver.colon_token.is_some() {
                convert_type(&receiver.ty)?
            } else {
                origins::try_node(Origin::SyntheticType, || {
                    origins::without_origins(|| convert_type(&receiver.ty))
                })?
            };
            Ok(Param {
                name: "self".to_string(),
                ty,
                mutable: receiver.reference.is_none() && receiver.mutability.is_some(),
                pattern: None,
            })
        }
        syn::FnArg::Typed(parameter) => {
            let (name, mutable, pattern) = match parameter.pat.as_ref() {
                Pat::Ident(binding)
                    if binding.attrs.is_empty()
                        && binding.by_ref.is_none()
                        && binding.subpat.is_none() =>
                {
                    (binding.ident.to_string(), binding.mutability.is_some(), None)
                }
                Pat::Wild(wildcard) if wildcard.attrs.is_empty() => ("_".to_string(), false, None),
                other => (String::new(), false, Some(convert_pattern(other)?)),
            };
            Ok(Param {
                name,
                ty: convert_type(&parameter.ty)?,
                mutable,
                pattern,
            })
        }
    }
}

/// Convert the type and generic parameters of an impl without its items.
pub fn convert_impl_header(item: &syn::ItemImpl) -> syn::Result<ImplBlock> {
    if item.defaultness.is_some()
        || item
            .trait_
            .as_ref()
            .is_some_and(|(negation, _, _)| negation.is_some())
    {
        return Err(syn::Error::new_spanned(item, "unsupported impl modifier"));
    }
    let generics = convert_generics(&item.generics)?;
    let trait_impl = item.trait_.as_ref()
        .map(|(_, path, _)| origins::try_node(Origin::Type, || convert_type_path(path)))
        .transpose()?;
    let target = convert_type(&item.self_ty)?;
    Ok(ImplBlock {
        unsafety: item.unsafety.is_some(),
        attrs: convert_attributes(&item.attrs),
        target,
        generics,
        items: Vec::new(),
        trait_impl,
    })
}

/// Lower an implementation and its members at the source boundary.
pub fn convert_implementation(item: &syn::ItemImpl) -> syn::Result<ImplBlock> {
    origins::try_node(Origin::Impl, || lower_implementation(item))
}

fn lower_implementation(item: &syn::ItemImpl) -> syn::Result<ImplBlock> {
    let mut block = convert_impl_header(item)?;
    block.items = item
        .items
        .iter()
        .map(convert_impl_item)
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(block)
}

fn convert_impl_item(item: &SynImplItem) -> syn::Result<ImplItem> {
    match item {
        SynImplItem::Fn(method) if method.defaultness.is_none() => {
            convert_function_body(&method.sig, &method.block, &method.vis, &method.attrs)
                .map(ImplItem::Method)
        }
        SynImplItem::Const(item_const)
            if item_const.defaultness.is_none()
                && item_const.generics.params.is_empty()
                && item_const.generics.where_clause.is_none() =>
        {
            Ok(ImplItem::AssocConst(ConstDef {
                name: item_const.ident.to_string(),
                ty: convert_type(&item_const.ty)?,
                value: convert_expr(&item_const.expr)?,
                vis: convert_visibility(&item_const.vis),
                docs: Vec::new(),
                attrs: convert_attributes(&item_const.attrs),
            }))
        }
        SynImplItem::Type(item_type) if item_type.defaultness.is_none() => {
            Ok(ImplItem::AssocType(TypeAlias {
                name: item_type.ident.to_string(),
                generics: convert_generics(&item_type.generics)?,
                target: convert_type(&item_type.ty)?,
                vis: convert_visibility(&item_type.vis),
                docs: Vec::new(),
                attrs: convert_attributes(&item_type.attrs),
            }))
        }
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported impl item kind in RustLightAST parser",
        )),
    }
}

/// Convert a parsed import at the Rust source boundary.
pub fn convert_use(item: &syn::ItemUse) -> UseStatement {
    UseStatement {
        tree: convert_use_tree(&item.tree),
        absolute: item.leading_colon.is_some(),
        vis: convert_visibility(&item.vis),
        attrs: item.attrs.iter().map(attribute_to_raw_source).collect(),
    }
}

fn convert_use_tree(tree: &syn::UseTree) -> UseTree {
    match tree {
        syn::UseTree::Path(path) => UseTree::Path {
            segment: path.ident.to_string(),
            tree: Box::new(convert_use_tree(&path.tree)),
        },
        syn::UseTree::Name(name) => UseTree::Name(name.ident.to_string()),
        syn::UseTree::Glob(_) => UseTree::Glob,
        syn::UseTree::Group(group) => UseTree::Group(group.items.iter().map(convert_use_tree).collect()),
        syn::UseTree::Rename(rename) => UseTree::Rename {
            name: rename.ident.to_string(),
            alias: rename.rename.to_string(),
        },
    }
}

fn convert_return_type(output: &ReturnType) -> syn::Result<Type> {
    match output {
        ReturnType::Default => Ok(origins::node(Origin::SyntheticType, || Type::Unit)),
        ReturnType::Type(_, ty) => convert_type(ty),
    }
}

/// Parse an isolated Rust type into the same model used by module declarations.
pub fn parse_rust_type(source: &str) -> syn::Result<Type> {
    convert_type(&syn::parse_str(source)?)
}

/// Convert a parsed Rust type into the same model used in declarations.
pub fn convert_type(ty: &SynType) -> syn::Result<Type> {
    if let SynType::Group(group) = ty {
        return convert_type(&group.elem);
    }
    origins::try_node(Origin::Type, || lower_type(ty))
}

fn lower_type(ty: &SynType) -> syn::Result<Type> {
    match ty {
        SynType::Path(type_path) => match &type_path.qself {
            Some(qualification) => {
                let (path, qualification) = convert_qualified_path(&type_path.path, qualification)?;
                Ok(Type::AssociatedPath(path, qualification))
            }
            None => convert_type_path(&type_path.path),
        },
        SynType::BareFn(function) => convert_bare_fn(function),
        SynType::Ptr(pointer) => Ok(Type::RawPointer(
            Box::new(convert_type(&pointer.elem)?),
            if pointer.mutability.is_some() {
                crate::PointerKind::Mut
            } else {
                crate::PointerKind::Const
            },
        )),
        SynType::Reference(TypeReference {
            elem,
            lifetime,
            mutability,
            ..
        }) => Ok(Type::Reference(
            Box::new(convert_type(elem)?),
            lifetime.as_ref().map(ToString::to_string),
            mutability.is_some(),
        )),
        SynType::Tuple(TypeTuple { elems, .. }) => {
            if elems.is_empty() {
                Ok(Type::Unit)
            } else {
                Ok(Type::Tuple(
                    elems
                        .iter()
                        .map(convert_type)
                        .collect::<syn::Result<Vec<_>>>()?,
                ))
            }
        }
        SynType::Slice(TypeSlice { elem, .. }) => Ok(Type::Slice(Box::new(convert_type(elem)?))),
        SynType::Array(TypeArray { elem, len, .. }) => Ok(Type::Array(
            Box::new(convert_type(elem)?),
            Box::new(convert_expr(len)?),
        )),
        SynType::TraitObject(TypeTraitObject { dyn_token, bounds }) =>
            if dyn_token.is_some() {
                origins::attempt(|| convert_callable_trait_bounds(bounds.iter(), CallableTraitQualifier::Dyn)).or_else(|_| {
                    Ok(Type::TraitObject { dyn_token: true, bounds: bounds.iter().map(convert_type_bound).collect::<syn::Result<_>>()? })
                })
            } else {
                Ok(Type::TraitObject { dyn_token: false, bounds: bounds.iter().map(convert_type_bound).collect::<syn::Result<_>>()? })
            },
        SynType::ImplTrait(TypeImplTrait { bounds, .. }) => {
            if let (Some(TypeParamBound::Trait(bound)), 1) = (bounds.first(), bounds.len()) {
                if let Some(opaque) = convert_associated_impl_trait(bound)? {
                    return Ok(opaque);
                }
            }
            origins::attempt(|| convert_callable_trait_bounds(bounds.iter(), CallableTraitQualifier::Impl))
                .or_else(|_| Ok(Type::ImplTraitBounds(bounds.iter().map(convert_type_bound).collect::<syn::Result<_>>()?)))
        }
        SynType::Never(_) => Ok(Type::Never),
        SynType::Infer(_) => Ok(Type::Infer),
        SynType::Paren(TypeParen { elem, .. }) => Ok(Type::Parenthesized(Box::new(convert_type(elem)?))),
        SynType::Group(TypeGroup { elem, .. }) => convert_type(elem),
        SynType::Macro(mac) => Ok(Type::Macro(Box::new(convert_macro_invocation(&mac.mac)))),
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported type syntax in RustLightAST parser",
        )),
    }
}

fn convert_abi(abi: &syn::Abi) -> FunctionAbi {
    match &abi.name {
        Some(name) => FunctionAbi::Named(name.value()),
        None => FunctionAbi::Default,
    }
}

fn convert_bare_fn(function: &syn::TypeBareFn) -> syn::Result<Type> {
    let higher_ranked = convert_higher_ranked_lifetimes(function.lifetimes.as_ref())?;
    let inputs = function
        .inputs
        .iter()
        .map(|input| {
            Ok(crate::BareFnParam {
                attrs: convert_attributes(&input.attrs),
                name: input.name.as_ref().map(|(name, _)| name.to_string()),
                ty: convert_type(&input.ty)?,
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let variadic = function.variadic.as_ref().map(|argument| crate::BareFnVariadic {
        attrs: convert_attributes(&argument.attrs),
        name: argument.name.as_ref().map(|(name, _)| name.to_string()),
    });
    let output = match &function.output {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) => Some(convert_type(ty)?),
    };
    Ok(Type::BareFn(Box::new(crate::BareFnType {
        higher_ranked,
        unsafety: function.unsafety.is_some(),
        abi: function.abi.as_ref().map(convert_abi),
        inputs,
        variadic,
        output,
    })))
}

fn convert_associated_impl_trait(bound: &TraitBound) -> syn::Result<Option<Type>> {
    if bound.lifetimes.is_some() || !matches!(bound.modifier, syn::TraitBoundModifier::None) {
        return Ok(None);
    }
    let mut segments = bound.path.segments.iter();
    let Some(last) = segments.next_back() else {
        return Ok(None);
    };
    let PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return Ok(None);
    };
    if arguments.args.is_empty() || segments.any(|segment| !segment.arguments.is_none()) {
        return Ok(None);
    }
    let associated = arguments.args.iter().map(|argument| match argument {
        SynGenericArgument::AssocType(assoc) if assoc.generics.is_none() => Some(assoc),
        _ => None,
    }).collect::<Option<Vec<_>>>();
    let Some(associated) = associated else { return Ok(None) };
    let associated_types = associated.into_iter().map(|assoc| {
        Ok(AssociatedTypeBinding { name: assoc.ident.to_string(), ty: convert_type(&assoc.ty)? })
    }).collect::<syn::Result<_>>()?;
    let mut trait_path = bound_path_segments(&bound.path, bound.path.segments.len() - 1);
    trait_path.push(last.ident.to_string());
    Ok(Some(Type::ImplTrait {
        trait_path,
        associated_types,
    }))
}

fn convert_callable_trait_bounds<'a>(
    bounds: impl Iterator<Item = &'a TypeParamBound>,
    qualifier: CallableTraitQualifier,
) -> syn::Result<Type> {
    let mut signature = None;
    let mut clone = false;
    let mut leading_bounds = Vec::new();
    let mut trailing_bounds = Vec::new();

    for bound in bounds {
        let additional = match bound {
            TypeParamBound::Lifetime(lifetime) => CallableBound::Lifetime(lifetime.to_string()),
            TypeParamBound::Trait(trait_bound)
                if matches!(trait_bound.modifier, syn::TraitBoundModifier::None) =>
            {
                let path = &trait_bound.path;
                let Some(last) = path.segments.last() else {
                    return Err(syn::Error::new_spanned(path, "empty callable trait path"));
                };
                if matches!(last.arguments, PathArguments::Parenthesized(_)) {
                    if signature.replace(trait_bound).is_some() {
                        return Err(syn::Error::new_spanned(
                            bound,
                            "multiple callable trait bounds",
                        ));
                    }
                    continue;
                }
                if !matches!(qualifier, CallableTraitQualifier::Impl)
                    || clone
                    || trait_bound.lifetimes.is_some()
                    || !last.arguments.is_none()
                    || !is_native_callable_bound_path(path, "clone", "Clone")
                {
                    return Err(syn::Error::new_spanned(
                        bound,
                        "unsupported callable trait bound",
                    ));
                }
                clone = true;
                CallableBound::Trait(bound_path_segments(path, path.segments.len()))
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    bound,
                    "unsupported callable trait bound",
                ))
            }
        };
        if signature.is_some() {
            trailing_bounds.push(additional);
        } else {
            leading_bounds.push(additional);
        }
    }
    let Some(signature) = signature else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "missing callable trait bound in RustLightAST parser",
        ));
    };
    if clone && !is_native_callable_bound_path(&signature.path, "ops", "Fn") {
        return Err(syn::Error::new_spanned(
            signature,
            "Clone is supported with the Fn call trait",
        ));
    }
    convert_callable_trait_bound(signature, qualifier, leading_bounds, trailing_bounds)
}

fn convert_callable_trait_bound(
    trait_bound: &TraitBound,
    qualifier: CallableTraitQualifier,
    leading_bounds: Vec<CallableBound>,
    trailing_bounds: Vec<CallableBound>,
) -> syn::Result<Type> {
    let last =
        trait_bound.path.segments.last().ok_or_else(|| {
            syn::Error::new_spanned(&trait_bound.path, "empty callable trait path")
        })?;
    let PathArguments::Parenthesized(ParenthesizedGenericArguments { inputs, output, .. }) =
        &last.arguments
    else {
        return Err(syn::Error::new_spanned(
            &last.arguments,
            "unsupported non-callable trait bound in RustLightAST parser",
        ));
    };
    let mut trait_path =
        bound_path_segments(&trait_bound.path, trait_bound.path.segments.len() - 1);
    trait_path.push(last.ident.to_string());

    let args = inputs
        .iter()
        .map(convert_type)
        .collect::<syn::Result<Vec<_>>>()?;
    let return_type = Box::new(convert_return_type(output)?);
    let higher_ranked = convert_higher_ranked_lifetimes(trait_bound.lifetimes.as_ref())?;
    Ok(Type::CallableTrait(CallableTraitType {
        qualifier,
        parenthesized: trait_bound.paren_token.is_some(),
        explicit_return_type: matches!(output, ReturnType::Type(..)),
        trait_path,
        args,
        return_type,
        leading_bounds,
        trailing_bounds,
        higher_ranked,
    }))
}

fn bound_path_segments(path: &syn::Path, segments: usize) -> Vec<String> {
    path.leading_colon
        .iter()
        .map(|_| String::new())
        .chain(path.segments.iter().take(segments).map(path_segment_source))
        .collect()
}

fn is_native_callable_bound_path(path: &syn::Path, module: &str, name: &str) -> bool {
    match path.segments.len() {
        1 => path.leading_colon.is_none() && path.segments[0].ident == name,
        3 => {
            let root = &path.segments[0];
            let parent = &path.segments[1];
            (root.ident == "std" || root.ident == "core")
                && root.arguments.is_none()
                && parent.ident == module
                && parent.arguments.is_none()
                && path.segments[2].ident == name
        }
        _ => false,
    }
}

fn convert_generic_arguments(
    arguments: &AngleBracketedGenericArguments,
) -> syn::Result<Vec<GenericArgument>> {
    origins::attempt(|| arguments.args.iter().map(convert_generic_argument).collect())
}

fn convert_generic_argument(argument: &SynGenericArgument) -> syn::Result<GenericArgument> {
    match argument {
        SynGenericArgument::Type(ty) => convert_type(ty).map(GenericArgument::Type),
        SynGenericArgument::Lifetime(lifetime) => {
            Ok(GenericArgument::Lifetime(lifetime.to_string()))
        }
        SynGenericArgument::Const(expr) => {
            Ok(GenericArgument::Const(Box::new(convert_expr(expr)?)))
        }
        // DEFENSE: dropping an associated type equality or constraint changes the type.
        _ => Err(syn::Error::new_spanned(
            argument,
            "unsupported associated generic argument",
        )),
    }
}

fn convert_type_path(path: &syn::Path) -> syn::Result<Type> {
    let last = path
        .segments
        .last()
        .ok_or_else(|| syn::Error::new_spanned(path, "empty path"))?;
    if path
        .segments
        .iter()
        .rev()
        .skip(1)
        .any(|segment| !segment.arguments.is_none())
    {
        let path = convert_namespace_path(path)?;
        return Ok(Type::PathWithArguments(path.segments, path.arguments));
    }
    let generic_args = match &last.arguments {
        PathArguments::None => Vec::new(),
        PathArguments::AngleBracketed(arguments) => convert_generic_arguments(arguments)?,
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "unsupported path arguments in RustLightAST parser",
            ));
        }
    };

    let segments: Vec<_> = path
        .leading_colon
        .iter()
        .map(|_| String::new())
        .chain(
            path.segments
                .iter()
                .map(|segment| segment.ident.to_string()),
        )
        .collect();
    if !generic_args.is_empty() {
        Ok(Type::Generic(segments.join("::"), generic_args))
    } else if segments.len() > 1 {
        Ok(Type::Path(segments))
    } else {
        Ok(Type::Named(last.ident.to_string()))
    }
}

/// Convert a parsed body into RustLight expressions and statements.
pub fn convert_block(block: &syn::Block) -> syn::Result<Block> {
    origins::try_node(Origin::Block, || lower_block(block))
}

fn lower_block(block: &syn::Block) -> syn::Result<Block> {
    let mut stmts = Vec::new();
    let mut tail_expr = None;

    for (index, stmt) in block.stmts.iter().enumerate() {
        let last = index + 1 == block.stmts.len();
        match stmt {
            Stmt::Expr(expr, None) if last => {
                tail_expr = Some(Box::new(convert_expr(expr)?));
            }
            Stmt::Macro(statement) if is_tail_macro(statement, last) => {
                let expression = origins::node(Origin::Expression, || {
                    Expr::Macro(Box::new(crate::MacroExpr {
                        attrs: statement.attrs.iter().map(attribute_to_raw_source).collect(),
                        invocation: convert_macro_invocation_with_attributes(
                            &statement.mac, !statement.attrs.is_empty(),
                        ),
                    }))
                });
                tail_expr = Some(Box::new(expression));
            }
            statement => {
                stmts.push(origins::try_node(
                    Origin::Statement,
                    || lower_statement(statement),
                )?);
            }
        }
    }

    Ok(Block {
        stmts,
        expr: tail_expr,
    })
}

fn lower_statement(statement: &syn::Stmt) -> syn::Result<Statement> {
    Ok(match statement {
        Stmt::Local(local) => return convert_local(local),
        Stmt::Item(SynItem::Macro(item)) => Statement::Macro(convert_item_macro(item)),
        Stmt::Item(item) => Statement::Item(Box::new(
            origins::try_node(Origin::Item, || convert_item(item))?,
        )),
        Stmt::Expr(expr, _) => Statement::Expr(convert_expr(expr)?),
        Stmt::Macro(statement) => Statement::Macro(crate::MacroStmt {
            attrs: statement.attrs.iter().map(attribute_to_raw_source).collect(),
            name: None,
            invocation: convert_macro_invocation_with_attributes(
                &statement.mac, !statement.attrs.is_empty(),
            ),
            function_template: None,
            semicolon: statement.semi_token.is_some(),
        }),
    })
}

fn convert_item_macro(item: &syn::ItemMacro) -> crate::MacroStmt {
    crate::MacroStmt {
        attrs: item.attrs.iter().map(attribute_to_raw_source).collect(),
        name: item.ident.as_ref().map(ToString::to_string),
        invocation: convert_macro_invocation_with_attributes(&item.mac, !item.attrs.is_empty()),
        function_template: macro_syntax::item_function_template(item),
        semicolon: item.semi_token.is_some(),
    }
}

/// A trailing paren/bracket macro without a semicolon occupies expression position.
pub fn is_tail_macro(statement: &syn::StmtMacro, last: bool) -> bool {
    last && statement.semi_token.is_none()
        && !matches!(statement.mac.delimiter, syn::MacroDelimiter::Brace(_))
}

/// Preserve the invocation spelling and recognize supported input forms.
pub fn convert_macro_invocation(mac: &syn::Macro) -> crate::MacroInvocation {
    convert_macro_invocation_with_attributes(mac, false)
}

fn convert_macro_invocation_with_attributes(
    mac: &syn::Macro,
    has_attributes: bool,
) -> crate::MacroInvocation {
    origins::macro_invocation(has_attributes, || crate::MacroInvocation {
        path: path_segments(&mac.path),
        delimiter: match mac.delimiter {
            syn::MacroDelimiter::Paren(_) => crate::MacroDelimiter::Parenthesis,
            syn::MacroDelimiter::Brace(_) => crate::MacroDelimiter::Brace,
            syn::MacroDelimiter::Bracket(_) => crate::MacroDelimiter::Bracket,
        },
        tokens: macro_syntax::input(mac.tokens.to_string(), &mac.tokens),
    })
}

/// Read the comma-separated expressions recognized at the input boundary.
pub fn parse_macro_expression_arguments(mac: &crate::MacroInvocation) -> syn::Result<Vec<Expr>> {
    mac.expression_arguments().map(<[_]>::to_vec).ok_or_else(|| {
        syn::Error::new(proc_macro2::Span::call_site(), "expected macro expression arguments")
    })
}

fn convert_local(local: &syn::Local) -> syn::Result<Statement> {
    let attrs = local
        .attrs
        .iter()
        .map(|attr| normalize_tokens(attr.to_token_stream()))
        .collect();
    if let Some(LocalInit {
        expr,
        diverge: Some((_, else_expr)),
        ..
    }) = &local.init
    {
        return Ok(Statement::LetElse {
            attrs,
            pattern: pattern::convert_pattern(&local.pat)?,
            value: Box::new(convert_expr(expr)?),
            else_block: expr_to_block(else_expr)?,
        });
    }

    origins::try_node(Origin::Binding, || lower_let_binding(local, attrs))
}

fn lower_let_binding(local: &syn::Local, attrs: Vec<String>) -> syn::Result<Statement> {
    let (pattern, ty) = match &local.pat {
        Pat::Type(typed) if typed.attrs.is_empty() => (
            origins::try_node(Origin::PatternAlias, || convert_pattern(&typed.pat))?,
            Some(convert_type(&typed.ty)?),
        ),
        pat => (convert_pattern(pat)?, None),
    };

    let init = match &local.init {
        Some(LocalInit { expr, .. }) => Some(convert_expr(expr)?),
        None => None,
    };

    Ok(Statement::Let(LetStmt {
        attrs,
        pattern,
        ty,
        init,
    }))
}

/// Convert a parsed Rust expression without reparsing its source.
pub fn convert_expr(expr: &syn::Expr) -> syn::Result<Expr> {
    origins::try_node(Origin::Expression, || lower_expr(expr))
}

fn lower_expr(expr: &syn::Expr) -> syn::Result<Expr> {
    match expr {
        syn::Expr::Path(expr_path) if expr_path.attrs.is_empty() => convert_expr_path(expr_path),
        syn::Expr::Call(ExprCall {
            attrs, func, args, ..
        }) if attrs.is_empty() => Ok(Expr::Call(
            Box::new(convert_expr(func)?),
            args.iter()
                .map(convert_expr)
                .collect::<syn::Result<Vec<_>>>()?,
        )),
        syn::Expr::MethodCall(ExprMethodCall {
            attrs,
            receiver,
            method,
            turbofish,
            args,
            ..
        }) if attrs.is_empty() => {
            let method = crate::MethodName {
                name: method.to_string(),
                generic_arguments: turbofish.as_ref().map(convert_generic_arguments).transpose()?,
            };
            Ok(Expr::MethodCall(
                Box::new(convert_expr(receiver)?),
                method,
                args.iter()
                    .map(convert_expr)
                    .collect::<syn::Result<Vec<_>>>()?,
            ))
        }
        syn::Expr::Match(ExprMatch {
            attrs, expr, arms, ..
        }) if attrs.is_empty() => Ok(Expr::Match {
            expr: Box::new(convert_expr(expr)?),
            arms: arms
                .iter()
                .map(convert_match_arm)
                .collect::<syn::Result<Vec<_>>>()?,
        }),
        syn::Expr::Block(ExprBlock {
            attrs,
            label,
            block,
            ..
        }) if attrs.is_empty() => {
            let body = convert_block(block)?;
            Ok(match label {
                Some(label) => Expr::LabeledBlock {
                    label: label.name.ident.to_string(),
                    body,
                },
                None => Expr::Block(body),
            })
        }
        syn::Expr::Loop(expr) if expr.attrs.is_empty() => Ok(Expr::Loop(
            Box::new(convert_block(&expr.body)?),
            expr.label
                .as_ref()
                .map(|label| label.name.ident.to_string()),
        )),
        syn::Expr::While(expr) if expr.attrs.is_empty() => {
            Ok(Expr::While {
                label: expr
                    .label
                    .as_ref()
                    .map(|label| label.name.ident.to_string()),
                condition: Box::new(convert_expr(&expr.cond)?),
                body: convert_block(&expr.body)?,
            })
        }
        syn::Expr::Let(condition) if condition.attrs.is_empty() => Ok(Expr::Let {
            pattern: pattern::convert_pattern(&condition.pat)?,
            value: Box::new(convert_expr(&condition.expr)?),
        }),
        syn::Expr::ForLoop(expr) if expr.attrs.is_empty() => Ok(Expr::For {
            label: expr
                .label
                .as_ref()
                .map(|label| label.name.ident.to_string()),
            pattern: pattern::convert_pattern(&expr.pat)?,
            iter: Box::new(convert_expr(&expr.expr)?),
            body: convert_block(&expr.body)?,
        }),
        syn::Expr::Return(expr) if expr.attrs.is_empty() => Ok(Expr::Return(
            expr.expr
                .as_deref()
                .map(convert_expr)
                .transpose()?
                .map(Box::new),
        )),
        syn::Expr::Break(expr) if expr.attrs.is_empty() => Ok(Expr::Break {
            label: expr.label.as_ref().map(|label| label.ident.to_string()),
            value: expr
                .expr
                .as_deref()
                .map(convert_expr)
                .transpose()?
                .map(Box::new),
        }),
        syn::Expr::Continue(expr) if expr.attrs.is_empty() => Ok(Expr::Continue(
            expr.label.as_ref().map(|label| label.ident.to_string()),
        )),
        syn::Expr::Reference(ExprReference {
            attrs,
            expr,
            mutability,
            ..
        }) if attrs.is_empty() => Ok(Expr::Reference(
            Box::new(convert_expr(expr)?),
            true,
            mutability.is_some(),
        )),
        syn::Expr::Cast(ExprCast {
            attrs, expr, ty, ..
        }) if attrs.is_empty() => Ok(Expr::Cast(Box::new(convert_expr(expr)?), convert_type(ty)?)),
        syn::Expr::Async(expr) => {
            if !expr.attrs.is_empty() {
                return Err(syn::Error::new_spanned(
                    expr,
                    "unsupported async block attributes",
                ));
            }
            Ok(Expr::Async {
                body: convert_block(&expr.block)?,
                move_capture: expr.capture.is_some(),
            })
        }
        syn::Expr::Await(expr) if expr.attrs.is_empty() => {
            Ok(Expr::Await(Box::new(convert_expr(&expr.base)?)))
        }
        syn::Expr::Try(expr) if expr.attrs.is_empty() => {
            Ok(Expr::Try(Box::new(convert_expr(&expr.expr)?)))
        }
        syn::Expr::Repeat(expr) if expr.attrs.is_empty() => Ok(Expr::Repeat {
            value: Box::new(convert_expr(&expr.expr)?),
            len: Box::new(convert_expr(&expr.len)?),
        }),
        syn::Expr::Range(expr) if expr.attrs.is_empty() => {
            let (start, end) = convert_range(expr)?;
            Ok(Expr::Range { start, end })
        }
        syn::Expr::Paren(ExprParen { attrs, expr, .. })
        | syn::Expr::Group(ExprGroup { attrs, expr, .. })
            if attrs.is_empty() =>
        {
            Ok(Expr::Parenthesized(Box::new(convert_expr(expr)?)))
        }
        syn::Expr::Binary(ExprBinary {
            attrs,
            left,
            op,
            right,
            ..
        }) if attrs.is_empty() => Ok(Expr::BinaryOp(
            Box::new(convert_expr(left)?),
            op.to_token_stream().to_string(),
            Box::new(convert_expr(right)?),
        )),
        syn::Expr::Unary(ExprUnary {
            attrs, op, expr, ..
        }) if attrs.is_empty() => Ok(Expr::UnaryOp(
            op.to_token_stream().to_string(),
            Box::new(convert_expr(expr)?),
        )),
        syn::Expr::Assign(ExprAssign {
            attrs, left, right, ..
        }) if attrs.is_empty() => Ok(Expr::Assign(
            Box::new(convert_expr(left)?),
            Box::new(convert_expr(right)?),
        )),
        syn::Expr::If(ExprIf {
            attrs,
            cond,
            then_branch,
            else_branch,
            ..
        }) if attrs.is_empty() => {
            let condition = match cond.as_ref() {
                syn::Expr::Let(condition) if condition.attrs.is_empty() => {
                    let pattern = origins::try_node(Origin::Condition, || {
                        convert_pattern(&condition.pat)
                    })?;
                    (Some(pattern), Box::new(convert_expr(&condition.expr)?))
                }
                condition => (None, Box::new(convert_expr(condition)?)),
            };
            let then_branch = convert_block(then_branch)?;
            let else_branch = else_branch
                .as_ref()
                .map(|(_, expr)| expr_to_block(expr))
                .transpose()?;
            Ok(match condition {
                (Some(pattern), value) => Expr::IfLet { pattern, value, then_branch, else_branch },
                (None, condition) => Expr::If { condition, then_branch, else_branch },
            })
        }
        syn::Expr::Lit(ExprLit { attrs, lit, .. }) if attrs.is_empty() => {
            Ok(Expr::Literal(convert_literal(lit)?))
        }
        syn::Expr::Tuple(ExprTuple { attrs, elems, .. }) if attrs.is_empty() => Ok(Expr::Tuple(
            elems
                .iter()
                .map(convert_expr)
                .collect::<syn::Result<Vec<_>>>()?,
        )),
        syn::Expr::Index(ExprIndex {
            attrs, expr, index, ..
        }) if attrs.is_empty() => Ok(Expr::Index(
            Box::new(convert_expr(expr)?),
            Box::new(convert_expr(index)?),
        )),
        syn::Expr::Field(ExprField {
            attrs,
            base,
            member,
            ..
        }) if attrs.is_empty() => Ok(Expr::Field {
            base: Box::new(convert_expr(base)?),
            member: convert_member(member),
        }),
        syn::Expr::Struct(record) if record.attrs.is_empty() => {
            if record.qself.is_some() {
                return Err(syn::Error::new_spanned(
                    record,
                    "unsupported qualified record constructor",
                ));
            }
            Ok(Expr::Record {
                path: convert_record_path(&record.path)?,
                fields: record
                    .fields
                    .iter()
                    .map(convert_record_field)
                    .collect::<syn::Result<_>>()?,
                rest: record
                    .rest
                    .as_deref()
                    .map(convert_expr)
                    .transpose()?
                    .map(Box::new),
            })
        }
        syn::Expr::Array(ExprArray { attrs, elems, .. }) if attrs.is_empty() => Ok(Expr::Array(
            elems
                .iter()
                .map(convert_expr)
                .collect::<syn::Result<Vec<_>>>()?,
        )),
        syn::Expr::Macro(expression) => Ok(Expr::Macro(Box::new(crate::MacroExpr {
            attrs: expression.attrs.iter().map(|attr| attr.to_token_stream().to_string()).collect(),
            invocation: convert_macro_invocation_with_attributes(&expression.mac, !expression.attrs.is_empty()),
        }))),
        syn::Expr::Closure(ExprClosure {
            attrs,
            capture,
            asyncness,
            constness: None,
            lifetimes: None,
            movability: None,
            inputs,
            output,
            body,
            ..
        }) if attrs.is_empty() => {
            let is_move = capture.is_some();
            let params = inputs
                .iter()
                .map(convert_closure_param)
                .collect::<syn::Result<Vec<_>>>()?;
            let body = Box::new(convert_expr(body)?);
            if asyncness.is_some() {
                let return_type = match output {
                    ReturnType::Default => None,
                    ReturnType::Type(_, ty) => Some(convert_type(ty)?),
                };
                return Ok(Expr::AsyncClosure {
                    params,
                    return_type,
                    body,
                    move_capture: is_move,
                });
            }
            match output {
                ReturnType::Default => Ok(Expr::Closure(params, body, is_move)),
                ReturnType::Type(_, ty) => {
                    Ok(Expr::TypedClosure(params, convert_type(ty)?, body, is_move))
                }
            }
        }
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported expression syntax in RustLightAST parser",
        )),
    }
}

fn convert_closure_param(pattern: &syn::Pat) -> syn::Result<ClosureParam> {
    origins::try_node(Origin::ClosureParameter, || lower_closure_param(pattern))
}

fn lower_closure_param(pattern: &syn::Pat) -> syn::Result<ClosureParam> {
    match pattern {
        syn::Pat::Type(typed) if typed.attrs.is_empty() => Ok(ClosureParam::typed(
            convert_pattern(&typed.pat)?,
            convert_type(&typed.ty)?,
        )),
        syn::Pat::Type(typed) => Err(syn::Error::new_spanned(
            typed,
            "unsupported attributes on typed closure parameter",
        )),
        other => Ok(ClosureParam::untyped(convert_pattern(other)?)),
    }
}

fn expr_to_block(expr: &syn::Expr) -> syn::Result<Block> {
    if let syn::Expr::Block(ExprBlock { attrs, label: None, block, .. }) = expr {
        if attrs.is_empty() {
            return origins::try_node(Origin::BlockExpression, || convert_block(block));
        }
    }
    origins::try_node(Origin::SyntheticBlock, || {
        Ok(Block { stmts: Vec::new(), expr: Some(Box::new(convert_expr(expr)?)) })
    })
}

fn convert_match_arm(arm: &syn::Arm) -> syn::Result<MatchArm> {
    if let Some(attr) = arm.attrs.first() {
        return Err(syn::Error::new_spanned(
            attr,
            "unsupported attributes on match arm",
        ));
    }
    Ok(MatchArm {
        pattern: pattern::convert_pattern(&arm.pat)?,
        guard: arm
            .guard
            .as_ref()
            .map(|(_, expr)| convert_expr(expr))
            .transpose()?,
        body: expr_to_block(&arm.body)?,
        braced: matches!(
            arm.body.as_ref(),
            syn::Expr::Block(ExprBlock { label: None, .. })
        ),
        comma: arm.comma.is_some(),
    })
}

fn convert_expr_path(expr_path: &ExprPath) -> syn::Result<Expr> {
    if let Some(qualification) = &expr_path.qself {
        let (path, qualification) = convert_qualified_path(&expr_path.path, qualification)?;
        return Ok(Expr::Path(path, PathType::Qualified(qualification)));
    }

    let path = convert_namespace_path(&expr_path.path)?;
    if path.arguments.is_empty() {
        if let [segment] = path.segments.as_slice() {
            return Ok(Expr::Ident(segment.clone()));
        }
        Ok(Expr::Path(path.segments, PathType::Namespace))
    } else {
        Ok(Expr::Path(path.segments, PathType::NamespaceWithTypeArguments(path.arguments)))
    }
}

fn convert_qualified_path(
    path: &syn::Path,
    qualification: &syn::QSelf,
) -> syn::Result<(Vec<String>, crate::PathQualification)> {
    let self_type = crate::QualifiedSelfType::Type(Box::new(convert_type(&qualification.ty)?));
    let root = usize::from(path.leading_colon.is_some());
    let (trait_path, trait_arguments) = if qualification.as_token.is_some() {
        let path = lower_path_segments(path.segments.iter().take(qualification.position), root)?;
        (Some(path.segments), path.arguments)
    } else {
        (None, Vec::new())
    };
    let associated = lower_path_segments(path.segments.iter().skip(qualification.position), 0)?;
    Ok((
        associated.segments,
        crate::PathQualification {
            self_type,
            trait_path,
            trait_arguments,
            associated_arguments: associated.arguments,
        },
    ))
}

fn convert_namespace_path(path: &syn::Path) -> syn::Result<crate::PatternPath> {
    lower_path_segments(path.segments.iter(), usize::from(path.leading_colon.is_some()))
}

fn lower_path_segments<'a>(
    segments: impl Iterator<Item = &'a syn::PathSegment>,
    root: usize,
) -> syn::Result<crate::PatternPath> {
    let mut path = crate::PatternPath {
        segments: vec![String::new(); root],
        arguments: Vec::new(),
    };
    for segment in segments {
        match &segment.arguments {
            PathArguments::None => {}
            PathArguments::AngleBracketed(arguments) => {
                path.arguments.push(crate::PathTypeArguments {
                    segment: path.segments.len(),
                    arguments: convert_generic_arguments(arguments)?,
                });
            }
            other => return Err(syn::Error::new_spanned(other, "unsupported path arguments")),
        }
        path.segments.push(segment.ident.to_string());
    }
    Ok(path)
}

/// Project modeled arguments on expression path segments, including terminal turbofish.
pub fn namespace_type_arguments(path: &syn::Path) -> syn::Result<Vec<crate::PathTypeArguments>> {
    Ok(convert_namespace_path(path)?.arguments)
}

/// Lower a named or positional member at the Rust source boundary.
pub fn convert_member(member: &syn::Member) -> Member {
    match member {
        syn::Member::Named(name) => Member::Named(name.to_string()),
        syn::Member::Unnamed(index) => Member::Index(index.index),
    }
}

fn convert_record_field(field: &syn::FieldValue) -> syn::Result<RecordField> {
    // DEFENSE: a `cfg` on an initializer controls whether its value is evaluated.
    if !field.attrs.is_empty() {
        return Err(syn::Error::new_spanned(
            field,
            "unsupported attribute on record field initializer",
        ));
    }
    Ok(RecordField {
        member: convert_member(&field.member),
        value: convert_expr(&field.expr)?,
        shorthand: field.colon_token.is_none(),
    })
}

/// Lower final record path arguments when they fit the generic argument model.
pub fn convert_record_path(path: &syn::Path) -> syn::Result<RecordPath> {
    let mut modeled = convert_namespace_path(path)?;
    if let Some(last) = modeled.arguments.pop() {
        if !modeled.arguments.is_empty() || last.segment + 1 != modeled.segments.len() {
            return Err(syn::Error::new_spanned(
                path,
                "unsupported generic arguments on record path prefix",
            ));
        }
        Ok(RecordPath::WithTypeArguments {
            segments: modeled.segments,
            arguments: last.arguments,
        })
    } else {
        Ok(RecordPath::Segments(modeled.segments))
    }
}

/// Read an unparameterized path without exposing the parser's syntax tree.
/// The first segment can be a Rust path keyword; later segments must be identifiers.
pub fn parse_plain_path(source: &str) -> Result<Vec<String>, SourceParseError> {
    use syn::ext::IdentExt;

    let path = syn::parse_str::<syn::Path>(source)?;
    if let Some(segment) = path.segments.iter().find(|segment| !segment.arguments.is_none()) {
        return Err(syn::Error::new_spanned(segment, "expected a path without arguments").into());
    }
    for segment in path.segments.iter().skip(1) {
        syn::parse_str::<syn::Ident>(&segment.ident.to_string())?;
    }
    Ok(path.leading_colon.iter().map(|_| String::new())
        .chain(path.segments.iter().map(|segment| segment.ident.unraw().to_string()))
        .collect())
}

/// One named flag or string-valued setting accepted by Rust's `--cfg`.
pub enum CfgAtom {
    Flag(String),
    KeyValue { key: String, value: String },
}

/// Parse the shape of a command-line cfg atom without exposing a syntax tree.
pub fn parse_cfg_atom(source: &str) -> Result<CfgAtom, SourceParseError> {
    let invalid = || syn::Error::new(
        proc_macro2::Span::call_site(),
        "expected NAME or NAME=\"VALUE\"",
    );
    match syn::parse_str::<Meta>(source) {
        Ok(Meta::Path(path)) => {
            let name = path.get_ident().ok_or_else(invalid)?;
            Ok(CfgAtom::Flag(name.to_string()))
        }
        Ok(Meta::NameValue(pair)) => {
            let name = pair.path.get_ident().ok_or_else(invalid)?;
            let syn::Expr::Lit(syn::ExprLit { lit: Lit::Str(value), .. }) = pair.value else {
                return Err(syn::Error::new_spanned(pair.value, "expected a string value").into());
            };
            Ok(CfgAtom::KeyValue { key: name.to_string(), value: value.value() })
        }
        _ => Err(invalid().into()),
    }
}

/// Lower a parsed Rust path into namespace segments.
pub fn path_segments(path: &syn::Path) -> Vec<String> {
    path.leading_colon
        .iter()
        .map(|_| String::new())
        .chain(path.segments.iter().map(path_segment_source))
        .collect()
}

fn path_segment_source(segment: &syn::PathSegment) -> String {
    match &segment.arguments {
        PathArguments::None => segment.ident.to_string(),
        _ => normalize_path_tokens(segment.to_token_stream()),
    }
}

fn convert_range(range: &syn::ExprRange) -> syn::Result<(Option<Box<Expr>>, crate::RangeEnd)> {
    let bound = |bound: Option<&syn::Expr>| {
        bound
            .map(|bound| convert_expr(bound).map(Box::new))
            .transpose()
    };
    let start = bound(range.start.as_deref())?;
    let end = bound(range.end.as_deref())?;
    let end = match range.limits {
        syn::RangeLimits::HalfOpen(_) => crate::RangeEnd::Exclusive(end),
        syn::RangeLimits::Closed(_) => crate::RangeEnd::Inclusive(end.ok_or_else(|| {
            syn::Error::new_spanned(range, "inclusive range requires an end expression")
        })?),
    };
    Ok((start, end))
}

fn convert_literal(lit: &Lit) -> syn::Result<Literal> {
    match lit {
        Lit::Bool(bool_lit) => Ok(Literal::Bool(bool_lit.value)),
        Lit::Int(integer) => {
            let token = integer.to_string();
            let suffix = integer.suffix();
            Ok(Literal::Integer {
                digits: token[..token.len() - suffix.len()].to_owned(),
                suffix: (!suffix.is_empty()).then(|| suffix.to_owned()),
            })
        }
        Lit::Byte(byte) => Ok(Literal::Byte {
            spelling: byte.to_token_stream().to_string(),
            value: byte.value(),
        }),
        Lit::Char(character) => Ok(Literal::CharToken {
            spelling: character.to_token_stream().to_string(),
            value: character.value(),
        }),
        Lit::Str(string) => Ok(Literal::StringToken {
            spelling: string.to_token_stream().to_string(),
            value: string.value(),
        }),
        Lit::Float(float) => {
            let token = float.to_string();
            let suffix = float.suffix();
            Ok(Literal::FloatToken {
                digits: token[..token.len() - suffix.len()].to_owned(),
                suffix: (!suffix.is_empty()).then(|| suffix.to_owned()),
            })
        }
        other => Ok(Literal::Raw(other.to_token_stream().to_string())),
    }
}

/// Lower a type bound into the modeled Rust grammar.
pub fn convert_type_bound(bound: &TypeParamBound) -> syn::Result<crate::TypeBound> {
    match bound {
        TypeParamBound::Lifetime(lifetime) => Ok(crate::TypeBound::Lifetime(lifetime.to_string())),
        TypeParamBound::Trait(bound) => convert_trait_bound(bound).map(crate::TypeBound::Trait),
        _ => Err(syn::Error::new_spanned(bound, "unsupported type bound")),
    }
}

fn convert_trait_bound(bound: &TraitBound) -> syn::Result<crate::TraitBound> {
    origins::attempt(|| lower_trait_bound(bound))
}

fn lower_trait_bound(bound: &TraitBound) -> syn::Result<crate::TraitBound> {
    use crate::TraitBoundArguments;
    let higher_ranked = convert_higher_ranked_lifetimes(bound.lifetimes.as_ref())?;
    let path = bound
        .path
        .segments
        .iter()
        .map(|segment| {
            let arguments = match &segment.arguments {
                PathArguments::None => TraitBoundArguments::None,
                PathArguments::AngleBracketed(arguments) => {
                    TraitBoundArguments::Angle(convert_bound_arguments(arguments)?)
                }
                PathArguments::Parenthesized(arguments) => TraitBoundArguments::Parenthesized {
                    inputs: arguments
                        .inputs
                        .iter()
                        .map(convert_type)
                        .collect::<syn::Result<_>>()?,
                    output: match &arguments.output {
                        ReturnType::Default => None,
                        ReturnType::Type(_, ty) => Some(Box::new(convert_type(ty)?)),
                    },
                },
            };
            Ok(crate::TraitBoundSegment {
                name: segment.ident.to_string(),
                arguments,
            })
        })
        .collect::<syn::Result<_>>()?;
    Ok(crate::TraitBound {
        absolute: bound.path.leading_colon.is_some(),
        higher_ranked,
        optional: matches!(bound.modifier, syn::TraitBoundModifier::Maybe(_)),
        parenthesized: bound.paren_token.is_some(),
        path,
    })
}

fn convert_bound_arguments(
    arguments: &AngleBracketedGenericArguments,
) -> syn::Result<Vec<crate::TraitBoundArgument>> {
    arguments
        .args
        .iter()
        .map(|argument| match argument {
            SynGenericArgument::AssocType(binding) => {
                Ok(crate::TraitBoundArgument::AssociatedType {
                    name: binding.ident.to_string(),
                    arguments: binding
                        .generics
                        .as_ref()
                        .map(convert_bound_arguments)
                        .transpose()?,
                    value: convert_type(&binding.ty)?,
                })
            }
            _ => convert_generic_argument(argument).map(crate::TraitBoundArgument::Generic),
        })
        .collect()
}

/// Lower a parsed Rust generic parameter list and its where clause.
pub fn convert_generics(generics: &syn::Generics) -> syn::Result<Generics> {
    origins::attempt(|| lower_generics(generics))
}

fn lower_generics(generics: &syn::Generics) -> syn::Result<Generics> {
    let params = generics
        .params
        .iter()
        .map(|param| match param {
            syn::GenericParam::Type(type_param) if type_param.attrs.is_empty() => {
                Ok(GenericParam::Type(TypeParameter {
                    name: type_param.ident.to_string(),
                    default: type_param.default.as_ref().map(convert_type).transpose()?,
                    bounds: type_param.bounds.iter().map(convert_type_bound).collect::<syn::Result<_>>()?,
                }))
            }
            syn::GenericParam::Lifetime(param) if param.attrs.is_empty() => {
                Ok(GenericParam::Lifetime(convert_lifetime_param(param)))
            }
            syn::GenericParam::Const(param) if param.attrs.is_empty() => Ok(GenericParam::Const {
                name: param.ident.to_string(),
                ty: convert_type(&param.ty)?,
                default: param.default.as_ref().map(convert_expr).transpose()?,
            }),
            _ => Err(syn::Error::new_spanned(
                param,
                "unsupported generic parameter in RustLightAST parser",
            )),
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let mut where_predicates = Vec::new();

    if let Some(clause) = &generics.where_clause {
        for predicate in &clause.predicates {
            match predicate {
                syn::WherePredicate::Type(predicate) if predicate.lifetimes.is_none() => {
                    where_predicates.push(WherePredicate::Type {
                        subject: convert_type(&predicate.bounded_ty)?,
                        bounds: predicate.bounds.iter().map(convert_type_bound).collect::<syn::Result<_>>()?,
                    });
                }
                syn::WherePredicate::Lifetime(predicate) => {
                    where_predicates.push(WherePredicate::Lifetime {
                        subject: predicate.lifetime.to_string(),
                        bounds: predicate.bounds.iter().map(ToString::to_string).collect(),
                    });
                }
                unsupported => {
                    return Err(syn::Error::new_spanned(
                        unsupported,
                        "unsupported where predicate in RustLightAST parser",
                    ));
                }
            }
        }
    }

    Ok(Generics {
        params,
        where_predicates,
    })
}

fn convert_higher_ranked_lifetimes(
    binder: Option<&syn::BoundLifetimes>,
) -> syn::Result<Vec<LifetimeParameter>> {
    binder
        .into_iter()
        .flat_map(|binder| &binder.lifetimes)
        .map(|parameter| match parameter {
            syn::GenericParam::Lifetime(lifetime) if lifetime.attrs.is_empty() => {
                Ok(convert_lifetime_param(lifetime))
            }
            _ => Err(syn::Error::new_spanned(
                parameter,
                "unsupported higher-ranked parameter",
            )),
        })
        .collect()
}

fn convert_lifetime_param(param: &syn::LifetimeParam) -> LifetimeParameter {
    LifetimeParameter {
        name: param.lifetime.to_string(),
        bounds: param.bounds.iter().map(ToString::to_string).collect(),
    }
}

/// Convert a declaration visibility.
pub fn convert_visibility(vis: &SynVisibility) -> Visibility {
    match vis {
        SynVisibility::Public(_) => Visibility::Public,
        SynVisibility::Restricted(restricted) => Visibility::Restricted(
            restricted
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect(),
        ),
        SynVisibility::Inherited => Visibility::Private,
    }
}

/// Render an attribute in its original inner or outer position.
pub fn attribute_to_raw_source(attr: &syn::Attribute) -> String {
    let marker = match attr.style {
        syn::AttrStyle::Inner(_) => "#!",
        syn::AttrStyle::Outer => "#",
    };

    format!("{marker}[{}]", attr.meta.to_token_stream())
}

/// Convert attributes represented by RustLightAST items.
pub fn convert_attributes(attrs: &[syn::Attribute]) -> Vec<Attribute> {
    attrs.iter().map(convert_attribute).collect()
}

/// Lower data attributes independently of the fields and variants they annotate.
pub fn convert_data_attributes(attrs: &[syn::Attribute]) -> Vec<crate::DataAttribute> {
    attrs.iter().map(convert_data_attribute).collect()
}

pub fn convert_data_attribute(attr: &syn::Attribute) -> crate::DataAttribute {
    let Some(paths) = derive_paths(attr) else {
        return crate::DataAttribute::Other(convert_attribute(attr));
    };
    crate::DataAttribute::Derive {
        trailing_comma: paths.trailing_punct(),
        paths: paths
            .into_iter()
            .map(|path| {
                path.leading_colon
                    .iter()
                    .map(|_| String::new())
                    .chain(path.segments.iter().map(|segment| segment.ident.to_string()))
                    .collect()
            })
            .collect(),
    }
}

fn convert_attribute(attr: &syn::Attribute) -> Attribute {
    Attribute::Source {
        meta: attr.meta.to_token_stream().to_string(),
        inner: matches!(attr.style, syn::AttrStyle::Inner(_)),
    }
}

/// Recover documentation attributes for a RustLight declaration.
pub fn extract_docs(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter_map(|attr| {
            if !attr.path().is_ident("doc") {
                return None;
            }
            match &attr.meta {
                Meta::NameValue(name_value) => {
                    if let syn::Expr::Lit(ExprLit {
                        lit: Lit::Str(_), ..
                    }) = &name_value.value
                    {
                        // Keep doc text inside its literal instead of decoding it as Rust source.
                        Some(attr.to_token_stream().to_string())
                    } else {
                        None
                    }
                }
                _ => None,
            }
        })
        .collect()
}

fn derive_paths(
    attr: &syn::Attribute,
) -> Option<syn::punctuated::Punctuated<syn::Path, syn::Token![,]>> {
    let Meta::List(list) = &attr.meta else {
        return None;
    };
    if !list.path.is_ident("derive") || !matches!(list.delimiter, syn::MacroDelimiter::Paren(_)) {
        return None;
    }
    let paths = list.parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated).ok()?;
    paths.iter().all(|path| {
        path.segments.iter().all(|segment| matches!(segment.arguments, PathArguments::None))
    }).then_some(paths)
}

fn normalize_tokens(tokens: TokenStream) -> String {
    render_tokens(tokens, false)
}

fn normalize_path_tokens(tokens: TokenStream) -> String {
    render_tokens(tokens, true)
}

fn render_tokens(tokens: TokenStream, path: bool) -> String {
    fn append_text(parts: &mut Vec<(String, bool)>, text: &str) {
        if let Some((previous, false)) = parts.last_mut() {
            previous.push_str(text);
        } else {
            parts.push((text.to_string(), false));
        }
    }

    fn append_stream(tokens: TokenStream, parts: &mut Vec<(String, bool)>) {
        let mut first = true;
        let mut joint = false;
        for token in tokens {
            if !first && !joint {
                append_text(parts, " ");
            }
            first = false;
            // Joint punctuation must remain joined, including :: and lifetimes.
            joint = matches!(&token, TokenTree::Punct(punct) if punct.spacing() == Spacing::Joint);
            match token {
                TokenTree::Literal(literal) => parts.push((literal.to_string(), true)),
                TokenTree::Group(group) => {
                    let (open, close) = match group.delimiter() {
                        Delimiter::Parenthesis => ("(", ")"),
                        Delimiter::Brace => ("{ ", " }"),
                        Delimiter::Bracket => ("[", "]"),
                        Delimiter::None => ("", ""),
                    };
                    append_text(parts, open);
                    append_stream(group.stream(), parts);
                    append_text(parts, close);
                }
                token => append_text(parts, &token.to_string()),
            }
        }
    }

    let mut parts = Vec::new();
    append_stream(tokens, &mut parts);
    parts
        .into_iter()
        .map(|(text, literal)| {
            // Literal spelling is an indivisible leaf, never normalized as source text.
            if literal {
                return text;
            }
            let compact = text
                .replace(" :: ", "::")
                .replace(" (", "(")
                .replace("( ", "(")
                .replace(" )", ")")
                .replace(" ,", ",")
                .replace(" !", "!");
            if path {
                compact.replace("< ", "<").replace(" >", ">")
            } else {
                compact
            }
        })
        .collect()
}

trait ToTokenStreamExt {
    fn to_token_stream(&self) -> TokenStream;
}

impl<T> ToTokenStreamExt for T
where
    T: quote::ToTokens,
{
    fn to_token_stream(&self) -> TokenStream {
        quote::ToTokens::to_token_stream(self)
    }
}
