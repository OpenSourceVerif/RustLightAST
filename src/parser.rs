use proc_macro2::TokenStream;
use syn::{
    AngleBracketedGenericArguments, ExprArray, ExprAssign, ExprBinary, ExprBlock, ExprCall,
    ExprField, ExprGroup, ExprIf, ExprIndex, ExprLit, ExprMatch, ExprMethodCall, ExprParen,
    ExprPath, ExprReference, ExprTuple, ExprUnary, File, GenericArgument, ImplItem as SynImplItem,
    Item as SynItem, Lit, LocalInit, Meta, Pat, PatIdent, PathArguments, ReturnType, Stmt,
    Type as SynType, TypeArray, TypeGroup, TypeParen, TypeReference, TypeSlice,
    TypeTuple, Visibility as SynVisibility, parse_file,
};

use crate::intermediate_ast::{
    Attribute, AttributeArg, Block, ConstDef, EnumDef, Expr, Field, FunctionDef, GenericParam,
    ImplBlock, ImplItem, Item, LetStmt, Literal, MatchArm, Param, PathType, RustModule,
    Statement, StructDef, Type, TypeAlias, UnionDef, UseKind, UseStatement, Variant, Visibility,
};
use crate::intermediate_print::RustCodeGenerator;

pub fn parse_rust_source(source: &str, module_name: impl Into<String>) -> syn::Result<RustModule> {
    let file = parse_file(source)?;
    convert_file(file, module_name.into())
}

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
    let items = file
        .items
        .iter()
        .map(convert_item)
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(RustModule {
        name: module_name,
        docs: extract_docs(&file.attrs),
        items,
        attrs: convert_attributes(&file.attrs),
        vis: Visibility::Private,
        withs: Vec::new(),
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
            properties: Vec::new(),
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
            properties: Vec::new(),
            generics: convert_generics(&item_union.generics),
            derives: extract_derives(&item_union.attrs),
            docs: extract_docs(&item_union.attrs),
            vis: convert_visibility(&item_union.vis),
        })),
        SynItem::Type(item_type) => Ok(Item::TypeAlias(TypeAlias {
            name: item_type.ident.to_string(),
            target: convert_type(&item_type.ty)?,
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
                withs: Vec::new(),
            })))
        }
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
            path: vec![rename.ident.to_string()],
            kind: UseKind::Nested(vec![format!("{} as {}", rename.ident, rename.rename)]),
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
            elem,
            mutability,
            ..
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
        Ok(Type::Generic(name, generic_args))
    } else if path.segments.len() > 1 {
        Ok(Type::Path(
            path.segments.iter().map(|segment| segment.ident.to_string()).collect(),
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
                return Err(syn::Error::new_spanned(
                    mac,
                    "unsupported statement macro in RustLightAST parser",
                ));
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
        syn::Expr::Path(ExprPath { path, .. }) => convert_expr_path(path),
        syn::Expr::Call(ExprCall { func, args, .. }) => Ok(Expr::Call(
            Box::new(convert_expr(func)?),
            args.iter().map(convert_expr).collect::<syn::Result<Vec<_>>>()?,
        )),
        syn::Expr::MethodCall(ExprMethodCall {
            receiver,
            method,
            args,
            ..
        }) => Ok(Expr::MethodCall(
            Box::new(convert_expr(receiver)?),
            method.to_string(),
            args.iter().map(convert_expr).collect::<syn::Result<Vec<_>>>()?,
        )),
        syn::Expr::Match(ExprMatch { expr, arms, .. }) => Ok(Expr::Match {
            expr: Box::new(convert_expr(expr)?),
            arms: arms.iter().map(convert_match_arm).collect::<syn::Result<Vec<_>>>()?,
        }),
        syn::Expr::Block(ExprBlock { block, .. }) => Ok(Expr::Block(convert_block(block)?)),
        syn::Expr::Reference(ExprReference {
            expr, mutability, ..
        }) => Ok(Expr::Reference(
            Box::new(convert_expr(expr)?),
            true,
            mutability.is_some(),
        )),
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
        syn::Expr::Tuple(ExprTuple { elems, .. }) => Ok(Expr::Call(
            Box::new(Expr::Parenthesized(Box::new(Expr::Ident(String::new())))),
            elems.iter().map(convert_expr).collect::<syn::Result<Vec<_>>>()?,
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
        syn::Expr::Array(ExprArray { elems, .. }) => Ok(Expr::Call(
            Box::new(Expr::Ident("vec!".to_string())),
            elems.iter().map(convert_expr).collect::<syn::Result<Vec<_>>>()?,
        )),
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported expression syntax in RustLightAST parser",
        )),
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

fn convert_expr_path(path: &syn::Path) -> syn::Result<Expr> {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();

    if segments.len() == 1 {
        Ok(Expr::Ident(segments[0].clone()))
    } else {
        Ok(Expr::Path(segments, PathType::Namespace))
    }
}

fn flatten_member_path(expr: &syn::Expr) -> syn::Result<Vec<String>> {
    match expr {
        syn::Expr::Path(ExprPath { path, .. }) => Ok(path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect()),
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

fn convert_literal(lit: &Lit) -> syn::Result<Literal> {
    match lit {
        Lit::Int(int_lit) => Ok(Literal::Int(int_lit.base10_parse()?)),
        Lit::Float(float_lit) => Ok(Literal::Float(float_lit.base10_parse()?)),
        Lit::Str(str_lit) => Ok(Literal::Str(str_lit.value())),
        Lit::Bool(bool_lit) => Ok(Literal::Bool(bool_lit.value)),
        Lit::Char(char_lit) => Ok(Literal::Char(char_lit.value())),
        other => Err(syn::Error::new_spanned(
            other,
            "unsupported literal syntax in RustLightAST parser",
        )),
    }
}

fn convert_generics(generics: &syn::Generics) -> Vec<GenericParam> {
    generics
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
        .collect()
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
                        lit: Lit::Str(doc),
                        ..
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
    use super::parse_rust_source;
    use crate::intermediate_ast::{Expr, Item, Type};
    use crate::intermediate_print::RustCodeGenerator;

    #[test]
    fn parses_rec_get_sample_and_prints_it() {
        let source = include_str!("../tests/Rec_Get_Tests.rs");
        let module = parse_rust_source(source, "Rec_Get_Tests").expect("sample parses");

        assert_eq!(module.name, "Rec_Get_Tests");
        assert_eq!(module.items.len(), 5);

        match &module.items[0] {
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

        match &module.items[3] {
            Item::Function(def) => {
                assert_eq!(def.name, "get");
                assert_eq!(def.params.len(), 1);
                match &def.body.expr {
                    Some(expr) => match expr.as_ref() {
                        Expr::Match { expr, arms } => {
                            assert!(matches!(expr.as_ref(), Expr::Ident(name) if name == "x0"));
                            assert_eq!(arms.len(), 3);
                            assert_eq!(arms[2].pattern, "Option::Rec(op)");
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
                                            && method == "as_ref"
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
        assert!(printed.contains("pub fn get(x0: &Option) -> Int"));
        assert!(printed.contains("Option::Rec(op) => {"));
        assert!(printed.contains("get(op.as_ref())"));
        assert!(!printed.is_empty());
    }
}
