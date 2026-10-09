use rustlightast::{
    parse_rust_source, parse_rust_type_facts,
    rustlight_parser::{first_explicit_type_argument, TYPE_FACT_ONLY_DOC},
    Expr, Item, ParseError, RustCodeGenerator, Type,
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
    let source = "pub fn bytes() -> [u8; 4] { [0u8; 4] }";
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
fn parser_keeps_its_existing_control_flow_boundary() {
    for source in [
        "fn f() { while false {} }",
        "fn f() { for x in [true] {} }",
        "fn f() { return; }",
    ] {
        assert!(
            parse_rust_source(source, "unsupported").is_err(),
            "{source}"
        );
    }
}
