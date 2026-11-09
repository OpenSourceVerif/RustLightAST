use rustlightast::{parse_rust_pattern, Pattern, RustCodeGenerator};

#[test]
fn a_field_binding_can_be_renamed_without_renaming_the_matched_field() {
    let mut pattern = parse_rust_pattern(
        "crate::Node { ref mut left, right: Some((head,)), children: [first, ..], .. }",
    )
    .unwrap();
    let Pattern::Record { fields, .. } = &mut pattern else {
        panic!("expected a record pattern");
    };
    let Pattern::Ident { name, .. } = &mut fields[0].pattern else {
        panic!("expected a binding in the shorthand field");
    };
    *name = "selected".into();

    let printed = RustCodeGenerator::new().generate_pattern_code(&pattern);
    assert_eq!(
        printed,
        "crate::Node { left: ref mut selected, right: Some((head,)), children: [first, ..], .. }",
    );
    parse_rust_pattern(&printed).unwrap();
}

#[test]
fn nested_alternation_and_type_ascription_survive_the_pattern_boundary() {
    let mut printer = RustCodeGenerator::new();
    for source in [
        "&mut crate::Node::Leaf(whole @ (1 | 2), _, ..)",
        "(left, _): (u8, u8)",
        "value: ::std::rc::Rc<::std::string::String>",
    ] {
        let pattern = parse_rust_pattern(source).unwrap();
        assert_eq!(printer.generate_pattern_code(&pattern), source);
    }
}
