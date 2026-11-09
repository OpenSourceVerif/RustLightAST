use rustlightast::{
    parse_rust_source, parse_rust_type_facts,
    rustlight_parser::{first_explicit_type_argument, TYPE_FACT_ONLY_DOC},
    Expr, Item, Member, ParseError, RustCodeGenerator, Type,
};

#[test]
fn downstream_consumer_can_parse_and_print_without_an_optimizer() {
    let module: rustlightast::RustModule =
        parse_rust_source("pub fn identity(x: u32) -> u32 { x }", "example").unwrap();
    assert_eq!(module.name, "example");
    let Item::Function(function) = &module.items[0] else {
        panic!("expected function");
    };
    assert!(matches!(function.body.expr.as_deref(), Some(Expr::Ident(name)) if name == "x"));
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    parse_rust_source(&printed, "roundtrip").unwrap();
}

#[test]
fn unsupported_body_still_allows_marked_type_fact_recovery() {
    let source = "pub fn bytes() -> [u8; 4] { const { [0u8; 4] } }";
    let _: ParseError = parse_rust_source(source, "example").unwrap_err();
    let facts = parse_rust_type_facts(source, "example").unwrap();
    let Item::Function(function) = &facts.items[0] else {
        panic!("expected recovered signature");
    };
    assert!(function.docs.iter().any(|doc| doc == TYPE_FACT_ONLY_DOC));
    assert!(matches!(&function.return_type, Type::Array(_, 4)));
    assert!(function.body.stmts.is_empty());
    assert!(function.body.expr.is_none());
    assert!(parse_rust_type_facts("fn {", "invalid").is_err());
}

#[test]
fn optimizer_type_argument_helper_remains_available() {
    assert!(
        matches!(first_explicit_type_argument("identity::<u32, bool>"),
        Some(Type::Named(name)) if name == "u32")
    );
    assert!(first_explicit_type_argument("identity").is_none());
    assert!(first_explicit_type_argument("identity::<").is_none());
}

#[test]
fn macro_statements_roundtrip_delimiters_attributes_and_semicolons() {
    use rustlightast::{MacroDelimiter, Statement};

    let source = r#"
#[macro_export]
macro_rules! add {
    ($out:ident, $value:expr) => { $out = $out + $value; };
}
fn total() -> u32 {
    let mut sum = 0;
    #[cfg(any())]
    crate::add!(sum, 100);
    crate::add!(sum, (1));
    crate::add![sum, 2];
    crate::add! { sum, 4 }
    crate::add! { sum, 8 };
    sum
}
"#;
    let module = parse_rust_source(source, "macros").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    let reparsed = parse_rust_source(&printed, "roundtrip").unwrap();

    for module in [&module, &reparsed] {
        let function = module
            .items
            .iter()
            .find_map(|item| match item {
                Item::Function(function) => Some(function),
                _ => None,
            })
            .unwrap();
        let statements: Vec<_> = function
            .body
            .stmts
            .iter()
            .filter_map(|statement| match statement {
                Statement::Macro(statement) => Some(statement),
                _ => None,
            })
            .collect();
        assert_eq!(
            statements
                .iter()
                .map(|statement| { (statement.delimiter, statement.semicolon) })
                .collect::<Vec<_>>(),
            [
                (MacroDelimiter::Parenthesis, true),
                (MacroDelimiter::Parenthesis, true),
                (MacroDelimiter::Bracket, true),
                (MacroDelimiter::Brace, false),
                (MacroDelimiter::Brace, true),
            ]
        );
        assert!(statements
            .iter()
            .all(|statement| statement.path == "crate::add"));
        assert_eq!(statements[0].attrs.len(), 1);
        assert!(statements[0].attrs[0].contains("cfg"));
        assert!(statements[1].tokens.contains("(1)"));
    }
}

#[test]
fn parser_rejects_labels_in_control_flow() {
    for source in [
        "fn f() { 'outer: loop { break 'outer; } }",
        "fn f() { 'outer: while false {} }",
        "fn f() { 'outer: for _ in [true] {} }",
        "fn f() { 'outer: { break 'outer; } }",
        "fn f() { loop { continue 'outer; } }",
        "fn f() { loop { break 'outer; } }",
    ] {
        assert!(
            parse_rust_source(source, "unsupported")
                .unwrap_err()
                .to_string()
                .contains("unsupported expression label"),
            "{source}"
        );
    }
}

