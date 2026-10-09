//! Rust source parsing for the RustLight subset.
//!
//! Unsupported syntax returns a `syn::Error`.

/// Parse and conversion errors returned by the public entry points.
pub use syn::Error as ParseError;

use proc_macro2::TokenStream;
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
    Attribute, AttributeArg, Block, CallableTraitQualifier, CallableTraitType, ClosureParam,
    ConstDef, EnumDef, Expr, Field, FunctionDef, GenericParam, ImplBlock, ImplItem, Item, LetStmt,
    Literal, MatchArm, Param, PathType, RustModule, Statement, StructDef, Type, TypeAlias,
    UnionDef, UseKind, UseStatement, Variant, Visibility,
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
    convert_file(file, module_name.into())
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

fn convert_function_type_fact(item_fn: &syn::ItemFn) -> Option<FunctionDef> {
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
        generics: convert_generics(&item_fn.sig.generics),
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

fn convert_file(file: File, module_name: String) -> syn::Result<RustModule> {
    let mut items = file
        .attrs
        .iter()
        .map(|attr| Item::Raw(attribute_to_raw_source(attr)))
        .collect::<Vec<_>>();

    items.extend(
        file.items
            .iter()
            .map(convert_item)
            .collect::<syn::Result<Vec<_>>>()?,
    );

    Ok(RustModule {
        name: module_name,
        docs: Vec::new(),
        items,
        attrs: Vec::new(),
        vis: Visibility::Private,
    })
}

fn convert_item(item: &SynItem) -> syn::Result<Item> {
    match item {
        SynItem::Enum(item_enum) => Ok(Item::Enum(EnumDef {
            name: item_enum.ident.to_string(),
            variants: item_enum
                .variants
                .iter()
                .map(convert_variant)
                .collect::<syn::Result<Vec<_>>>()?,
            generics: convert_generics(&item_enum.generics),
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
            generics: convert_generics(&item_fn.sig.generics),
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
            generics: convert_generics(&item_struct.generics),
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
            generics: convert_generics(&item_union.generics),
            derives: extract_derives(&item_union.attrs),
            docs: extract_docs(&item_union.attrs),
            vis: convert_visibility(&item_union.vis),
        })),
        SynItem::Type(item_type) => Ok(Item::TypeAlias(TypeAlias {
            name: item_type.ident.to_string(),
            target: convert_type(&item_type.ty)?,
            generics: convert_generics(&item_type.generics),
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
        SynItem::Use(item_use) => Ok(Item::Use(convert_use_tree(&item_use.tree))),
        SynItem::Impl(item_impl) => Ok(Item::Impl(ImplBlock {
            target: convert_type(&item_impl.self_ty)?,
            generics: convert_generics(&item_impl.generics),
            items: item_impl
                .items
                .iter()
                .map(convert_impl_item)
                .collect::<syn::Result<Vec<_>>>()?,
            trait_impl: item_impl
                .trait_
                .as_ref()
                .map(|(_, path, _)| convert_type_path(path))
                .transpose()?,
        })),
        SynItem::Mod(item_mod) if item_mod.content.is_none() => Ok(Item::Raw(format!(
            "{}mod {};",
            visibility_to_source(&convert_visibility(&item_mod.vis)),
            item_mod.ident
        ))),
        SynItem::Mod(item_mod) => {
            let nested_items = item_mod
                .content
                .as_ref()
                .map(|(_, items)| items.iter().map(convert_item).collect())
                .transpose()?
                .unwrap_or_default();

            Ok(Item::Mod(Box::new(RustModule {
                name: item_mod.ident.to_string(),
                docs: extract_docs(&item_mod.attrs),
                items: nested_items,
                attrs: convert_attributes(&item_mod.attrs),
                vis: convert_visibility(&item_mod.vis),
            })))
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
        docs: extract_docs(&field.attrs),
        attrs: convert_attributes(&field.attrs),
    })
}

fn convert_fn_arg(arg: &syn::FnArg) -> syn::Result<Param> {
    match arg {
        syn::FnArg::Receiver(receiver) => Ok(Param {
            name: if receiver.reference.is_some() {
                if receiver.mutability.is_some() {
                    "&mut self".to_string()
                } else {
                    "&self".to_string()
                }
            } else {
                "self".to_string()
            },
            ty: Type::Named("Self".to_string()),
        }),
        syn::FnArg::Typed(pat_type) => Ok(Param {
            name: convert_pat_name(&pat_type.pat)?,
            ty: convert_type(&pat_type.ty)?,
        }),
    }
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
            generics: convert_generics(&method.sig.generics),
            body: convert_block(&method.block)?,
            asyncness: method.sig.asyncness.is_some(),
            vis: Visibility::None,
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

fn convert_use_tree(tree: &syn::UseTree) -> UseStatement {
    match tree {
        syn::UseTree::Path(path) => {
            let mut converted = convert_use_tree(&path.tree);
            converted.path.insert(0, path.ident.to_string());
            converted
        }
        syn::UseTree::Name(name) => UseStatement {
            path: vec![name.ident.to_string()],
            kind: UseKind::Simple,
        },
        syn::UseTree::Glob(_) => UseStatement {
            path: Vec::new(),
            kind: UseKind::Glob,
        },
        syn::UseTree::Group(group) => UseStatement {
            path: Vec::new(),
            kind: UseKind::Nested(
                group
                    .items
                    .iter()
                    .map(use_tree_to_string)
                    .collect::<Vec<_>>(),
            ),
        },
        syn::UseTree::Rename(rename) => UseStatement {
            path: vec![format!("{} as {}", rename.ident, rename.rename)],
            kind: UseKind::Simple,
        },
    }
}

fn use_tree_to_string(tree: &syn::UseTree) -> String {
    tree.to_token_stream().to_string().replace(" :: ", "::")
}

fn convert_return_type(output: &ReturnType) -> syn::Result<Type> {
    match output {
        ReturnType::Default => Ok(Type::Unit),
        ReturnType::Type(_, ty) => convert_type(ty),
    }
}

fn convert_type(ty: &SynType) -> syn::Result<Type> {
    match ty {
        SynType::Path(type_path) => convert_type_path(&type_path.path),
        SynType::Reference(TypeReference {
            elem, mutability, ..
        }) => Ok(Type::Reference(
            Box::new(convert_type(elem)?),
            true,
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
            convert_callable_trait_bounds(bounds.iter(), CallableTraitQualifier::Impl)
        }
        SynType::Never(_) => Ok(Type::Never),
        SynType::Paren(TypeParen { elem, .. }) | SynType::Group(TypeGroup { elem, .. }) => {
            convert_type(elem)
        }
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported type syntax in RustLightAST parser",
        )),
    }
}

fn convert_callable_trait_bounds<'a>(
    bounds: impl Iterator<Item = &'a TypeParamBound>,
    qualifier: CallableTraitQualifier,
) -> syn::Result<Type> {
    let callable_bounds = bounds
        .filter_map(|bound| match bound {
            TypeParamBound::Trait(trait_bound) => Some(trait_bound),
            _ => None,
        })
        .collect::<Vec<_>>();

    if callable_bounds.len() != 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "unsupported callable trait bounds in RustLightAST parser",
        ));
    }

    convert_callable_trait_bound(callable_bounds[0], qualifier)
}

