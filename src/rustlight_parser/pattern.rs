use super::{convert_literal, convert_member, convert_type, path_segments, ParseError};
use crate::{BindingMode, Pattern, PatternField};
use syn::{
    parse::{ParseStream, Parser},
    Pat, Token,
};

/// Parse a pattern in isolation, including a top-level type annotation.
pub fn parse_rust_pattern(source: &str) -> Result<Pattern, ParseError> {
    parse_pattern.parse_str(source)
}

fn parse_pattern(input: ParseStream<'_>) -> syn::Result<Pattern> {
    let pattern = convert_pattern(&Pat::parse_multi_with_leading_vert(input)?)?;
    if input.peek(Token![:]) {
        input.parse::<Token![:]>()?;
        Ok(Pattern::Typed {
            pattern: Box::new(pattern),
            ty: convert_type(&input.parse()?)?,
        })
    } else {
        Ok(pattern)
    }
}

pub(super) fn convert_pattern(pattern: &Pat) -> syn::Result<Pattern> {
    match pattern {
        Pat::Ident(binding) => Ok(Pattern::Ident {
            name: binding.ident.to_string(),
            mode: match (binding.by_ref.is_some(), binding.mutability.is_some()) {
                (false, false) => BindingMode::Value,
                (false, true) => BindingMode::Mut,
                (true, false) => BindingMode::Ref,
                (true, true) => BindingMode::RefMut,
            },
            subpattern: binding
                .subpat
                .as_ref()
                .map(|(_, subpattern)| convert_pattern(subpattern).map(Box::new))
                .transpose()?,
        }),
        Pat::Path(path) => Ok(Pattern::Path(pattern_path(&path.path, &path.qself)?)),
        Pat::Lit(literal) => Ok(Pattern::Literal(convert_literal(&literal.lit)?)),
        Pat::Wild(_) => Ok(Pattern::Wildcard),
        Pat::Rest(_) => Ok(Pattern::Rest),
        Pat::Tuple(tuple) => Ok(Pattern::Tuple(convert_patterns(&tuple.elems)?)),
        Pat::TupleStruct(constructor) => Ok(Pattern::TupleStruct {
            path: pattern_path(&constructor.path, &constructor.qself)?,
            fields: convert_patterns(&constructor.elems)?,
        }),
        Pat::Struct(record) => Ok(Pattern::Record {
            path: pattern_path(&record.path, &record.qself)?,
            fields: record
                .fields
                .iter()
                .map(convert_pattern_field)
                .collect::<syn::Result<_>>()?,
            rest: record.rest.is_some(),
        }),
        Pat::Or(alternatives) => Ok(Pattern::Or(convert_patterns(&alternatives.cases)?)),
        Pat::Slice(slice) => Ok(Pattern::Slice(convert_patterns(&slice.elems)?)),
        Pat::Type(typed) => Ok(Pattern::Typed {
            pattern: Box::new(convert_pattern(&typed.pat)?),
            ty: convert_type(&typed.ty)?,
        }),
        Pat::Reference(reference) => Ok(Pattern::Reference {
            pattern: Box::new(convert_pattern(&reference.pat)?),
            mutable: reference.mutability.is_some(),
        }),
        Pat::Paren(paren) => Ok(Pattern::Parenthesized(Box::new(convert_pattern(
            &paren.pat,
        )?))),
        other => Err(ParseError::new_spanned(
            other,
            "unsupported RustLight pattern",
        )),
    }
}

fn convert_patterns<'a>(patterns: impl IntoIterator<Item = &'a Pat>) -> syn::Result<Vec<Pattern>> {
    patterns.into_iter().map(convert_pattern).collect()
}

fn pattern_path(path: &syn::Path, qualified_self: &Option<syn::QSelf>) -> syn::Result<Vec<String>> {
    if let Some(qualified_self) = qualified_self {
        return Err(ParseError::new_spanned(
            &qualified_self.ty,
            "unsupported qualified path in pattern",
        ));
    }
    Ok(path_segments(path))
}

fn convert_pattern_field(field: &syn::FieldPat) -> syn::Result<PatternField> {
    // DEFENSE: a `cfg` on a record pattern field controls whether it binds.
    if !field.attrs.is_empty() {
        return Err(ParseError::new_spanned(
            field,
            "unsupported attribute on pattern field",
        ));
    }
    Ok(PatternField {
        member: convert_member(&field.member),
        pattern: convert_pattern(&field.pat)?,
        shorthand: field.colon_token.is_none(),
    })
}
