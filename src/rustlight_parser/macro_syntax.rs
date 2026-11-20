//! Syntax projections for macros whose spelling stays authoritative.

use proc_macro2::{Delimiter, Group, Ident, Span, TokenStream, TokenTree};
use quote::ToTokens;
use syn::{ext::IdentExt, parse::Parser, punctuated::Punctuated, Token};

use super::{convert_expr, convert_item, path_segments};
use crate::{
    IdentifierMacroFunctionName, IdentifierMacroFunctionTemplate, Item,
    MacroInput, MacroMatchInput, MacroStatic,
    SpecializedMacroInput,
};

mod spanned;
use spanned::thread_local_statics;

pub(super) fn input(source: String, tokens: &TokenStream) -> MacroInput {
    let identifiers = Punctuated::<Ident, Token![,]>::parse_terminated
        .parse2(tokens.clone())
        .ok()
        .map(|idents| {
            let trailing_comma = idents.trailing_punct();
            (
                idents.into_iter().map(|ident| ident.to_string()).collect(),
                trailing_comma,
            )
        });
    let parsed = Punctuated::<syn::Expr, Token![,]>::parse_terminated
        .parse2(tokens.clone())
        .ok();
    let expressions = parsed.as_ref().and_then(|expressions| {
        super::origins::attempt(|| expressions.iter().map(convert_expr).collect::<Result<_, _>>())
            .ok()
    });
    let may_be_match = parsed
        .as_ref()
        .is_none_or(|expressions| expressions.len() == 2);
    let facts = token_facts(tokens.clone());
    let input = MacroInput::new(
        source,
        expressions,
        identifiers,
        facts.identifiers,
        facts.count,
        (!facts.opaque).then_some(facts.macro_paths),
    );
    let mut specialized = match parsed {
        Some(expressions) => {
            let expressions: Vec<_> = expressions.into_iter().collect();
            let format_captures = std::array::from_fn(|leading| {
                spanned::format_arguments(expressions.get(leading..)?)
            });
            SpecializedMacroInput {
                format_captures,
                ..SpecializedMacroInput::default()
            }
        }
        None => {
            let repeat = super::origins::attempt(|| (|input: syn::parse::ParseStream<'_>| {
                let value: syn::Expr = input.parse()?;
                let count = spanned::parse_repeat_count(input)?;
                Ok((convert_expr(&value)?, convert_expr(&count)?))
            })
            .parse2(tokens.clone())).ok();
            let statics = thread_local_statics(tokens.clone()).map(convert_statics);
            SpecializedMacroInput {
                repeat,
                statics,
                ..SpecializedMacroInput::default()
            }
        }
    };
    // A pattern such as a literal or a path may also parse as an expression.
    if may_be_match {
        specialized.match_test = super::origins::attempt(|| (|input: syn::parse::ParseStream<'_>| {
            let (value, pattern, guard) = spanned::parse_match_test(input)?;
            Ok(MacroMatchInput {
                value: convert_expr(&value)?,
                pattern: super::convert_pattern(&pattern)?,
                guard: guard.map(|(_, guard)| convert_expr(&guard)).transpose()?,
            })
        })
        .parse2(tokens.clone())).ok();
    }
    if tokens.is_empty() {
        specialized.statics = Some(Vec::new());
    }
    input.with_specialized_syntax(specialized)
}

fn convert_statics(statics: Vec<syn::ItemStatic>) -> Vec<MacroStatic> {
    statics.iter().map(convert_static).collect()
}

fn convert_static(item: &syn::ItemStatic) -> MacroStatic {
    MacroStatic {
        name: item.ident.to_string(),
        visibility: super::convert_visibility(&item.vis),
        attrs: super::convert_attributes(&item.attrs),
        ty: super::convert_type(&item.ty).ok(),
        initializer: convert_expr(&item.expr).ok(),
    }
}

