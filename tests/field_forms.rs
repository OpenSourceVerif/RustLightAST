use rustlightast::{parse_rust_source, RustCodeGenerator};
use syn::{Fields, Item};

#[test]
fn field_forms_and_variant_attributes_survive_roundtrip() {
    let source = r#"
        struct Unit;
        struct EmptyTuple();
        struct EmptyRecord {}

        #[derive(Default)]
        enum Event {
            #[default]
            Idle,
            Allocated { round: Option<i64>, allocation: u32 },
            EmptyTuple(),
            EmptyRecord {},
        }
    "#;
    let module = parse_rust_source(source, "field_forms").unwrap();
    let output = RustCodeGenerator::new().generate_module_code(&module);
    let file = syn::parse_file(&output).unwrap();

    let [Item::Struct(unit), Item::Struct(tuple), Item::Struct(record), Item::Enum(event)] =
        file.items.as_slice()
    else {
        panic!("unexpected declarations: {output}");
    };
    assert!(matches!(unit.fields, Fields::Unit));
    assert!(matches!(&tuple.fields, Fields::Unnamed(fields) if fields.unnamed.is_empty()));
    assert!(matches!(&record.fields, Fields::Named(fields) if fields.named.is_empty()));

    let variants = &event.variants;
    assert!(matches!(variants[0].fields, Fields::Unit));
    assert!(variants[0].attrs[0].path().is_ident("default"));
    let Fields::Named(allocated) = &variants[1].fields else {
        panic!("unexpected allocation variant: {output}");
    };
    assert_eq!(
        allocated
            .named
            .iter()
            .map(|field| field.ident.as_ref().unwrap().to_string())
            .collect::<Vec<_>>(),
        ["round", "allocation"]
    );
    assert!(matches!(&variants[2].fields, Fields::Unnamed(fields) if fields.unnamed.is_empty()));
    assert!(matches!(&variants[3].fields, Fields::Named(fields) if fields.named.is_empty()));
}