fn convert_callable_trait_bound(
    trait_bound: &TraitBound,
    qualifier: CallableTraitQualifier,
) -> syn::Result<Type> {
    let last =
        trait_bound.path.segments.last().ok_or_else(|| {
            syn::Error::new_spanned(&trait_bound.path, "empty callable trait path")
        })?;
    let trait_name = last.ident.to_string();

    let PathArguments::Parenthesized(ParenthesizedGenericArguments { inputs, output, .. }) =
        &last.arguments
    else {
        return Err(syn::Error::new_spanned(
            &last.arguments,
            "unsupported non-callable trait bound in RustLightAST parser",
        ));
    };

    let args = inputs
        .iter()
        .map(convert_type)
        .collect::<syn::Result<Vec<_>>>()?;
    let return_type = Box::new(convert_return_type(output)?);

    Ok(Type::CallableTrait(CallableTraitType {
        qualifier,
        trait_name,
        args,
        return_type,
    }))
}

fn convert_type_path(path: &syn::Path) -> syn::Result<Type> {
    let last = path
        .segments
        .last()
        .ok_or_else(|| syn::Error::new_spanned(path, "empty path"))?;
    let name = last.ident.to_string();

    let generic_args = match &last.arguments {
        PathArguments::None => Vec::new(),
        PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }) => args
            .iter()
            .filter_map(|arg| match arg {
                GenericArgument::Type(ty) => Some(convert_type(ty)),
                _ => None,
            })
            .collect::<syn::Result<Vec<_>>>()?,
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "unsupported path arguments in RustLightAST parser",
            ));
        }
    };

    if !generic_args.is_empty() {
        // Preserve path qualifiers so that e.g. `crate::Product_Type::Prod<A>`
        // roundtrips as `crate::Product_Type::Prod<A>` rather than bare `Prod<A>`.
        // Without this, source files that lack a `use crate::Product_Type::*;`
        // import fail to compile after optimiser rewriting.
        let qualified_name = if path.segments.len() > 1 {
            let prefix = path
                .segments
                .iter()
                .take(path.segments.len() - 1)
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            format!("{}::{}", prefix, name)
        } else {
            name
        };
        Ok(Type::Generic(qualified_name, generic_args))
    } else if path.segments.len() > 1 {
        Ok(Type::Path(
            path.segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect(),
        ))
    } else {
        Ok(Type::Named(name))
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

fn convert_block(block: &syn::Block) -> syn::Result<Block> {
    let mut stmts = Vec::new();
    let mut tail_expr = None;

    for stmt in &block.stmts {
        match stmt {
            Stmt::Local(local) => stmts.push(Statement::Let(convert_local(local)?)),
            Stmt::Item(item) => stmts.push(Statement::Item(Box::new(convert_item(item)?))),
            Stmt::Expr(expr, semi) => {
                if semi.is_some() {
                    stmts.push(Statement::Expr(convert_expr(expr)?));
                } else {
                    tail_expr = Some(Box::new(convert_expr(expr)?));
                }
            }
            Stmt::Macro(mac) => {
                // A statement-position macro can contain syntax outside the
                // lightweight AST (notably the `macro_rules!` declarations in
                // the native Word adapter). Preserve it verbatim as an opaque
                // item so the rest of the module, including explicit trait
                // impls, remains available to package-wide analyses.
                let source = normalize_tokens(mac.to_token_stream()).replace(" !", "!");
                stmts.push(Statement::Item(Box::new(Item::Raw(source))));
            }
        }
    }

    Ok(Block {
        stmts,
        expr: tail_expr,
    })
}

fn convert_local(local: &syn::Local) -> syn::Result<LetStmt> {
    let (name, ifmut) = match &local.pat {
        Pat::Ident(PatIdent {
            ident, mutability, ..
        }) => (ident.to_string(), mutability.is_some()),
        pat => (pat.to_token_stream().to_string(), false),
    };

    let (ty, init) = match &local.init {
        Some(LocalInit { expr, .. }) => (None, Some(convert_expr(expr)?)),
        None => (None, None),
    };

    Ok(LetStmt {
        ifmut,
        name,
        ty,
        init,
    })
}

fn convert_expr(expr: &syn::Expr) -> syn::Result<Expr> {
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
        syn::Expr::Block(ExprBlock { block, .. }) => Ok(Expr::Block(convert_block(block)?)),
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
        }) => Ok(Expr::If {
            condition: Box::new(convert_expr(cond)?),
            then_branch: convert_block(then_branch)?,
            else_branch: else_branch
                .as_ref()
                .map(|(_, expr)| expr_to_block(expr))
                .transpose()?,
        }),
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
        syn::Expr::Field(ExprField { base, member, .. }) => {
            let mut path = flatten_member_path(base)?;
            path.push(member.to_token_stream().to_string());
            Ok(Expr::Path(path, PathType::Member))
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
            let source = normalize_tokens(expr_macro.to_token_stream()).replace(" !", "!");
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
        syn::Expr::Block(ExprBlock { block, .. }) => convert_block(block),
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
        pattern: normalize_tokens(arm.pat.to_token_stream()),
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

    let segments = expr_path
        .path
        .segments
        .iter()
        .map(path_segment_source)
        .collect::<Vec<_>>();

    if segments.len() == 1 {
        Ok(Expr::Ident(segments[0].clone()))
    } else {
        Ok(Expr::Path(segments, PathType::Namespace))
    }
}

