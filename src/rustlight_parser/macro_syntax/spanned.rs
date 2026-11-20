//! Grammars used to lower macro operands during ingestion.

use syn::ext::IdentExt;

fn canonical_identifier(name: &syn::Ident) -> String {
    name.unraw().to_string()
}

pub fn thread_local_statics(tokens: proc_macro2::TokenStream) -> Option<Vec<syn::ItemStatic>> {
    fn contains_impl(tokens: proc_macro2::TokenStream) -> bool {
        tokens.into_iter().any(|token| match token {
            proc_macro2::TokenTree::Ident(ident) => ident == "impl",
            proc_macro2::TokenTree::Group(group) => contains_impl(group.stream()),
            _ => false,
        })
    }
    if contains_impl(tokens.clone()) {
        return None;
    }
    let file = syn::parse2::<syn::File>(tokens).ok()?;
    if !file.attrs.is_empty() || file.shebang.is_some() {
        return None;
    }
    file.items
        .into_iter()
        .map(|item| match item {
            syn::Item::Static(item)
                if item.attrs.iter().all(|attribute| {
                    matches!(
                        attribute
                            .path()
                            .get_ident()
                            .map(canonical_identifier)
                            .as_deref(),
                        Some("allow" | "warn" | "deny" | "forbid" | "doc" | "cfg")
                    )
                }) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect()
}

pub(super) fn format_arguments(expressions: &[syn::Expr]) -> Option<Vec<String>> {
    let Some(syn::Expr::Lit(literal)) = expressions.first() else {
        return None;
    };
    let syn::Lit::Str(template) = &literal.lit else {
        return None;
    };
    if expressions
        .iter()
        .skip(1)
        .any(|expression| matches!(expression, syn::Expr::Assign(_)))
    {
        return None;
    }
    format_captures(&template.value())
}

// Captured names have lexical binding identities, but are not source Expr nodes.
fn format_captures(template: &str) -> Option<Vec<String>> {
    let bytes = template.as_bytes();
    let mut captures = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' if bytes.get(index + 1) == Some(&b'{') => index += 2,
            b'}' if bytes.get(index + 1) == Some(&b'}') => index += 2,
            b'{' => {
                let start = index + 1;
                let end = start + bytes[start..].iter().position(|byte| *byte == b'}')?;
                let argument = &template[start..end];
                let (argument, specification) = argument.split_once(':').unwrap_or((argument, ""));
                // KNOWN LIMITATION: dynamic width/precision remains opaque until
                // its separate formatting inputs are represented as captures.
                if specification.contains('$') || specification.contains('{') {
                    return None;
                }
                if !argument.is_empty() && !argument.bytes().all(|byte| byte.is_ascii_digit()) {
                    let path = syn::parse_str::<syn::Path>(argument).ok()?;
                    let name = canonical_identifier(path.get_ident()?);
                    if seen.insert(name.clone()) {
                        captures.push(name);
                    }
                }
                index = end + 1;
            }
            b'}' => return None,
            _ => index += 1,
        }
    }
    Some(captures)
}

pub(in crate::rustlight_parser) fn parse_match_test(
    input: syn::parse::ParseStream<'_>,
) -> syn::Result<(
    syn::Expr,
    syn::Pat,
    Option<(syn::Token![if], Box<syn::Expr>)>,
)> {
    let value = input.parse()?;
    input.parse::<syn::Token![,]>()?;
    let pattern = syn::Pat::parse_multi_with_leading_vert(input)?;
    let guard = if input.peek(syn::Token![if]) {
        Some((input.parse()?, Box::new(input.parse()?)))
    } else {
        None
    };
    if input.peek(syn::Token![,]) {
        input.parse::<syn::Token![,]>()?;
    }
    Ok((value, pattern, guard))
}

pub(in crate::rustlight_parser) fn parse_repeat_count(input: syn::parse::ParseStream<'_>) -> syn::Result<syn::Expr> {
    input.parse::<syn::Token![;]>()?;
    input.parse()
}
