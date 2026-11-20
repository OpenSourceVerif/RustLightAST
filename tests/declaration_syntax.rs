use quote::ToTokens;
use rustlightast::{
    parse_rust_source, ForeignMember, ForeignSafety, Initializer, Item, RustCodeGenerator,
    TraitMember, Type, TypeBound,
};

fn assert_same_tokens(source: &str, printed: &str) {
    let tokens = |source: &str| {
        syn::parse_file(source)
            .unwrap()
            .to_token_stream()
            .to_string()
    };
    assert_eq!(tokens(printed), tokens(source), "{printed}");
}

#[test]
fn extern_members_expose_written_signatures_and_safety() {
    let source = r#"
        #[link(name = "c")]
        unsafe extern "C" {
            #![allow(unused)]
            pub safe fn length(#[allow(unused)] text: *const u8) -> usize;
            pub unsafe fn print(format: *const u8, ...) -> i32;
            pub safe static CODE: i32;
            pub unsafe static mut COUNTER: usize;
            type Handle;
            members!();
        }
        pub static mut POINTER: *const u8 = core::ptr::null();
    "#;
    let module = parse_rust_source(source, "ffi").unwrap();
    let [Item::Foreign(block), Item::Static(pointer)] = module.items.as_slice() else {
        panic!("expected foreign block and static");
    };
    assert!(block.unsafety);
    let [ForeignMember::Function(length), ForeignMember::Function(print), ForeignMember::Static(code), ForeignMember::Static(counter), ForeignMember::Type(handle), ForeignMember::Macro(mac)] =
        block.members.as_slice()
    else {
        panic!("expected structural extern members: {:?}", block.members)
    };
    assert_eq!(length.name, "length");
    assert!(length.safe);
    assert!(matches!(length.params[0].param.ty, Type::RawPointer(..)));
    assert_eq!(length.params[0].attrs[0].unqualified_name(), Some("allow"));
    assert!(print.qualifiers.unsafety);
    assert!(print.variadic.is_some());
    assert_eq!(code.safety, ForeignSafety::Safe);
    assert_eq!(counter.safety, ForeignSafety::Unsafe);
    assert!(counter.mutable);
    assert_eq!(handle.name, "Handle");
    assert_eq!(mac.invocation.path, ["members"]);
    assert!(pointer.mutable);
    assert!(matches!(pointer.value, Initializer::Expr(_)));

    assert_same_tokens(
        source,
        &RustCodeGenerator::new().generate_module_code(&module),
    );
}

#[test]
fn trait_interface_types_and_bounds_are_visited_and_can_be_edited() {
    struct Collect<'a>(Vec<&'a str>);
    impl<'a> rustlightast::visit::Visit<'a> for Collect<'a> {
        fn visit_type(&mut self, ty: &'a Type) {
            if let Type::Named(name) = ty {
                self.0.push(name);
            }
            rustlightast::visit::walk_type(self, ty);
        }
    }
    struct ReplaceInput;
    impl rustlightast::visit_mut::VisitMut for ReplaceInput {
        fn visit_type(&mut self, ty: &mut Type) {
            if let Type::Named(name) = ty {
                if name == "Input" {
                    *name = "NewInput".to_owned();
                }
            }
            rustlightast::visit_mut::walk_type(self, ty);
        }
    }

    let source = r#"
        trait Visitor<'a>: Extend<Parent> + 'a {
            type Entry<'s>: Extend<Bound> where Self: 's;
            const LIMIT: Count;
            fn visit(&'a mut self, input: Input) -> Result where Self: Sized;
            fn default(self: &Self) -> Result { Result::new() }
        }
    "#;
    let mut module = parse_rust_source(source, "visitors").unwrap();
    let Item::Trait(definition) = &module.items[0] else {
        panic!("expected trait")
    };
    assert!(matches!(definition.supertraits[0], TypeBound::Trait(_)));
    let [TraitMember::AssocType(entry), TraitMember::AssocConst(limit), TraitMember::RequiredMethod(method), TraitMember::DefaultMethod(default)] =
        definition.members.as_slice()
    else {
        panic!("expected modeled trait members")
    };
    assert_eq!(entry.name, "Entry");
    assert_eq!(entry.generics.params[0].name(), "'s");
    assert_eq!(limit.name, "LIMIT");
    assert!(method.shorthand_receiver);
    assert_eq!(default.return_type, Type::Named("Result".into()));
    assert_same_tokens(
        source,
        &RustCodeGenerator::new().generate_module_code(&module),
    );

    let mut visitor = Collect(Vec::new());
    rustlightast::visit::Visit::visit_module(&mut visitor, &module);
    assert!(["Parent", "Bound", "Count", "Input", "Result"]
        .iter()
        .all(|name| visitor.0.contains(name)));
    rustlightast::visit_mut::VisitMut::visit_item(&mut ReplaceInput, &mut module.items[0]);
    let output = RustCodeGenerator::new().generate_module_code(&module);
    assert_same_tokens(&source.replace("input: Input", "input: NewInput"), &output);
}
