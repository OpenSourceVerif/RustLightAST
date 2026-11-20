use rustlightast::{
    rustlight_parser::{
        model_origins::{
            ExpressionOrigin, FunctionOrigin, MacroOrigin, ModelOriginVisitor, TypeOrigin,
        },
        ParsedSourceFile,
    },
    Expr, FunctionDef, MacroInvocation, Type,
};

#[test]
fn occurrences_nest_in_local_functions_and_macro_inputs() {
    let text = "\u{feff}#!/usr/bin/env rustx\n\
        fn parent() {\n\
            let input: Option<Box<u32>> = None;\n\
            fn local() {\n\
                let _ = café();\n\
                let _ = matches!(input, Some(value) if use_value::<Box<u32>>(value));\n\
            }\n\
        }\n";
    let parsed = ParsedSourceFile::parse(text).unwrap();

    #[derive(Default)]
    struct Origins {
        functions: Vec<(&'static str, FunctionOrigin)>,
        calls: Vec<ExpressionOrigin>,
        boxes: Vec<TypeOrigin>,
        macro_inputs: Vec<MacroOrigin>,
    }
    impl<'model> ModelOriginVisitor<'model> for Origins {
        fn function(&mut self, model: &'model FunctionDef, origin: FunctionOrigin) {
            let name = match model.name.as_str() {
                "parent" => "parent",
                "local" => "local",
                other => panic!("unexpected function {other}"),
            };
            self.functions.push((name, origin));
        }
        fn expression(&mut self, model: &'model Expr, origin: ExpressionOrigin) {
            if matches!(model, Expr::Call(..)) {
                self.calls.push(origin);
            }
        }
        fn ty(&mut self, model: &'model Type, origin: TypeOrigin) {
            if matches!(model, Type::Generic(name, _) if name == "Box") {
                self.boxes.push(origin);
            }
        }
        fn macro_invocation(&mut self, _: &'model MacroInvocation, origin: MacroOrigin) {
            self.macro_inputs.push(origin);
        }
        fn unmatched(&mut self) {
            panic!("origin events must cover the original model");
        }
    }

    let mut origins = Origins::default();
    parsed.visit_original_origins(&mut origins);

    let [("parent", parent), ("local", local)] = origins.functions.as_slice() else {
        panic!("expected parent and local functions");
    };
    let within = |outer: &rustlightast::rustlight_parser::model_origins::ModelNodeOrigin,
                  inner: &rustlightast::rustlight_parser::model_origins::ModelNodeOrigin| {
        outer.source_address < inner.source_address && inner.subtree_end <= outer.subtree_end
    };
    assert_eq!(parent.node.snapshot, parsed.identity());
    assert!(within(&parent.node, &local.node));
    let [first, second] = origins.calls.as_slice() else {
        panic!("expected one direct call and one in a macro")
    };
    let [mac] = origins.macro_inputs.as_slice() else {
        panic!("expected the macro invocation")
    };
    assert!(within(&local.node, &first.node));
    assert!(within(&mac.node, &second.node));
    let [first_ty, second_ty] = origins.boxes.as_slice() else {
        panic!("expected two independent uses of Box")
    };
    assert!(!within(&local.node, &first_ty.node));
    assert!(within(&second.node, &second_ty.node));
}

#[test]
fn source_occurrences_stay_distinct_in_shorthand_fields_and_macros() {
    use rustlightast::rustlight_parser::model_origins::ModelNodeOrigin;
    use rustlightast::Pattern;

    let source = r#"
        fn update(point: Point, name: i32) {
            let Point { name, .. } = point;
            let _ = Point { name, other: name };
            let _ = format!("{}", name);
            #[allow(unused)]
            emit!(name);
        }
    "#;
    let parsed = ParsedSourceFile::parse(source).unwrap();

    #[derive(Default)]
    struct Origins {
        reads: Vec<ModelNodeOrigin>,
        binding: Option<ModelNodeOrigin>,
        macros: Vec<MacroOrigin>,
    }

    impl ModelOriginVisitor<'_> for Origins {
        fn unmatched(&mut self) {
            panic!("origin events must cover the original model");
        }

        fn expression(&mut self, expression: &Expr, origin: ExpressionOrigin) {
            if matches!(expression, Expr::Ident(name) if name == "name") {
                self.reads.push(origin.node);
            }
        }

        fn pattern(&mut self, pattern: &Pattern, origin: ModelNodeOrigin) {
            if pattern.binding_name() == Some("name") {
                self.binding.get_or_insert(origin);
            }
        }

        fn macro_invocation(&mut self, _: &MacroInvocation, origin: MacroOrigin) {
            self.macros.push(origin);
        }
    }

    let mut origins = Origins::default();
    parsed.visit_original_origins(&mut origins);
    let binding = origins.binding.unwrap();
    let reads: Vec<_> = origins.reads.iter().map(|origin| origin.source_address).collect();
    assert_eq!(reads.len(), 4);
    assert!(reads.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(binding.subtree_end <= reads[0]);
    let [format, emit] = origins.macros.as_slice() else {
        panic!("expected two macro invocations");
    };
    assert!((format.node.source_address..format.node.subtree_end).contains(&reads[2]));
    assert!((emit.node.source_address..emit.node.subtree_end).contains(&reads[3]));
    assert!(emit.has_attributes);
}

#[test]
fn exact_clone_transfers_occurrence_ids_before_editing() {
    use rustlightast::Item;

    #[derive(Default)]
    struct Calls(Vec<(usize, usize)>);

    impl ModelOriginVisitor<'_> for Calls {
        fn unmatched(&mut self) {
            panic!("origin events must cover the original model");
        }

        fn expression(&mut self, expression: &Expr, origin: ExpressionOrigin) {
            if matches!(expression, Expr::Call(..)) {
                self.0.push((
                    std::ptr::from_ref(expression).addr(),
                    origin.node.source_address,
                ));
            }
        }
    }

    let parsed = ParsedSourceFile::parse("fn build() { Box::new(1) }").unwrap();
    let mut original = Calls::default();
    parsed.visit_original_origins(&mut original);

    let mut cloned = None;
    let mut copied = Calls::default();
    parsed.clone_with_origins(&mut cloned, &mut copied);

    let ([before], [after]) = (original.0.as_slice(), copied.0.as_slice()) else {
        panic!("expected a single call in each model");
    };
    assert_ne!(before.0, after.0);
    assert_eq!(before.1, after.1);
    let [Item::Function(function)] = &mut cloned.as_mut().unwrap().items[..] else {
        panic!("cloned function missing");
    };
    let expression = function.body.expr.as_deref_mut().unwrap();
    assert_eq!(std::ptr::from_ref(expression).addr(), after.0);
    *expression = Expr::Tuple(Vec::new());

    let mut original_after_edit = Calls::default();
    parsed.visit_original_origins(&mut original_after_edit);
    assert_eq!(original.0, original_after_edit.0);
}
