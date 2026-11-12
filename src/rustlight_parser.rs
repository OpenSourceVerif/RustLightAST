//! Rust source parsing for the RustLight subset.
//!
//! Unsupported syntax returns a `syn::Error`.

/// Parse and conversion errors returned by the public entry points.
pub use syn::Error as ParseError;

mod pattern;

pub use pattern::parse_rust_pattern;

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use syn::{
    parse_file, AngleBracketedGenericArguments, ExprArray, ExprAssign, ExprBinary, ExprBlock,
    ExprCall, ExprCast, ExprClosure, ExprField, ExprGroup, ExprIf, ExprIndex, ExprLit, ExprMatch,
    ExprMethodCall, ExprParen, ExprPath, ExprReference, ExprTuple, ExprUnary, File,
    GenericArgument, ImplItem as SynImplItem, Item as SynItem, Lit, LocalInit, Meta,
    ParenthesizedGenericArguments, Pat, PatIdent, PathArguments, ReturnType, Stmt, TraitBound,
    Type as SynType, TypeArray, TypeGroup, TypeImplTrait, TypeParamBound, TypeParen, TypeReference,
    TypeSlice, TypeTraitObject, TypeTuple, Visibility as SynVisibility,
};

#[cfg(test)]
use crate::RustCodeGenerator;
use crate::{
    AssociatedTypeBinding, Attribute, AttributeArg, Block, CallableBound, CallableTraitQualifier,
    CallableTraitType, ClosureParam, ConstDef, EnumDef, Expr, Field, FunctionDef, GenericParam,
    Generics, ImplBlock, ImplItem, Item, LetStmt,
    LifetimeParameter, Literal, MatchArm, Member, ModuleDecl, Param, PathType, RecordField,
    RustModule,
    Statement, StructDef, Type, TypeAlias, TypeParameter, UnionDef, UseKind, UseStatement, Variant,
    Visibility, WherePredicate,
};

/// Marker attached to recovered function signatures with synthetic empty bodies.
/// Kept verbatim for compatibility with existing Copy and Borrow analyses.
pub const TYPE_FACT_ONLY_DOC: &str = "@isabelle2rust-type-fact-only";

/// Parse a Rust file into the supported RustLight subset.
///
/// `module_name` is caller-provided metadata. No files are read or written.
/// Syntax outside the converter's supported subset returns an error; this is
/// not a lossless parser for arbitrary Rust source.
pub fn parse_rust_source(source: &str, module_name: impl Into<String>) -> syn::Result<RustModule> {
    let file = parse_file(source)?;
    convert_file(&file, module_name.into())
}

/// Recover package-wide type facts even when an unsupported function body
/// prevents conversion of the complete source file.  These fact-only modules
/// are never printed; they let Copy and Borrow observe datatype declarations,
/// imports, aliases, function signatures, and explicit `impl Copy`
/// declarations in adapter modules.
pub fn parse_rust_type_facts(
    source: &str,
    module_name: impl Into<String>,
) -> syn::Result<RustModule> {
    let file = parse_file(source)?;
    Ok(RustModule {
        name: module_name.into(),
        docs: vec![],
        items: file
            .items
            .iter()
            .filter_map(convert_type_fact_item)
            .collect(),
        attrs: vec![],
        vis: crate::Visibility::Public,
    })
}

fn convert_type_fact_item(item: &SynItem) -> Option<Item> {
    match item {
        SynItem::Struct(_) | SynItem::Enum(_) | SynItem::Type(_) | SynItem::Use(_) => {
            convert_item(item).ok()
        }
        SynItem::Fn(item_fn) => convert_function_type_fact(item_fn).map(Item::Function),
        SynItem::Impl(item_impl)
            if item_impl
                .trait_
                .as_ref()
                .and_then(|(_, path, _)| path.segments.last())
                .is_some_and(|segment| segment.ident == "Copy") =>
        {
            convert_item(item).ok()
        }
        SynItem::Mod(item_mod) => item_mod.content.as_ref().map(|(_, nested)| {
            Item::Mod(Box::new(RustModule {
                name: item_mod.ident.to_string(),
                docs: vec![],
                items: nested.iter().filter_map(convert_type_fact_item).collect(),
                attrs: vec![],
                vis: convert_visibility(&item_mod.vis),
            }))
        }),
        _ => None,
    }
}

/// Lower a declaration signature for analyses that do not inspect its body.
pub fn convert_function_type_fact(item_fn: &syn::ItemFn) -> Option<FunctionDef> {
    Some(FunctionDef {
        name: item_fn.sig.ident.to_string(),
        params: item_fn
            .sig
            .inputs
            .iter()
            .map(convert_fn_arg)
            .collect::<syn::Result<Vec<_>>>()
            .ok()?,
        return_type: convert_return_type(&item_fn.sig.output).ok()?,
        generics: convert_generics(&item_fn.sig.generics).ok()?,
        body: Block {
            stmts: Vec::new(),
            expr: None,
        },
        asyncness: item_fn.sig.asyncness.is_some(),
        vis: convert_visibility(&item_fn.vis),
        docs: vec![TYPE_FACT_ONLY_DOC.to_string()],
        attrs: Vec::new(),
    })
}

#[cfg(test)]
pub fn parse_and_print_rust_source(
    source: &str,
    module_name: impl Into<String>,
) -> syn::Result<(RustModule, String)> {
    let module = parse_rust_source(source, module_name)?;
    let mut generator = RustCodeGenerator::new();
    let printed = generator.generate_module_code(&module);
    Ok((module, printed))
}

/// Lower a parsed file at the Rust source boundary.
pub fn convert_file(file: &File, module_name: String) -> syn::Result<RustModule> {
    let items = file
        .items
        .iter()
        .map(convert_item)
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(file_module(file, module_name, items))
}

/// Convert supported declarations and preserve other declarations as opaque items.
///
/// Inline modules still expose their supported declarations to visitors.
pub fn convert_file_preserving_items(file: &File, module_name: String) -> RustModule {
    file_module(
        file,
        module_name,
        file.items.iter().map(convert_item_preserving),
    )
}

