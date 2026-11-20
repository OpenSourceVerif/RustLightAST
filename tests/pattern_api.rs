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

#[test]
fn constructor_pattern_arguments_can_be_specialized() {
    use rustlightast::visit_mut::VisitMut;
    use rustlightast::Type;

    struct Specialize;
    impl VisitMut for Specialize {
        fn visit_type(&mut self, ty: &mut Type) {
            if matches!(ty, Type::Named(name) if name == "T") {
                *ty = Type::Named("u64".into());
            } else {
                rustlightast::visit_mut::walk_type(self, ty);
            }
        }
    }

    let mut printer = RustCodeGenerator::new();
    for source in [
        "::pkg::List::<T>::Nil",
        "crate::List::<T>::Cons(value, _)",
        "crate::Tree::<T>::Node { value, .. }",
    ] {
        let mut pattern = parse_rust_pattern(source).unwrap();
        Specialize.visit_pattern(&mut pattern);
        let printed = printer.generate_pattern_code(&pattern);
        assert_eq!(printed, source.replace("::<T>", "::<u64>"));
        assert_eq!(parse_rust_pattern(&printed).unwrap(), pattern);
    }
}
