use rustlightast::rustlight_parser::parse_rust_source;
use rustlightast::{RustCodeGenerator, RustModule};

fn parse_and_print(source: &str) -> (RustModule, String) {
    let module = parse_rust_source(source, "test").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    (module, printed)
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