fn file_module(file: &File, module_name: String, items: impl IntoIterator<Item = Item>) -> RustModule {
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

fn convert_item_preserving(item: &SynItem) -> Item {
    if let SynItem::Mod(module) = item {
        if let Some((_, items)) = &module.content {
            let items = items.iter().map(convert_item_preserving).collect();
            return Item::Mod(Box::new(convert_inline_module(module, items)));
        }
    }
    // DEFENSE: files containing a static or a bare function pointer may
    // still contain ordinary functions that can be lowered and optimized.
    convert_item(item).unwrap_or_else(|_| Item::Raw(normalize_tokens(item.to_token_stream())))
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
    items.splice(..0, inner);

    RustModule {
        name: module.ident.to_string(),
        docs: extract_docs(&outer),
        items,
        attrs: convert_attributes(&outer),
        vis: convert_visibility(&module.vis),
    }
}

/// Lower a parsed item without reparsing its source text.
pub fn convert_item(item: &SynItem) -> syn::Result<Item> {
    match item {
        SynItem::Enum(item_enum) => Ok(Item::Enum(EnumDef {
            name: item_enum.ident.to_string(),
            variants: item_enum
                .variants
                .iter()
                .map(convert_variant)
                .collect::<syn::Result<Vec<_>>>()?,
            generics: convert_generics(&item_enum.generics)?,
            derives: extract_derives(&item_enum.attrs),
            docs: extract_docs(&item_enum.attrs),
            vis: convert_visibility(&item_enum.vis),
        })),
        SynItem::Fn(item_fn) => Ok(Item::Function(FunctionDef {
            name: item_fn.sig.ident.to_string(),
            params: item_fn
                .sig
                .inputs
                .iter()
                .map(convert_fn_arg)
                .collect::<syn::Result<Vec<_>>>()?,
            return_type: convert_return_type(&item_fn.sig.output)?,
            generics: convert_generics(&item_fn.sig.generics)?,
            body: convert_block(&item_fn.block)?,
            asyncness: item_fn.sig.asyncness.is_some(),
            vis: convert_visibility(&item_fn.vis),
            docs: extract_docs(&item_fn.attrs),
            attrs: convert_attributes(&item_fn.attrs),
        })),
        SynItem::Struct(item_struct) => Ok(Item::Struct(StructDef {
            name: item_struct.ident.to_string(),
            fields: item_struct
                .fields
                .iter()
                .map(convert_field)
                .collect::<syn::Result<Vec<_>>>()?,
            generics: convert_generics(&item_struct.generics)?,
            derives: extract_derives(&item_struct.attrs),
            docs: extract_docs(&item_struct.attrs),
            vis: convert_visibility(&item_struct.vis),
        })),
        SynItem::Union(item_union) => Ok(Item::Union(UnionDef {
            name: item_union.ident.to_string(),
            fields: item_union
                .fields
                .named
                .iter()
                .map(convert_field)
                .collect::<syn::Result<Vec<_>>>()?,
            generics: convert_generics(&item_union.generics)?,
            derives: extract_derives(&item_union.attrs),
            docs: extract_docs(&item_union.attrs),
            vis: convert_visibility(&item_union.vis),
        })),
        SynItem::Type(item_type) => Ok(Item::TypeAlias(TypeAlias {
            name: item_type.ident.to_string(),
            target: convert_type(&item_type.ty)?,
            generics: convert_generics(&item_type.generics)?,
            vis: convert_visibility(&item_type.vis),
            docs: extract_docs(&item_type.attrs),
        })),
        SynItem::Const(item_const) => Ok(Item::Const(ConstDef {
            name: item_const.ident.to_string(),
            ty: convert_type(&item_const.ty)?,
            value: convert_expr(&item_const.expr)?,
            vis: convert_visibility(&item_const.vis),
            docs: extract_docs(&item_const.attrs),
        })),
        SynItem::Use(item_use) => Ok(Item::Use(convert_use(item_use))),
        SynItem::Impl(item_impl) => {
            let mut block = convert_impl_header(item_impl)?;
            block.items = item_impl
                .items
                .iter()
                .map(convert_impl_item)
                .collect::<syn::Result<Vec<_>>>()?;
            Ok(Item::Impl(block))
        }
        SynItem::Mod(item_mod) if item_mod.content.is_none() => {
            Ok(Item::ModuleDecl(ModuleDecl {
                name: item_mod.ident.to_string(),
                vis: convert_visibility(&item_mod.vis),
                attrs: item_mod.attrs.iter().map(attribute_to_raw_source).collect(),
            }))
        }
        SynItem::Mod(item_mod) => {
            let nested_items = item_mod
                .content
                .as_ref()
                .map(|(_, items)| items.iter().map(convert_item).collect())
                .transpose()?
                .unwrap_or_default();

            Ok(Item::Mod(Box::new(convert_inline_module(
                item_mod,
                nested_items,
            ))))
        }
        SynItem::Trait(item_trait) => Ok(Item::Raw(normalize_tokens(item_trait.to_token_stream()))),
        SynItem::Macro(item_macro) => Ok(Item::Raw(normalize_tokens(item_macro.to_token_stream()))),
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported item kind in RustLightAST parser",
        )),
    }
}

fn convert_variant(variant: &syn::Variant) -> syn::Result<Variant> {
    let data = match &variant.fields {
        syn::Fields::Unit => None,
        syn::Fields::Unnamed(fields) => Some(
            fields
                .unnamed
                .iter()
                .map(|field| convert_type(&field.ty))
                .collect::<syn::Result<Vec<_>>>()?,
        ),
        syn::Fields::Named(fields) => Some(
            fields
                .named
                .iter()
                .map(|field| convert_type(&field.ty))
                .collect::<syn::Result<Vec<_>>>()?,
        ),
    };

    Ok(Variant {
        name: variant.ident.to_string(),
        data,
        docs: extract_docs(&variant.attrs),
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
        docs: extract_docs(&field.attrs),
        attrs: convert_attributes(&field.attrs),
    })
}

fn convert_fn_arg(arg: &syn::FnArg) -> syn::Result<Param> {
    match arg {
        syn::FnArg::Receiver(receiver) => Ok(Param {
            name: "self".to_string(),
            ty: convert_type(&receiver.ty)?,
            mutable: receiver.reference.is_none() && receiver.mutability.is_some(),
        }),
        syn::FnArg::Typed(parameter) => {
            let (name, mutable) = match parameter.pat.as_ref() {
                Pat::Ident(binding) if binding.by_ref.is_none() && binding.subpat.is_none() => {
                    (binding.ident.to_string(), binding.mutability.is_some())
                }
                Pat::Wild(_) => ("_".to_string(), false),
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unsupported parameter pattern in RustLightAST parser",
                    ));
                }
            };
            Ok(Param {
                name,
                ty: convert_type(&parameter.ty)?,
                mutable,
            })
        }
    }
}