pub(super) fn item_function_template(
    item: &syn::ItemMacro,
) -> Option<IdentifierMacroFunctionTemplate> {
    if item.ident.is_some()
        && item.mac.path.is_ident("macro_rules")
        && item.attrs.iter().all(|attribute| {
            ["allow", "warn", "deny", "forbid", "doc", "macro_export"]
                .iter()
                .any(|name| attribute.path().is_ident(name))
        })
    {
        super::origins::without_origins(|| function_template(item.mac.tokens.clone()))
    } else {
        None
    }
}

/// Find nested invocation paths if the payload has no attribute or
/// implementation tokens. Resolving those paths is a consumer's responsibility.
pub(super) fn macro_nested_paths(tokens: TokenStream) -> Option<Vec<Vec<String>>> {
    let facts = token_facts(tokens);
    (!facts.opaque).then_some(facts.macro_paths)
}

#[derive(Default)]
struct TokenFacts {
    identifiers: Vec<String>,
    count: usize,
    macro_paths: Vec<Vec<String>>,
    opaque: bool,
}

fn token_facts(tokens: TokenStream) -> TokenFacts {
    fn collect(tokens: TokenStream, facts: &mut TokenFacts) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        facts.count += tokens.len();
        for (index, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Ident(name) => {
                    facts.identifiers.push(name.to_string());
                    facts.opaque |= name == "impl" || name == "derive";
                }
                TokenTree::Punct(value) if value.as_char() == '#' => facts.opaque = true,
                TokenTree::Punct(value) if value.as_char() == '!' => {
                    if index == 0
                        || !matches!(tokens[index - 1], TokenTree::Ident(_))
                        || !matches!(tokens.get(index + 1), Some(TokenTree::Group(_)))
                    {
                        continue;
                    }
                    let mut start = index - 1;
                    while start >= 2
                        && punctuation(&tokens[start - 1], ':')
                        && punctuation(&tokens[start - 2], ':')
                    {
                        start -= 2;
                        if start > 0 && matches!(tokens[start - 1], TokenTree::Ident(_)) {
                            start -= 1;
                        } else {
                            break;
                        }
                    }
                    // An expression keyword before unary ! is not a macro path.
                    let path_tokens = tokens[start..index].iter().cloned().collect();
                    if let Ok(path) = syn::parse2::<syn::Path>(path_tokens) {
                        facts.macro_paths.push(path_segments(&path));
                    }
                }
                TokenTree::Group(group) => collect(group.stream(), facts),
                _ => {}
            }
        }
    }

    let mut facts = TokenFacts::default();
    collect(tokens, &mut facts);
    facts
}

fn punctuation(token: &TokenTree, expected: char) -> bool {
    matches!(token, TokenTree::Punct(value) if value.as_char() == expected)
}