fn flatten_member_path(expr: &syn::Expr) -> syn::Result<Vec<String>> {
    match expr {
        syn::Expr::Path(ExprPath { path, .. }) => {
            Ok(path.segments.iter().map(path_segment_source).collect())
        }
        syn::Expr::Field(ExprField { base, member, .. }) => {
            let mut segments = flatten_member_path(base)?;
            segments.push(member.to_token_stream().to_string());
            Ok(segments)
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            "unsupported member expression base",
        )),
    }
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

fn convert_generics(generics: &syn::Generics) -> Vec<GenericParam> {
    let mut params = generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(type_param) => Some(GenericParam {
                name: type_param.ident.to_string(),
                bounds: type_param
                    .bounds
                    .iter()
                    .map(|bound| normalize_tokens(bound.to_token_stream()))
                    .collect(),
            }),
            _ => None,
        })
        .collect::<Vec<_>>();

    if let Some(where_clause) = &generics.where_clause {
        for predicate in &where_clause.predicates {
            let syn::WherePredicate::Type(predicate) = predicate else {
                continue;
            };
            let SynType::Path(type_path) = &predicate.bounded_ty else {
                continue;
            };
            if type_path.path.segments.len() != 1 {
                continue;
            }

            let name = type_path.path.segments[0].ident.to_string();
            if let Some(param) = params.iter_mut().find(|param| param.name == name) {
                for bound in &predicate.bounds {
                    let bound = normalize_tokens(bound.to_token_stream());
                    if !param.bounds.iter().any(|existing| existing == &bound) {
                        param.bounds.push(bound);
                    }
                }
            }
        }
    }

    params
}