/// Convert the type and generic parameters of an impl without its items.
pub fn convert_impl_header(item: &syn::ItemImpl) -> syn::Result<ImplBlock> {
    Ok(ImplBlock {
        target: convert_type(&item.self_ty)?,
        generics: convert_generics(&item.generics)?,
        items: Vec::new(),
        trait_impl: item
            .trait_
            .as_ref()
            .map(|(_, path, _)| convert_type_path(path))
            .transpose()?,
    })
}

fn convert_impl_item(item: &SynImplItem) -> syn::Result<ImplItem> {
    match item {
        SynImplItem::Fn(method) => Ok(ImplItem::Method(FunctionDef {
            name: method.sig.ident.to_string(),
            params: method
                .sig
                .inputs
                .iter()
                .map(convert_fn_arg)
                .collect::<syn::Result<Vec<_>>>()?,
            return_type: convert_return_type(&method.sig.output)?,
            generics: convert_generics(&method.sig.generics)?,
            body: convert_block(&method.block)?,
            asyncness: method.sig.asyncness.is_some(),
            vis: convert_visibility(&method.vis),
            docs: extract_docs(&method.attrs),
            attrs: convert_attributes(&method.attrs),
        })),
        SynImplItem::Const(item_const) => Ok(ImplItem::AssocConst(
            item_const.ident.to_string(),
            convert_type(&item_const.ty)?,
            convert_expr(&item_const.expr)?,
        )),
        SynImplItem::Type(item_type) => Ok(ImplItem::AssocType(
            item_type.ident.to_string(),
            convert_type(&item_type.ty)?,
        )),
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported impl item kind in RustLightAST parser",
        )),
    }
}

fn convert_use(item: &syn::ItemUse) -> UseStatement {
    let mut path = Vec::new();
    let kind = convert_use_tree(&item.tree, &mut path);
    UseStatement {
        path,
        kind,
        absolute: item.leading_colon.is_some(),
        vis: convert_visibility(&item.vis),
        attrs: item.attrs.iter().map(attribute_to_raw_source).collect(),
    }
}

fn convert_use_tree(tree: &syn::UseTree, segments: &mut Vec<String>) -> UseKind {
    match tree {
        syn::UseTree::Path(path) => {
            segments.push(path.ident.to_string());
            convert_use_tree(&path.tree, segments)
        }
        syn::UseTree::Name(name) => {
            segments.push(name.ident.to_string());
            UseKind::Simple
        }
        syn::UseTree::Glob(_) => UseKind::Glob,
        syn::UseTree::Group(group) => UseKind::Nested(
            group
                .items
                .iter()
                .map(|tree| normalize_tokens(tree.to_token_stream()))
                .collect(),
        ),
        syn::UseTree::Rename(rename) => {
            segments.push(format!("{} as {}", rename.ident, rename.rename));
            UseKind::Simple
        }
    }
}

fn convert_return_type(output: &ReturnType) -> syn::Result<Type> {
    match output {
        ReturnType::Default => Ok(Type::Unit),
        ReturnType::Type(_, ty) => convert_type(ty),
    }
}

