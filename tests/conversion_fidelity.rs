use rustlightast::rustlight_parser::{convert_file_preserving_items, parse_rust_source};
use rustlightast::{Item, RustCodeGenerator, RustModule, Statement};

fn parse_and_print(source: &str) -> (RustModule, String) {
    let module = parse_rust_source(source, "test").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    (module, printed)
}

#[test]
fn function_qualifiers_and_local_generics_survive_conversion() {
    let source = r#"
        pub const unsafe extern "C" fn top<const N: usize>() -> usize { N }
        struct S;
        unsafe impl Send for S {}
        fn outer() {
            const fn local() -> u8 { 3 }
            fn collect() -> Vec<u8> { [1u8].into_iter().collect::<Vec<_>>() }
        }
    "#;
    let (module, printed) = parse_and_print(source);
    assert!(
        printed.contains(r#"pub const unsafe extern "C" fn top<const N: usize>"#),
        "{printed}"
    );
    assert!(printed.contains("const fn local"), "{printed}");
    assert!(printed.contains("unsafe impl Send for S"), "{printed}");
    let Item::Function(outer) = &module.items[3] else {
        panic!("outer function is opaque");
    };
    assert!(outer.body.stmts.iter().all(|statement| matches!(
        statement,
        Statement::Item(item) if matches!(item.as_ref(), Item::Function(_))
    )));
    assert_eq!(outer.body.stmts.len(), 2);
    syn::parse_file(&printed).unwrap();
}

#[test]
fn explicit_enum_discriminants_survive_printing() {
    let source = "#[repr(u8)] enum Code { First = 3, Second = 1 << 4 }";
    let (_, printed) = parse_and_print(source);
    assert!(
        printed.contains("First = 3,") && printed.contains("Second = 1 << 4,"),
        "{printed}"
    );
    syn::parse_file(&printed).unwrap();
}

#[test]
fn unsupported_substructure_never_disappears_from_preserved_source() {
    let cases = [
        ("struct Buf<const N: usize>([u8; N]);", "const N"),
        ("fn f(#[cfg(any())] x: u8) {}", "#[cfg"),
        ("fn f() { #[cfg(any())] missing(); }", "#[cfg"),
        ("fn f(x: u8) { let _p = &raw const x; }", "&raw const"),
        (
            "fn f() { match true { #[cfg(any())] false => (), _ => () } }",
            "#[cfg",
        ),
        ("struct S; impl !Send for S {}", "impl !Send"),
    ];
    for (source, expected) in cases {
        assert!(parse_rust_source(source, "test").is_err(), "{source}");
        let original = syn::parse_file(source).unwrap();
        let module = convert_file_preserving_items(&original, "test".into());
        let printed = RustCodeGenerator::new().generate_module_code(&module);
        assert!(
            printed
                .split_whitespace()
                .collect::<String>()
                .contains(&expected.split_whitespace().collect::<String>()),
            "{source}\n{printed}"
        );
        syn::parse_file(&printed).unwrap();
    }
}