#[test]
fn loop_patterns_and_jump_values_roundtrip() {
    let source = r#"
fn evaluate(mut items: Vec<(u8, u8)>, stop: bool) -> u8 {
    let mut sum = 0;
    for (a, b) in [(1, 2), (3, 4)] {
        sum = sum + a + b;
    }
    while let Some((x, y)) = items.pop() {
        sum = sum + x + y;
    }
    loop {
        if stop { return sum }
        if sum == 10 { break sum + 1; }
        sum = sum - 1;
        continue;
    }
}
"#;
    let module = parse_rust_source(source, "loops").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("for (a, b) in"));
    assert!(printed.contains("while let Some((x, y)) = items.pop()"));
    assert!(printed.contains("return sum"));
    assert!(printed.contains("break sum + 1"));
    assert!(printed.contains("continue;"));
    parse_rust_source(&printed, "roundtrip").unwrap();
}

#[test]
fn record_fields_can_be_rewritten_without_losing_shorthand_or_update_order() {
    let source = r#"
fn row(original: Row, tag: u8, second: u8) -> Row {
    crate::Row { tag, first: get(1)[0].0, second, ..original }
}
"#;
    let mut module = parse_rust_source(source, "record").unwrap();
    let Item::Function(function) = &mut module.items[0] else {
        panic!("expected function");
    };
    let Some(Expr::Record { fields, .. }) = function.body.expr.as_deref_mut() else {
        panic!("expected record initializer");
    };
    fields[0].value = Expr::Ident("new_tag".into());
    let Expr::Field {
        base,
        member: Member::Index(0),
    } = &fields[1].value
    else {
        panic!("expected tuple-field access");
    };
    assert!(matches!(base.as_ref(), Expr::Index(..)));

    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("crate::Row { tag: new_tag, first: get(1)[0].0, second, ..original }"));
    parse_rust_source(&printed, "roundtrip").unwrap();
}

#[test]
fn callable_types_roundtrip_qualified_traits_and_ordered_bounds() {
    let source = r#"
fn callback_view(value: &u32) -> Box<dyn std::ops::Fn() -> u32 + '_> {
    Box::new(move || *value)
}
fn observe(callback: &(dyn std::ops::Fn() -> u32 + '_)) -> u32 { callback() }
fn factory() -> impl ::core::clone::Clone + std::ops::Fn(u32) -> u32 + 'static {
    |value| value
}
"#;
    let module = parse_rust_source(source, "callable").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("Box<dyn std::ops::Fn() -> u32 + '_>"));
    assert!(printed.contains("&(dyn std::ops::Fn() -> u32 + '_)"));
    assert!(printed.contains("impl ::core::clone::Clone + std::ops::Fn(u32) -> u32 + 'static"));
    parse_rust_source(&printed, "roundtrip").unwrap();
}

#[test]
fn declared_lifetimes_remain_attached_to_their_binders_and_references() {
    let source = r#"
pub fn relay<'short, 'long: 'short, T: Clone>(value: &'long mut T) -> &'short T { value }
pub fn callback() -> impl for<'a> Fn(&'a u32) -> &'a u32 + Clone { |value| value }
"#;
    let module = parse_rust_source(source, "lifetimes").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("relay<'short, 'long: 'short, T>"));
    assert!(printed.contains("(value: &'long mut T) -> &'short T"));
    assert!(printed.contains("for<'a> Fn(&'a u32) -> &'a u32 + Clone"));
    parse_rust_source(&printed, "roundtrip").unwrap();
}

#[test]
fn parameter_reassignment_retains_the_mutable_binding() {
    let source = "pub fn advance(mut value: u32, _: u32) -> u32 { value = value + 1; value }";
    let module = parse_rust_source(source, "parameter_binding").unwrap();
    let Item::Function(function) = &module.items[0] else {
        panic!("expected function");
    };
    assert_eq!(function.params[0].binding_name(), Some("value"));
    assert_eq!(function.params[1].binding_name(), None);
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("advance(mut value: u32, _: u32)"));
    parse_rust_source(&printed, "roundtrip").unwrap();
}

#[test]
fn where_bounds_on_parameters_and_instantiated_types_survive_printing() {
    let source = r#"
        fn describe<T>(value: Vec<T>) -> String
        where
            T: Clone,
            Vec<T>: std::fmt::Debug,
        {
            format!("{value:?}")
        }
    "#;
    let module = parse_rust_source(source, "where_bounds").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("T: Clone"));
    assert!(printed.contains("Vec<T>: std::fmt::Debug"));
    parse_rust_source(&printed, "roundtrip").unwrap();
}

#[test]
fn iterator_opaque_types_keep_the_associated_item_type() {
    let source = r#"
        fn list<T>(values: impl IntoIterator<Item = T>) -> Vec<T> {
            values.into_iter().collect()
        }

        fn iter<'a, T>(values: &'a [T]) -> impl Iterator<Item = &'a T> {
            values.iter()
        }
    "#;
    let module = parse_rust_source(source, "iterator_types").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("impl IntoIterator<Item = T>"));
    assert!(printed.contains("impl Iterator<Item = &'a T>"));
    parse_rust_source(&printed, "roundtrip").unwrap();
}