/// Convert a parsed Rust type into the same model used in declarations.
pub fn convert_type(ty: &SynType) -> syn::Result<Type> {
    match ty {
        SynType::Path(type_path) => {
            // DEFENSE: dropping qself changes which associated type is named.
            if type_path.qself.is_some() {
                return Err(syn::Error::new_spanned(
                    type_path,
                    "qualified associated type is unsupported",
                ));
            }
            convert_type_path(&type_path.path)
        }
        SynType::Reference(TypeReference {
            elem, lifetime, mutability, ..
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
            parse_array_len(len)?,
        )),
        SynType::TraitObject(TypeTraitObject { bounds, .. }) => {
            convert_callable_trait_bounds(bounds.iter(), CallableTraitQualifier::Dyn)
        }
        SynType::ImplTrait(TypeImplTrait { bounds, .. }) => {
            if let (Some(TypeParamBound::Trait(bound)), 1) = (bounds.first(), bounds.len()) {
                if let Some(opaque) = convert_associated_impl_trait(bound)? {
                    return Ok(opaque);
                }
            }
            convert_callable_trait_bounds(bounds.iter(), CallableTraitQualifier::Impl)
        }
        SynType::Never(_) => Ok(Type::Never),
        SynType::Infer(_) => Ok(Type::Infer),
        SynType::Paren(TypeParen { elem, .. }) | SynType::Group(TypeGroup { elem, .. }) => {
            convert_type(elem)
        }
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported type syntax in RustLightAST parser",
        )),
    }
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
    let mut associated_types = Vec::new();
    for argument in &arguments.args {
        let GenericArgument::AssocType(assoc) = argument else {
            return Ok(None);
        };
        if assoc.generics.is_some() {
            return Ok(None);
        }
        associated_types.push(AssociatedTypeBinding {
            name: assoc.ident.to_string(),
            ty: convert_type(&assoc.ty)?,
        });
    }
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
                        return Err(syn::Error::new_spanned(bound, "multiple callable trait bounds"));
                    }
                    continue;
                }
                if !matches!(qualifier, CallableTraitQualifier::Impl)
                    || clone
                    || trait_bound.lifetimes.is_some()
                    || !last.arguments.is_none()
                    || !is_native_callable_bound_path(path, "clone", "Clone")
                {
                    return Err(syn::Error::new_spanned(bound, "unsupported callable trait bound"));
                }
                clone = true;
                CallableBound::Trait(bound_path_segments(path, path.segments.len()))
            }
            _ => return Err(syn::Error::new_spanned(bound, "unsupported callable trait bound")),
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
    let higher_ranked = trait_bound
        .lifetimes
        .iter()
        .flat_map(|binder| &binder.lifetimes)
        .filter_map(|param| match param {
            syn::GenericParam::Lifetime(param) => Some(convert_lifetime_param(param)),
            _ => None,
        })
        .collect();
    Ok(Type::CallableTrait(CallableTraitType {
        qualifier,
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

fn convert_type_path(path: &syn::Path) -> syn::Result<Type> {
    let last = path
        .segments
        .last()
        .ok_or_else(|| syn::Error::new_spanned(path, "empty path"))?;
    // DEFENSE: `Outer<T>::Item` would lose the arguments that select `Item`.
    if let Some(segment) = path
        .segments
        .iter()
        .rev()
        .skip(1)
        .find(|segment| !segment.arguments.is_none())
    {
        return Err(syn::Error::new_spanned(
            segment,
            "generic arguments on a non-final type path segment are unsupported",
        ));
    }
    let generic_args = match &last.arguments {
        PathArguments::None => Vec::new(),
        PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }) => args
            .iter()
            .map(|arg| match arg {
                GenericArgument::Type(ty) => convert_type(ty),
                // DEFENSE: dropping `3` from `Model<3>` may select a default const.
                _ => Err(syn::Error::new_spanned(
                    arg,
                    "non-type generic argument is unsupported",
                )),
            })
            .collect::<syn::Result<Vec<_>>>()?,
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
        .chain(path.segments.iter().map(|segment| segment.ident.to_string()))
        .collect();
    if !generic_args.is_empty() {
        Ok(Type::Generic(segments.join("::"), generic_args))
    } else if segments.len() > 1 {
        Ok(Type::Path(segments))
    } else {
        Ok(Type::Named(last.ident.to_string()))
    }
}

fn parse_array_len(expr: &syn::Expr) -> syn::Result<usize> {
    match expr {
        syn::Expr::Lit(ExprLit {
            lit: Lit::Int(int_lit),
            ..
        }) => int_lit.base10_parse(),
        _ => Err(syn::Error::new_spanned(
            expr,
            "array length must be an integer literal",
        )),
    }
}

/// Convert a parsed body into RustLight expressions and statements.
pub fn convert_block(block: &syn::Block) -> syn::Result<Block> {
    let mut stmts = Vec::new();
    let mut tail_expr = None;

    for (index, stmt) in block.stmts.iter().enumerate() {
        match stmt {
            Stmt::Local(local) => stmts.push(convert_local(local)?),
            Stmt::Item(item) => stmts.push(Statement::Item(Box::new(convert_local_item(item)))),
            Stmt::Expr(expr, semi) => {
                if semi.is_some() || index + 1 != block.stmts.len() {
                    stmts.push(Statement::Expr(convert_expr(expr)?));
                } else {
                    tail_expr = Some(Box::new(convert_expr(expr)?));
                }
            }
            Stmt::Macro(statement) => stmts.push(Statement::Macro(crate::MacroStmt {
                attrs: statement
                    .attrs
                    .iter()
                    .map(|attr| attr.to_token_stream().to_string())
                    .collect(),
                path: normalize_path_tokens(statement.mac.path.to_token_stream()),
                delimiter: match statement.mac.delimiter {
                    syn::MacroDelimiter::Paren(_) => crate::MacroDelimiter::Parenthesis,
                    syn::MacroDelimiter::Brace(_) => crate::MacroDelimiter::Brace,
                    syn::MacroDelimiter::Bracket(_) => crate::MacroDelimiter::Bracket,
                },
                tokens: statement.mac.tokens.to_string(),
                semicolon: statement.semi_token.is_some(),
            })),
        }
    }

    Ok(Block {
        stmts,
        expr: tail_expr,
    })
}

fn convert_local_item(item: &SynItem) -> Item {
    if let SynItem::Fn(function) = item {
        if can_model_local_function(function) {
            if let Ok(function) = convert_item(item) {
                return function;
            }
        }
    }
    Item::Raw(normalize_tokens(item.to_token_stream()))
}

fn can_model_local_function(function: &syn::ItemFn) -> bool {
    use syn::visit::{self, Visit};

    // DEFENSE: unlike module declarations, local functions have no metadata
    // restoration after printing. Keep their original syntax when a supported
    // conversion would still erase qualifiers, attributes, or arguments.
    struct Complete(bool);
    impl<'ast> Visit<'ast> for Complete {
        fn visit_item(&mut self, _: &'ast SynItem) {
            // Nested items make their own conversion decision.
        }

        fn visit_attribute(&mut self, _: &'ast syn::Attribute) {
            self.0 = false;
        }

        fn visit_generic_param(&mut self, parameter: &'ast syn::GenericParam) {
            self.0 &= match parameter {
                syn::GenericParam::Type(parameter) => parameter.default.is_none(),
                syn::GenericParam::Lifetime(_) => true,
                syn::GenericParam::Const(_) => false,
            };
            visit::visit_generic_param(self, parameter);
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            self.0 &= call.turbofish.is_none();
            visit::visit_expr_method_call(self, call);
        }

        fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
            self.0 &= closure.asyncness.is_none()
                && closure.constness.is_none()
                && closure.movability.is_none()
                && closure.lifetimes.is_none();
            visit::visit_expr_closure(self, closure);
        }

        fn visit_local(&mut self, local: &'ast syn::Local) {
            self.0 &= !matches!(&local.pat, Pat::Ident(pattern) if pattern.by_ref.is_some());
            self.visit_pat(&local.pat);
            if let Some(init) = &local.init {
                self.visit_local_init(init);
            }
        }
    }

    let signature = &function.sig;
    let mut complete = Complete(
        signature.constness.is_none()
            && signature.unsafety.is_none()
            && signature.abi.is_none()
            && signature.variadic.is_none(),
    );
    complete.visit_item_fn(function);
    complete.0
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

    let (name, ifmut, ty) = match &local.pat {
        Pat::Type(typed) => (
            typed.pat.to_token_stream().to_string(),
            false,
            Some(convert_type(&typed.ty)?),
        ),
        Pat::Ident(PatIdent {
            ident,
            by_ref: None,
            mutability,
            subpat: None,
            ..
        }) => (ident.to_string(), mutability.is_some(), None),
        pat => (pat.to_token_stream().to_string(), false, None),
    };

    let init = match &local.init {
        Some(LocalInit { expr, .. }) => Some(convert_expr(expr)?),
        None => None,
    };

    Ok(Statement::Let(LetStmt {
        attrs,
        ifmut,
        name,
        ty,
        init,
    }))
}

/// Convert a parsed Rust expression without reparsing its source.
pub fn convert_expr(expr: &syn::Expr) -> syn::Result<Expr> {
    match expr {
        syn::Expr::Path(expr_path) => convert_expr_path(expr_path),
        syn::Expr::Call(ExprCall { func, args, .. }) => Ok(Expr::Call(
            Box::new(convert_expr(func)?),
            args.iter()
                .map(convert_expr)
                .collect::<syn::Result<Vec<_>>>()?,
        )),
        syn::Expr::MethodCall(ExprMethodCall {
            receiver,
            method,
            args,
            ..
        }) => Ok(Expr::MethodCall(
            Box::new(convert_expr(receiver)?),
            method.to_string(),
            args.iter()
                .map(convert_expr)
                .collect::<syn::Result<Vec<_>>>()?,
        )),
        syn::Expr::Match(ExprMatch { expr, arms, .. }) => Ok(Expr::Match {
            expr: Box::new(convert_expr(expr)?),
            arms: arms
                .iter()
                .map(convert_match_arm)
                .collect::<syn::Result<Vec<_>>>()?,
        }),
        syn::Expr::Block(ExprBlock { label, block, .. }) => {
            let body = convert_block(block)?;
            Ok(match label {
                Some(label) => Expr::LabeledBlock {
                    label: label.name.ident.to_string(),
                    body,
                },
                None => Expr::Block(body),
            })
        }
        syn::Expr::Loop(expr) => Ok(Expr::Loop(
            Box::new(convert_block(&expr.body)?),
            expr.label.as_ref().map(|label| label.name.ident.to_string()),
        )),
        syn::Expr::While(expr) => {
            let condition = match expr.cond.as_ref() {
                syn::Expr::Let(condition) => Expr::Let {
                    pattern: pattern::convert_pattern(&condition.pat)?,
                    value: Box::new(convert_expr(&condition.expr)?),
                },
                condition => convert_expr(condition)?,
            };
            Ok(Expr::While {
                label: expr.label.as_ref().map(|label| label.name.ident.to_string()),
                condition: Box::new(condition),
                body: convert_block(&expr.body)?,
            })
        }
        syn::Expr::ForLoop(expr) => Ok(Expr::For {
            label: expr.label.as_ref().map(|label| label.name.ident.to_string()),
            pattern: pattern::convert_pattern(&expr.pat)?,
            iter: Box::new(convert_expr(&expr.expr)?),
            body: convert_block(&expr.body)?,
        }),
        syn::Expr::Return(expr) => Ok(Expr::Return(
            expr.expr.as_deref().map(convert_expr).transpose()?.map(Box::new),
        )),
        syn::Expr::Break(expr) => Ok(Expr::Break {
            label: expr.label.as_ref().map(|label| label.ident.to_string()),
            value: expr.expr.as_deref().map(convert_expr).transpose()?.map(Box::new),
        }),
        syn::Expr::Continue(expr) => Ok(Expr::Continue(
            expr.label.as_ref().map(|label| label.ident.to_string()),
        )),
        syn::Expr::Reference(ExprReference {
            expr, mutability, ..
        }) => Ok(Expr::Reference(
            Box::new(convert_expr(expr)?),
            true,
            mutability.is_some(),
        )),
        syn::Expr::Cast(ExprCast { expr, ty, .. }) => {
            Ok(Expr::Cast(Box::new(convert_expr(expr)?), convert_type(ty)?))
        }
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
        syn::Expr::Try(expr) => Ok(Expr::Try(Box::new(convert_expr(&expr.expr)?))),
        syn::Expr::Repeat(expr) => Ok(Expr::Repeat {
            value: Box::new(convert_expr(&expr.expr)?),
            len: Box::new(convert_expr(&expr.len)?),
        }),
        syn::Expr::Range(expr) => {
            let bound = |bound: Option<&syn::Expr>| {
                bound.map(|bound| convert_expr(bound).map(Box::new)).transpose()
            };
            let start = bound(expr.start.as_deref())?;
            let end = bound(expr.end.as_deref())?;
            let end = match expr.limits {
                syn::RangeLimits::HalfOpen(_) => crate::RangeEnd::Exclusive(end),
                syn::RangeLimits::Closed(_) => crate::RangeEnd::Inclusive(end.ok_or_else(|| {
                    syn::Error::new_spanned(expr, "inclusive range requires an end expression")
                })?),
            };
            Ok(Expr::Range { start, end })
        }
        syn::Expr::Paren(ExprParen { expr, .. }) | syn::Expr::Group(ExprGroup { expr, .. }) => {
            Ok(Expr::Parenthesized(Box::new(convert_expr(expr)?)))
        }
        syn::Expr::Binary(ExprBinary {
            left, op, right, ..
        }) => Ok(Expr::BinaryOp(
            Box::new(convert_expr(left)?),
            op.to_token_stream().to_string(),
            Box::new(convert_expr(right)?),
        )),
        syn::Expr::Unary(ExprUnary { op, expr, .. }) => Ok(Expr::UnaryOp(
            op.to_token_stream().to_string(),
            Box::new(convert_expr(expr)?),
        )),
        syn::Expr::Assign(ExprAssign { left, right, .. }) => Ok(Expr::Assign(
            Box::new(convert_expr(left)?),
            Box::new(convert_expr(right)?),
        )),
        syn::Expr::If(ExprIf {
            cond,
            then_branch,
            else_branch,
            ..
        }) => {
            let then_branch = convert_block(then_branch)?;
            let else_branch = else_branch
                .as_ref()
                .map(|(_, expr)| expr_to_block(expr))
                .transpose()?;
            match cond.as_ref() {
                syn::Expr::Let(condition) => Ok(Expr::IfLet {
                    pattern: normalize_tokens(condition.pat.to_token_stream()),
                    value: Box::new(convert_expr(&condition.expr)?),
                    then_branch,
                    else_branch,
                }),
                condition => Ok(Expr::If {
                    condition: Box::new(convert_expr(condition)?),
                    then_branch,
                    else_branch,
                }),
            }
        }
        syn::Expr::Lit(ExprLit { lit, .. }) => Ok(Expr::Literal(convert_literal(lit)?)),
        syn::Expr::Tuple(ExprTuple { elems, .. }) => Ok(Expr::Tuple(
            elems
                .iter()
                .map(convert_expr)
                .collect::<syn::Result<Vec<_>>>()?,
        )),
        syn::Expr::Index(ExprIndex { expr, index, .. }) => Ok(Expr::Index(
            Box::new(convert_expr(expr)?),
            Box::new(convert_expr(index)?),
        )),
        syn::Expr::Field(ExprField { base, member, .. }) => Ok(Expr::Field {
            base: Box::new(convert_expr(base)?),
            member: convert_member(member),
        }),
        syn::Expr::Struct(record) => {
            if record.qself.is_some() {
                return Err(syn::Error::new_spanned(
                    record,
                    "unsupported qualified record constructor",
                ));
            }
            Ok(Expr::Record {
                path: path_segments(&record.path),
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
        syn::Expr::Array(ExprArray { elems, .. }) => Ok(Expr::Array(
            elems
                .iter()
                .map(convert_expr)
                .collect::<syn::Result<Vec<_>>>()?,
        )),
        syn::Expr::Macro(expr_macro) => {
            // Preserve generated fallback arms such as
            // `_ => panic!("non-exhaustive match")` as opaque Rust source.
            let source = normalize_tokens(expr_macro.to_token_stream());
            Ok(Expr::Macro(source))
        }
        syn::Expr::Closure(ExprClosure {
            capture,
            inputs,
            output,
            body,
            ..
        }) => {
            let is_move = capture.is_some();
            let params = inputs
                .iter()
                .map(convert_closure_param)
                .collect::<syn::Result<Vec<_>>>()?;
            let body = Box::new(convert_expr(body)?);
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
    match pattern {
        syn::Pat::Type(typed) => Ok(ClosureParam::typed(
            normalize_tokens(typed.pat.to_token_stream()),
            convert_type(&typed.ty)?,
        )),
        other => Ok(ClosureParam::untyped(normalize_tokens(
            other.to_token_stream(),
        ))),
    }
}

fn expr_to_block(expr: &syn::Expr) -> syn::Result<Block> {
    match expr {
        syn::Expr::Block(ExprBlock { label: None, block, .. }) => convert_block(block),
        syn::Expr::If(expr_if) => Ok(Block {
            stmts: Vec::new(),
            expr: Some(Box::new(convert_expr(&syn::Expr::If(expr_if.clone()))?)),
        }),
        _ => Ok(Block {
            stmts: Vec::new(),
            expr: Some(Box::new(convert_expr(expr)?)),
        }),
    }
}

fn convert_match_arm(arm: &syn::Arm) -> syn::Result<MatchArm> {
    Ok(MatchArm {
        pattern: pattern::convert_pattern(&arm.pat)?,
        guard: arm
            .guard
            .as_ref()
            .map(|(_, expr)| convert_expr(expr))
            .transpose()?,
        body: expr_to_block(&arm.body)?,
    })
}

fn convert_expr_path(expr_path: &ExprPath) -> syn::Result<Expr> {
    if expr_path.qself.is_some() {
        return Ok(Expr::Macro(normalize_path_tokens(
            expr_path.to_token_stream(),
        )));
    }

    let segments = path_segments(&expr_path.path);

    match segments.as_slice() {
        [segment] => Ok(Expr::Ident(segment.clone())),
        _ => Ok(Expr::Path(segments, PathType::Namespace)),
    }
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

/// Recover the first supported type argument from a textual path segment,
/// such as `identity::<u32>`. Returns `None` for invalid or unsupported input.
pub fn first_explicit_type_argument(segment: &str) -> Option<Type> {
    let path = syn::parse_str::<syn::Path>(segment).ok()?;
    let last = path.segments.last()?;
    let PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| match argument {
        GenericArgument::Type(ty) => convert_type(ty).ok(),
        _ => None,
    })
}

fn convert_literal(lit: &Lit) -> syn::Result<Literal> {
    match lit {
        Lit::Bool(bool_lit) => Ok(Literal::Bool(bool_lit.value)),
        other => Ok(Literal::Raw(other.to_token_stream().to_string())),
    }
}

fn convert_generics(generics: &syn::Generics) -> syn::Result<Generics> {
    let mut params = generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(type_param) => Some(GenericParam::Type(TypeParameter {
                name: type_param.ident.to_string(),
                bounds: type_param
                    .bounds
                    .iter()
                    .map(|bound| normalize_tokens(bound.to_token_stream()))
                    .collect(),
            })),
            syn::GenericParam::Lifetime(param) => {
                Some(GenericParam::Lifetime(convert_lifetime_param(param)))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut where_predicates = Vec::new();

    if let Some(clause) = &generics.where_clause {
        for predicate in &clause.predicates {
            match predicate {
                syn::WherePredicate::Type(predicate) if predicate.lifetimes.is_none() => {
                    let bounds = predicate
                        .bounds
                        .iter()
                        .map(|bound| normalize_tokens(bound.to_token_stream()))
                        .collect();
                    let local_parameter = match &predicate.bounded_ty {
                        SynType::Path(path) if path.qself.is_none() => path.path.get_ident(),
                        _ => None,
                    }
                    .and_then(|name| {
                        params
                            .iter_mut()
                            .filter_map(GenericParam::type_parameter_mut)
                            .find(|param| *name == param.name)
                    });
                    if let Some(parameter) = local_parameter {
                        extend_bounds(&mut parameter.bounds, bounds);
                    } else {
                        where_predicates.push(WherePredicate::Type {
                            subject: convert_type(&predicate.bounded_ty)?,
                            bounds,
                        });
                    }
                }
                syn::WherePredicate::Lifetime(predicate) => {
                    let subject = predicate.lifetime.to_string();
                    let bounds = predicate.bounds.iter().map(ToString::to_string).collect();
                    let parameter = params.iter_mut().find_map(|param| match param {
                        GenericParam::Lifetime(param) if param.name == subject => Some(param),
                        _ => None,
                    });
                    if let Some(parameter) = parameter {
                        extend_bounds(&mut parameter.bounds, bounds);
                    } else {
                        where_predicates.push(WherePredicate::Lifetime { subject, bounds });
                    }
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

fn extend_bounds(bounds: &mut Vec<String>, additional: Vec<String>) {
    for bound in additional {
        if !bounds.contains(&bound) {
            bounds.push(bound);
        }
    }
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

    match &attr.meta {
        Meta::Path(path) => format!("{marker}[{}]", normalize_tokens(path.to_token_stream())),
        Meta::NameValue(name_value) => format!(
            "{marker}[{} = {}]",
            normalize_tokens(name_value.path.to_token_stream()),
            normalize_tokens(name_value.value.to_token_stream())
        ),
        Meta::List(list) => format!(
            "{marker}[{}({})]",
            normalize_tokens(list.path.to_token_stream()),
            normalize_tokens(list.tokens.clone())
        ),
    }
}

/// Convert attributes represented by RustLightAST items.
pub fn convert_attributes(attrs: &[syn::Attribute]) -> Vec<Attribute> {
    attrs.iter().filter_map(convert_attribute).collect()
}

fn convert_attribute(attr: &syn::Attribute) -> Option<Attribute> {
    if attr.path().is_ident("doc") || attr.path().is_ident("derive") {
        return None;
    }

    let args = match &attr.meta {
        Meta::Path(_) => Vec::new(),
        Meta::NameValue(name_value) => vec![AttributeArg::KeyValue(
            name_value.path.to_token_stream().to_string(),
            convert_literal_from_expr(&name_value.value)?,
        )],
        Meta::List(list) => normalize_tokens(list.tokens.clone())
            .split(',')
            .filter(|part| !part.trim().is_empty())
            .map(|part| AttributeArg::Ident(part.trim().to_string()))
            .collect(),
    };

    Some(Attribute {
        name: normalize_tokens(attr.path().to_token_stream()),
        args,
    })
}

fn convert_literal_from_expr(expr: &syn::Expr) -> Option<Literal> {
    if let syn::Expr::Lit(ExprLit { lit, .. }) = expr {
        convert_literal(lit).ok()
    } else {
        None
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

fn extract_derives(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("derive"))
        .flat_map(|attr| match &attr.meta {
            Meta::List(list) => normalize_tokens(list.tokens.clone())
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::{parse_and_print_rust_source, parse_rust_source, parse_rust_type_facts};
    use crate::{Expr, Item, Pattern, RustCodeGenerator, Statement, Type};

    #[test]
    fn parses_rec_get_sample_and_prints_it() {
        let source = r#"
use crate::Int::*;

#[derive(Clone)]
pub enum Num {
    One,
    Bit0(Box<Num>),
    Bit1(Box<Num>),
}

#[derive(Clone)]
pub enum Int {
    ZeroInt,
    Pos(Num),
    Neg(Num),
}

#[derive(Clone)]
pub enum Option {
    None,
    Some(Int),
    Rec(Box<Option>),
}

pub fn get(x0: Option) -> Int {
    match x0 {
        Option::Some(x) => x.clone(),
        Option::None => Int::ZeroInt,
        Option::Rec(p0) => {
            let op = *p0;
            get(op.clone())
        },
    }
}
"#;
        let module = parse_rust_source(source, "Rec_Get_Tests").expect("sample parses");

        assert_eq!(module.name, "Rec_Get_Tests");
        assert_eq!(module.items.len(), 5);

        match &module.items[1] {
            Item::Enum(def) => {
                assert_eq!(def.name, "Num");
                assert_eq!(def.variants.len(), 3);
                assert_eq!(def.derives, vec!["Clone"]);
                match &def.variants[1].data.as_ref().expect("tuple variant")[0] {
                    Type::Generic(name, params) => {
                        assert_eq!(name, "Box");
                        assert_eq!(params.len(), 1);
                    }
                    other => panic!("unexpected variant payload: {other:?}"),
                }
            }
            other => panic!("unexpected first item: {other:?}"),
        }

        match &module.items[4] {
            Item::Function(def) => {
                assert_eq!(def.name, "get");
                assert_eq!(def.params.len(), 1);
                match &def.body.expr {
                    Some(expr) => match expr.as_ref() {
                        Expr::Match { expr, arms } => {
                            assert!(matches!(expr.as_ref(), Expr::Ident(name) if name == "x0"));
                            assert_eq!(arms.len(), 3);
                            assert!(matches!(
                                &arms[2].pattern,
                                Pattern::TupleStruct { path, fields }
                                    if path == &["Option", "Rec"]
                                        && matches!(fields.as_slice(),
                                            [Pattern::Ident { name, .. }] if name == "p0")
                            ));
                            assert_eq!(arms[2].body.stmts.len(), 1);
                            match &arms[2].body.stmts[0] {
                                Statement::Let(let_stmt) => {
                                    assert_eq!(let_stmt.name, "op");
                                    assert!(matches!(
                                        let_stmt.init.as_ref(),
                                        Some(Expr::UnaryOp(op, inner))
                                            if op == "*"
                                                && matches!(inner.as_ref(), Expr::Ident(name) if name == "p0")
                                    ));
                                }
                                other => panic!("unexpected extraction statement: {other:?}"),
                            }
                            match arms[2].body.expr.as_ref().expect("arm expr").as_ref() {
                                Expr::Call(callee, args) => {
                                    assert!(matches!(
                                        callee.as_ref(),
                                        Expr::Ident(name) if name == "get"
                                    ));
                                    assert_eq!(args.len(), 1);
                                    assert!(matches!(
                                        &args[0],
                                        Expr::MethodCall(receiver, method, _)
                                        if matches!(receiver.as_ref(), Expr::Ident(name) if name == "op")
                                            && method == "clone"
                                    ));
                                }
                                other => panic!("unexpected recursive arm body: {other:?}"),
                            }
                        }
                        other => panic!("unexpected function body expression: {other:?}"),
                    },
                    None => panic!("expected tail expression"),
                }
            }
            other => panic!("unexpected fourth item: {other:?}"),
        }

        let mut generator = RustCodeGenerator::new();
        let printed = generator.generate_module_code(&module);
        assert!(printed.contains("enum Num"));
        assert!(printed.contains("pub fn get(x0: Option) -> Int"));
        assert!(printed.contains("Option::Rec(p0) => {"));
        assert!(printed.contains("let op = *p0;"));
        assert!(printed.contains("get(op.clone())"));
        assert!(!printed.is_empty());
    }

    #[test]
    fn preserves_expression_macros_as_opaque_rust() {
        let source = r#"
pub fn unwrap_or_panic(x0: Option<bool>) -> bool {
    match x0 {
        Some(x) => x,
        _ => panic!("non-exhaustive match"),
    }
}
"#;
        let module = parse_rust_source(source, "Macro_Test").expect("macro expression parses");

        let mut generator = RustCodeGenerator::new();
        let printed = generator.generate_module_code(&module);
        assert!(printed.contains(r#"panic!("non-exhaustive match")"#));
    }

    #[test]
    fn preserves_statement_macros_without_rejecting_the_module() {
        let source = r#"
pub fn value() -> u64 {
    macro_rules! one {
        () => { 1u64 };
    }
    one!()
}
"#;
        let module = parse_rust_source(source, "Macro_Stmt_Test").expect("module parses");
        let mut generator = RustCodeGenerator::new();
        let printed = generator.generate_module_code(&module);
        syn::parse_file(&printed).expect("statement macro roundtrips as valid Rust");
        assert!(printed.replace(' ', "").contains("macro_rules!one"));
    }

    #[test]
    fn recovers_explicit_copy_facts_from_an_unconvertible_module() {
        let source = r#"
pub struct Word<W>(pub u128, pub std::marker::PhantomData<W>);
impl<W> Copy for Word<W> {}
impl<W> Clone for Word<W> { fn clone(&self) -> Self { *self } }
pub fn unsupported() { let Some(_) = None::<u8> else { return; }; }
"#;
        assert!(parse_rust_source(source, "Facts_Test").is_err());
        let facts = parse_rust_type_facts(source, "Facts_Test").expect("facts parse");
        assert!(facts
            .items
            .iter()
            .any(|item| matches!(item, Item::Struct(def) if def.name == "Word")));
        assert!(facts.items.iter().any(|item| matches!(
            item,
            Item::Impl(block)
                if block.trait_impl.as_ref().is_some_and(type_is_copy_trait_for_test)
        )));
    }

    fn type_is_copy_trait_for_test(ty: &Type) -> bool {
        matches!(ty, Type::Named(name) if name == "Copy")
            || matches!(ty, Type::Path(path) if path.last().is_some_and(|name| name == "Copy"))
    }

    #[test]
    fn preserves_tuple_struct_syntax() {
        let source = r#"
#[derive(Clone)]
struct Signed<A>(PhantomData<A>);
"#;
        let (_, printed) =
            parse_and_print_rust_source(source, "Tuple_Struct_Test").expect("tuple struct parses");

        assert!(printed.contains("struct Signed <A>(PhantomData<A>);"));
        syn::parse_file(&printed).expect("printed tuple struct remains valid Rust");
    }

    #[test]
    fn preserves_array_expressions_without_allocating_vecs() {
        let source = r#"
pub fn bytes(x: u8) -> [u8; 3] {
    [1, x, 3]
}
"#;
        let (module, printed) =
            parse_and_print_rust_source(source, "Array_Test").expect("array expression parses");

        let Item::Function(function) = &module.items[0] else {
            panic!("expected bytes function");
        };
        assert!(matches!(
            function.body.expr.as_deref(),
            Some(Expr::Array(items)) if items.len() == 3
        ));
        assert!(printed.contains("[1, x, 3]"));
        assert!(!printed.contains("vec!"));
        syn::parse_file(&printed).expect("printed array remains valid Rust");
    }

    #[test]
    fn preserves_literal_spelling_and_escaping() {
        let source = r##"
pub fn shift(x: u64) -> u64 {
    x << 1usize
}

pub fn escaped() {
    let _s = "line\n\"quoted\"";
    let _c = '\n';
}
"##;
        let (_, printed) =
            parse_and_print_rust_source(source, "Literal_Test").expect("literals parse");

        assert!(printed.contains("1usize"));
        assert!(printed.contains(r#""line\n\"quoted\"""#));
        assert!(printed.contains(r"'\n'"));
        syn::parse_file(&printed).expect("printed literals remain valid Rust");
    }

    #[test]
    fn preserves_qualified_self_paths() {
        let source = r#"
pub fn one_bigint() -> BigInt {
    <BigInt as One>::one()
}
"#;
        let module = parse_rust_source(source, "QSelf_Test").expect("qself path parses");

        let mut generator = RustCodeGenerator::new();
        let printed = generator.generate_module_code(&module);
        assert!(printed.contains("<BigInt as One>::one()"));
    }

    #[test]
    fn preserves_trait_items_and_renamed_uses_as_raw_rust() {
        let source = r#"
use num_traits::sign::Signed as _;

pub trait Zero {
    fn zero() -> Self where Self: Sized;
}
"#;
        let module = parse_rust_source(source, "Trait_Test").expect("trait item parses");

        let mut generator = RustCodeGenerator::new();
        let printed = generator.generate_module_code(&module);
        assert!(printed.contains("use num_traits::sign::Signed as _;"));
        assert!(printed.contains("pub trait Zero"));
        assert!(printed.contains("fn zero"));
        assert!(printed.contains("Self : Sized"));
    }

    #[test]
    fn parses_callable_trait_types() {
        let source = r#"
use std::rc::Rc;

pub fn apply<A, B>(f: Rc<dyn Fn(A) -> B>, x: A) -> B {
    (*f)(x)
}

pub fn call_impl<A, B>(f: impl Fn(A) -> B, x: A) -> B {
    f(x)
}
"#;
        let module = parse_rust_source(source, "Callable_Test").expect("callable types parse");

        match &module.items[1] {
            Item::Function(def) => match &def.params[0].ty {
                Type::Generic(name, params) => {
                    assert_eq!(name, "Rc");
                    assert!(matches!(&params[0], Type::CallableTrait(callable)
                        if callable.trait_path == ["Fn"]
                            && callable.args.len() == 1
                            && matches!(callable.return_type.as_ref(), Type::Named(name) if name == "B")));
                }
                other => panic!("unexpected callable parameter type: {other:?}"),
            },
            other => panic!("unexpected item: {other:?}"),
        }

        let mut generator = RustCodeGenerator::new();
        let printed = generator.generate_module_code(&module);
        assert!(printed.contains("Rc<dyn Fn(A) -> B>"));
        assert!(printed.contains("impl Fn(A) -> B"));
    }

    #[test]
    fn parses_and_prints_captured_closure_cast() {
        let source = r"
use std::rc::Rc;

pub fn make_pair(y: Int) -> (Rc<dyn Fn(Int) -> Int>, Int) {
    ((({
        let y_cap = y.clone();
        Rc::new(move |x: Int| {
            plus_int(x.clone(), y_cap.clone())
        })
    }) as Rc<dyn Fn(Int) -> Int>), y.clone())
}
";
        let (module, printed) =
            parse_and_print_rust_source(source, "Closure_Cast_Test").expect("cast parses");

        let Item::Function(function) = &module.items[1] else {
            panic!("expected make_pair function");
        };
        let Expr::Tuple(items) = function.body.expr.as_deref().expect("function tail") else {
            panic!("expected tuple tail");
        };
        let Expr::Parenthesized(cast) = &items[0] else {
            panic!("expected parenthesized cast");
        };
        let Expr::Cast(_, Type::Generic(name, params)) = cast.as_ref() else {
            panic!("expected structured cast");
        };
        assert_eq!(name, "Rc");
        assert!(matches!(&params[0], Type::CallableTrait(callable)
            if callable.trait_path == ["Fn"]
                && callable.args.len() == 1
                && matches!(callable.return_type.as_ref(), Type::Named(name) if name == "Int")));

        assert!(printed.contains("as Rc<dyn Fn(Int) -> Int>"));
        syn::parse_file(&printed).expect("printed cast remains valid Rust");
    }

    #[test]
    fn preserves_explicit_closure_return_types() {
        let source = r#"
use std::rc::Rc;

pub fn partial() -> Rc<dyn Fn(Int) -> Pred<Unit>> {
    Rc::new(move |x: Int| -> Pred<Unit> {
        panic!("partial")
    })
}
"#;
        let (_, printed) = parse_and_print_rust_source(source, "Typed_Closure_Test")
            .expect("typed closure parses");

        assert!(printed.contains("move |x: Int| -> Pred<Unit>"));
        syn::parse_file(&printed).expect("printed typed closure remains valid Rust");
    }
}