fn convert_visibility(vis: &SynVisibility) -> Visibility {
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

fn visibility_to_source(vis: &Visibility) -> String {
    match vis {
        Visibility::Public => "pub ".to_string(),
        Visibility::Private | Visibility::None => String::new(),
        Visibility::Restricted(path) => format!("pub(in {}) ", path.join("::")),
    }
}

fn attribute_to_raw_source(attr: &syn::Attribute) -> String {
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

fn convert_attributes(attrs: &[syn::Attribute]) -> Vec<Attribute> {
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

fn extract_docs(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter_map(|attr| {
            if !attr.path().is_ident("doc") {
                return None;
            }
            match &attr.meta {
                Meta::NameValue(name_value) => {
                    if let syn::Expr::Lit(ExprLit {
                        lit: Lit::Str(doc), ..
                    }) = &name_value.value
                    {
                        Some(format!("///{}", doc.value()))
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

fn convert_pat_name(pat: &Pat) -> syn::Result<String> {
    match pat {
        Pat::Ident(pat_ident) => Ok(pat_ident.ident.to_string()),
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported parameter pattern in RustLightAST parser",
        )),
    }
}

fn normalize_tokens(tokens: TokenStream) -> String {
    tokens
        .to_string()
        .replace(" :: ", "::")
        .replace(" (", "(")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace(" ,", ",")
}

fn normalize_path_tokens(tokens: TokenStream) -> String {
    normalize_tokens(tokens)
        .replace("< ", "<")
        .replace(" >", ">")
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
    use crate::{Expr, Item, RustCodeGenerator, Statement, Type};

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
                            assert_eq!(arms[2].pattern, "Option::Rec(p0)");
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
pub fn unsupported() { let _bytes = [0u8; 4]; }
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
                        if callable.trait_name == "Fn"
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
            if callable.trait_name == "Fn"
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