fn function_template(tokens: TokenStream) -> Option<IdentifierMacroFunctionTemplate> {
    let parse_rule = |input: syn::parse::ParseStream<'_>| {
        let pattern: proc_macro2::Group = input.parse()?;
        input.parse::<Token![=>]>()?;
        let body: proc_macro2::Group = input.parse()?;
        if !input.is_empty() {
            input.parse::<Token![;]>()?;
        }
        Ok((pattern, body))
    };
    let (pattern, body) = parse_rule.parse2(tokens).ok()?;
    if pattern.delimiter() != Delimiter::Parenthesis || body.delimiter() != Delimiter::Brace {
        return None;
    }
    let (parameters, trailing_comma) = matcher_parameters(pattern.stream())?;
    let body = body.stream();
    let mut prefix = "__rustlight_macro_parameter_".to_owned();
    while contains_identifier_prefix(body.clone(), &prefix) {
        prefix.push('_');
    }
    let markers: Vec<_> = (0..parameters.len())
        .map(|index| format!("{prefix}{index}"))
        .collect();
    let substituted = substitute_parameters(body, &parameters, &markers)?;
    if !macro_nested_paths(substituted.clone())?.is_empty() {
        return None;
    }

    let file = syn::parse2::<syn::File>(substituted).ok()?;
    if !file.attrs.is_empty() || file.shebang.is_some() {
        return None;
    }
    #[derive(Default)]
    struct LocalItems(bool);
    impl<'ast> syn::visit::Visit<'ast> for LocalItems {
        fn visit_item(&mut self, _: &'ast syn::Item) {
            self.0 = true;
        }
    }
    let mut local_items = LocalItems::default();
    let functions = file
        .items
        .iter()
        .map(|item| {
            let syn::Item::Fn(source) = item else {
                return None;
            };
            syn::visit::visit_item_fn(&mut local_items, source);
            let Item::Function(mut function) = convert_item(item).ok()? else {
                return None;
            };
            if let crate::Visibility::Restricted(path) = &function.vis {
                if path.iter().any(|segment| markers.contains(segment)) {
                    // Substitution into visibility paths needs a visibility template.
                    return None;
                }
            }
            let name = markers
                .iter()
                .position(|marker| marker == &function.name)
                .map(IdentifierMacroFunctionName::Argument)
                .unwrap_or_else(|| IdentifierMacroFunctionName::Literal(function.name.clone()));
            let mut signature = source.sig.clone();
            signature.ident = Ident::new("function", signature.ident.span());
            let fixed_signature = !contains_identifier_prefix(signature.into_token_stream(), &prefix)
                && function.attrs.is_empty();
            let visibility = function.vis.clone();
            function.body = crate::Block { stmts: Vec::new(), expr: None };
            function.docs.clear();
            Some((name, visibility, fixed_signature.then_some(function)))
        })
        .collect::<Option<Vec<_>>>()?;
    IdentifierMacroFunctionTemplate::new(parameters.len(), trailing_comma, functions, local_items.0)
}

fn matcher_parameters(tokens: TokenStream) -> Option<(Vec<String>, bool)> {
    let parse_parameters = |input: syn::parse::ParseStream<'_>| {
        Punctuated::<Ident, Token![,]>::parse_terminated_with(input, |input| {
            input.parse::<Token![$]>()?;
            let name = input.parse::<Ident>()?;
            input.parse::<Token![:]>()?;
            let fragment = input.parse::<Ident>()?;
            if fragment != "ident" {
                return Err(syn::Error::new(
                    fragment.span(),
                    "expected an identifier matcher",
                ));
            }
            Ok(name)
        })
    };
    let parsed = parse_parameters.parse2(tokens).ok()?;
    let trailing_comma = parsed.trailing_punct();
    let mut names = Vec::new();
    for name in parsed {
        let name = name.unraw().to_string();
        if names.contains(&name) {
            return None;
        }
        names.push(name);
    }
    Some((names, trailing_comma))
}

fn contains_identifier_prefix(tokens: TokenStream, prefix: &str) -> bool {
    tokens.into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident.to_string().starts_with(prefix),
        TokenTree::Group(group) => contains_identifier_prefix(group.stream(), prefix),
        _ => false,
    })
}

fn substitute_parameters(
    tokens: TokenStream,
    parameters: &[String],
    markers: &[String],
) -> Option<TokenStream> {
    let mut tokens = tokens.into_iter();
    let mut result = TokenStream::new();
    while let Some(token) = tokens.next() {
        let token = match token {
            token if punctuation(&token, '$') => {
                let TokenTree::Ident(name) = tokens.next()? else {
                    return None;
                };
                let name = name.unraw().to_string();
                let index = parameters.iter().position(|parameter| parameter == &name)?;
                TokenTree::Ident(Ident::new(&markers[index], Span::call_site()))
            }
            TokenTree::Group(group) => TokenTree::Group(Group::new(
                group.delimiter(),
                substitute_parameters(group.stream(), parameters, markers)?,
            )),
            token => token,
        };
        result.extend([token]);
    }
    Some(result)
}
