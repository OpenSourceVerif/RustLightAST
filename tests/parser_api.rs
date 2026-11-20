use rustlightast::{
    parse_rust_source,
    Expr, GenericArgument, Item, Member, RustCodeGenerator, Statement,
    TraitBoundArgument, TraitBoundArguments, Type, TypeBound, WherePredicate,
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
fn parenthesized_and_bracketed_macro_calls_can_be_block_results() {
    let source = r#"
fn check(value: u32) -> bool {
    matches!(value, 0);
    matches![value, 1]
}
fn check_again(value: u32) -> bool {
    matches!(value, 2)
}
"#;
    let module = parse_rust_source(source, "example").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    let reparsed = parse_rust_source(&printed, "example").unwrap();
    for module in [module, reparsed] {
        for (index, item) in module.items.iter().enumerate() {
            let Item::Function(function) = item else { panic!("expected function"); };
            assert!(matches!(function.body.expr.as_deref(), Some(Expr::Macro(_))));
            assert_eq!(function.body.stmts.len(), usize::from(index == 0));
        }
    }
}

#[test]
fn local_attributes_survive_a_modeled_function_roundtrip() {
    use quote::ToTokens;

    let source = r#"
fn read(input: Option<u32>) -> u32 {
    fn worker(input: Option<u32>) -> u32 {
        #[allow(unused_mut)]
        let mut pending = input;
        #[allow(unused_variables)]
        let Some(value) = pending else { return 0; };
        value
    }
    worker(input)
}
"#;
    let module = parse_rust_source(source, "read").unwrap();
    let Item::Function(read) = &module.items[0] else {
        panic!("expected function");
    };
    let Statement::Item(worker) = &read.body.stmts[0] else {
        panic!("expected a local function statement");
    };
    assert!(matches!(worker.as_ref(), Item::Function(_)));

    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert_eq!(
        syn::parse_file(&printed)
            .unwrap()
            .to_token_stream()
            .to_string(),
        syn::parse_file(source)
            .unwrap()
            .to_token_stream()
            .to_string(),
    );
}

#[test]
fn binary_operations_keep_leading_blocks_in_statements_and_tails() {
    let mut module = parse_rust_source(
        "fn sum() -> u32 { let value = { 1 } + { 2 } + 3; value }",
        "sum",
    )
    .unwrap();
    let Item::Function(function) = &mut module.items[0] else {
        panic!("expected function");
    };
    let Statement::Let(binding) = &function.body.stmts[0] else {
        panic!("expected binding");
    };
    let sum = binding.init.clone().unwrap();
    function.body.stmts.push(Statement::Expr(sum.clone()));
    function.body.expr = Some(Box::new(sum));

    let printed = RustCodeGenerator::new().generate_module_code(&module);
    let parsed = parse_rust_source(&printed, "roundtrip").unwrap();
    let Item::Function(function) = &parsed.items[0] else {
        panic!("expected function");
    };
    let [Statement::Let(binding), Statement::Expr(statement)] = function.body.stmts.as_slice()
    else {
        panic!("expected one binding and one expression statement");
    };
    fn first_operand(expression: &Expr) -> &Expr {
        match expression {
            Expr::BinaryOp(left, _, _) => first_operand(left),
            other => other,
        }
    }
    assert!(matches!(
        first_operand(binding.init.as_ref().unwrap()),
        Expr::Block(_)
    ));
    for expression in [statement, function.body.expr.as_deref().unwrap()] {
        assert!(matches!(
            first_operand(expression),
            Expr::Parenthesized(operand) if matches!(operand.as_ref(), Expr::Block(_))
        ));
    }
}

#[test]
fn array_lengths_round_trip_as_expressions() {
    let module = parse_rust_source(
        "type Buffer = [u8; crate::CAPACITY + 1]; type Hex = [u8; 0x4usize];\n\
         struct Buf<const N: usize>([u8; N]);",
        "arrays",
    )
    .unwrap();
    let Item::TypeAlias(buffer) = &module.items[0] else {
        panic!("expected alias");
    };
    assert!(matches!(
        &buffer.target, Type::Array(_, length) if length.array_length_value().is_none()
    ));
    let Item::TypeAlias(hex) = &module.items[1] else {
        panic!("expected alias");
    };
    assert!(matches!(
        &hex.target, Type::Array(_, length) if length.array_length_value() == Some(4)
    ));
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("crate::CAPACITY + 1"), "{printed}");
    assert!(printed.contains("0x4usize"), "{printed}");
    assert!(printed.contains("[u8; N]"), "{printed}");
    assert_eq!(parse_rust_source(&printed, "arrays").unwrap(), module);
}

#[test]
fn inline_module_attributes_keep_their_targets() {
    let source = r#"
#[cfg(test)] mod tests {
    #![allow(dead_code)]
    use super::*;
    pub fn value() -> u32 { test_value() }
}
"#;
    let module = parse_rust_source(source, "adapter").unwrap();
    let output = RustCodeGenerator::new().generate_module_code(&module);
    let output = syn::parse_file(&output).unwrap();
    let syn::Item::Mod(tests) = &output.items[0] else {
        panic!("expected a test module");
    };
    assert_eq!(tests.attrs.len(), 2);
    assert!(matches!(tests.attrs[0].style, syn::AttrStyle::Outer));
    assert!(tests.attrs[0].path().is_ident("cfg"));
    assert!(matches!(tests.attrs[1].style, syn::AttrStyle::Inner(_)));
    assert!(tests.attrs[1].path().is_ident("allow"));
    let (_, items) = tests.content.as_ref().unwrap();
    let syn::Item::Use(import) = &items[0] else {
        panic!("expected the first declaration to be an import");
    };
    assert!(import.attrs.is_empty());
}

#[test]
fn use_tree_preserves_groups_renames_and_globs() {
    use rustlightast::{Item, UseTree};

    let source = "use ::outer::{self, nested::{Thing as Local, *}};";
    let module = parse_rust_source(source, "imports").unwrap();
    let [Item::Use(import)] = module.items.as_slice() else {
        panic!("expected a use declaration");
    };
    assert!(import.absolute);
    let UseTree::Path { segment, tree } = &import.tree else {
        panic!("expected the outer path");
    };
    assert_eq!(segment, "outer");
    let UseTree::Group(entries) = tree.as_ref() else {
        panic!("expected a group");
    };
    assert!(matches!(&entries[0], UseTree::Name(name) if name == "self"));
    assert!(matches!(&entries[1], UseTree::Path { segment, tree }
        if segment == "nested" && matches!(tree.as_ref(), UseTree::Group(children)
            if matches!(children.as_slice(), [UseTree::Rename { name, alias }, UseTree::Glob]
                if name == "Thing" && alias == "Local"))));
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert_eq!(parse_rust_source(&printed, "imports").unwrap(), module);
}

#[test]
fn let_else_preserves_its_binding_and_failure_branch() {
    let source = r#"
fn min_key(cases: &[(u32, u32)], default: u32) -> u32 {
    let Some(minkey): Option<u32> = cases
        .iter()
        .map(|(key, _)| key)
        .min_by(|left, right| left.cmp(right))
        .cloned()
    else {
        return default;
    };
    minkey
}
"#;
    let module = parse_rust_source(source, "let_else").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("let Some(minkey): Option<u32>"));
    assert!(printed.contains(".cloned() else {"));
    assert!(printed.contains("return default;"));
    parse_rust_source(&printed, "roundtrip").unwrap();
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
    macro_rules! zero { () => { 0 }; }
    let mut sum = zero!();
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
    let compact: String = printed.split_whitespace().collect();
    assert!(compact.contains("macro_rules!zero"), "{printed}");
    assert!(compact.contains("letmutsum=zero!();"), "{printed}");

    for module in [&module, &reparsed] {
        let function = module
            .items
            .iter()
            .find_map(|item| match item {
                Item::Function(function) => Some(function),
                _ => None,
            })
            .unwrap();
        let Some(Statement::Macro(definition)) = function.body.stmts.first() else {
            panic!("expected a modeled local macro definition");
        };
        assert_eq!(definition.name.as_deref(), Some("zero"));
        assert_eq!(definition.invocation.path, ["macro_rules"]);
        let statements: Vec<_> = function
            .body
            .stmts
            .iter()
            .filter_map(|statement| match statement {
                Statement::Macro(statement) if statement.name.is_none() => Some(statement),
                _ => None,
            })
            .collect();
        assert_eq!(
            statements
                .iter()
                .map(|statement| (statement.invocation.delimiter, statement.semicolon))
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
            .all(|statement| statement.invocation.path == ["crate", "add"]));
        assert_eq!(statements[0].attrs.len(), 1);
        assert!(statements[0].attrs[0].contains("cfg"));
        assert!(statements[1].invocation.tokens.written_source().unwrap().contains("(1)"));
    }
}

#[test]
fn macro_expression_retains_its_path_and_input_tokens() {
    use rustlightast::{Expr, MacroDelimiter};

    let source = r##"fn message() { let _value = ::example::message![r#"a "quote""#, nested!(1)]; }"##;
    let module = parse_rust_source(source, "macros").unwrap();
    let Item::Function(function) = &module.items[0] else {
        panic!("expected function");
    };
    let rustlightast::Statement::Let(binding) = &function.body.stmts[0] else {
        panic!("expected let binding");
    };
    let Some(Expr::Macro(mac)) = binding.init.as_ref() else {
        panic!("expected macro expression");
    };
    assert_eq!(mac.invocation.path, ["", "example", "message"]);
    assert_eq!(mac.invocation.delimiter, MacroDelimiter::Bracket);
    let arguments = mac.invocation.expression_arguments().unwrap();
    assert!(matches!(&arguments[0], Expr::Literal(literal)
        if literal.string_value() == Some("a \"quote\"")));
    assert!(matches!(&arguments[1], Expr::Macro(nested) if nested.invocation.path == ["nested"]));
    assert_eq!(
        mac.invocation.nested_macro_paths().unwrap(),
        &[vec!["nested".to_owned()]],
    );
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert_eq!(parse_rust_source(&printed, "macros").unwrap(), module);
}

#[test]
fn modeled_macro_arguments_supply_paths_and_printed_input() {
    use quote::ToTokens;
    use rustlightast::{MacroInput, ModeledMacroInput};

    let mut module = parse_rust_source(
        "fn f() { let _ = vec![x]; let _ = outer!(inner!(x)); }",
        "macros",
    )
    .unwrap();
    let [Item::Function(function)] = module.items.as_mut_slice() else {
        panic!("expected one function");
    };
    let [Statement::Let(vector), Statement::Let(nested)] = function.body.stmts.as_mut_slice() else {
        panic!("expected two macro bindings");
    };
    let nested = nested.init.clone().unwrap();
    let Expr::Macro(vector) = vector.init.as_mut().unwrap() else {
        panic!("expected vector macro");
    };
    vector.invocation.tokens = MacroInput::from_model(
        ModeledMacroInput::Expressions {
            arguments: vec![nested],
            format_captures: Default::default(),
        },
        true,
    )
    .unwrap();
    assert!(vector.invocation.tokens.written_source().is_none());
    assert_eq!(
        vector.invocation.nested_macro_paths().unwrap(),
        &[vec!["outer".to_owned()], vec!["inner".to_owned()]],
    );
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    let expected = "fn f() { let _ = vec![outer!(inner!(x)),]; let _ = outer!(inner!(x)); }";
    let tokens = |source: &str| syn::parse_file(source).unwrap().to_token_stream().to_string();
    assert_eq!(tokens(&printed), tokens(expected));
}

#[test]
fn macro_items_match_identifier_arguments_and_roundtrip() {
    use quote::ToTokens;
    use rustlightast::Visibility;

    let source = r#"
macro_rules! getter {
    ($name:ident, $field:ident) => {
        pub(crate) fn $name(state: Options) -> bool { state.$field }
    };
}

getter!(r#type, enabled);
getter!(r#type, enabled,);
getter!(impl, enabled);
getter!(valid, 42);
macro_rules! changes_trait {
    ($name:ident) => {
        fn $name() { impl Clone for Options {} }
    };
}
"#;
    let module = parse_rust_source(source, "macros").unwrap();
    let item_macro = |index| {
        let Item::Macro(mac) = &module.items[index] else {
            panic!("expected a macro item");
        };
        mac.as_ref()
    };
    let template = item_macro(0).function_template.as_ref().unwrap();
    assert_eq!(item_macro(0).name.as_deref(), Some("getter"));
    assert_eq!(
        template.functions(&item_macro(1).invocation).unwrap().collect::<Vec<_>>(),
        [("r#type", &Visibility::Restricted(vec!["crate".into()]))],
    );
    for index in 2..=4 {
        assert!(template.functions(&item_macro(index).invocation).is_none());
    }
    assert!(item_macro(5).function_template.is_none());

    let printed = RustCodeGenerator::new().generate_module_code(&module);
    let tokens = |source: &str| syn::parse_file(source).unwrap().to_token_stream().to_string();
    assert_eq!(tokens(source), tokens(&printed));
}

#[test]
fn macro_input_exposes_supported_operand_syntax() {
    let source = r#"
thread_local! {
    #[cfg(test)]
    pub(crate) static COUNT: u64 = const { 1 };
}
fn input(value: u64, count: usize) {
    let _ = vec![value; count];
    let _ = matches!(value, 1);
    let _ = matches!(Some(value), Some(found) if found > 0);
    let _ = format!("{value:04} {value} {{ignored}}");
    assert!(value > 0, "{value}");
    let _ = format!("{value:width$}");
}
"#;
    let module = parse_rust_source(source, "macro_inputs").unwrap();
    let [Item::Macro(thread), Item::Function(function)] = module.items.as_slice() else {
        panic!("expected an item macro and a function");
    };
    let [count] = thread.invocation.thread_local_statics().unwrap() else {
        panic!("expected one static declaration");
    };
    assert_eq!(count.name, "COUNT");
    assert_eq!(count.attrs[0].unqualified_name(), Some("cfg"));
    assert!(count.ty.is_some());
    let invocations: Vec<_> = function.body.stmts.iter().map(|statement| {
        match statement {
            Statement::Let(binding) => match binding.init.as_ref() {
                Some(Expr::Macro(mac)) => &mac.invocation,
                _ => panic!("expected a macro expression"),
            },
            Statement::Macro(mac) => &mac.invocation,
            _ => panic!("expected a macro statement"),
        }
    }).collect();
    let (values, count) = invocations[0].vector_input().unwrap();
    assert!(matches!(values, [Expr::Ident(value)] if value == "value"));
    assert!(matches!(count, Some(Expr::Ident(count)) if count == "count"));
    assert!(invocations[1].match_input().is_some_and(|input| input.guard.is_none()));
    assert!(invocations[2].match_input().is_some_and(|input| input.guard.is_some()));
    assert_eq!(invocations[3].format_captures(0).unwrap(), ["value"]);
    assert_eq!(invocations[4].format_captures(1).unwrap(), ["value"]);
    assert!(invocations[5].format_captures(0).is_none());
}

#[test]
fn loop_labels_roundtrip() {
    let source = r#"
fn f() -> u32 {
    'outer: loop {
        'repeat: while false { continue 'repeat; }
        'items: for _ in [true] { break 'items; }
        break 'outer 7;
    }
}
"#;
    let module = parse_rust_source(source, "labels").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    parse_rust_source(&printed, "roundtrip").unwrap();
    for fragment in [
        "'outer: loop",
        "'repeat: while",
        "'items: for",
        "break 'items;",
        "continue 'repeat;",
        "break 'outer 7;",
    ] {
        assert!(printed.contains(fragment), "{printed}");
    }
}

#[test]
fn labeled_block_preserves_break_and_tail_values() {
    let source = r#"
fn choose(early: bool) -> u32 {
    match early {
        value => 'answer: {
            if value { break 'answer 3; }
            7
        },
    }
}
"#;
    let module = parse_rust_source(source, "label").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    parse_rust_source(&printed, "roundtrip").unwrap();
    for fragment in ["'answer: {", "break 'answer 3;", "7\n"] {
        assert!(printed.contains(fragment), "{printed}");
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
fn record_constructor_type_arguments_can_be_rewritten() {
    use rustlightast::{RecordPath, Statement};

    let source = r#"
struct Pending<T> { marker: std::marker::PhantomData<T> }
fn build() {
    let _record = Pending::<std::boxed::Box<u32>> { marker: std::marker::PhantomData };
}
"#;
    let mut module = parse_rust_source(source, "record_type_arguments").unwrap();
    let Item::Function(function) = &mut module.items[1] else {
        panic!("expected function");
    };
    let Statement::Let(local) = &mut function.body.stmts[0] else {
        panic!("expected local binding");
    };
    let Some(Expr::Record {
        path: RecordPath::WithTypeArguments { arguments, .. },
        ..
    }) = &mut local.init
    else {
        panic!("expected record with explicit type arguments");
    };
    let [GenericArgument::Type(Type::Generic(name, _))] = arguments.as_mut_slice() else {
        panic!("expected one generic type argument");
    };
    *name = "std::rc::Rc".into();

    let mut printer = RustCodeGenerator::new();
    let printed = printer.generate_module_code(&module);
    assert!(printed.contains("Pending::<std::rc::Rc<u32>>"));
    parse_rust_source(&printed, "roundtrip").unwrap();

    let unsupported = syn::parse_str("Opaque::<{ const { 3 } }> {}").unwrap();
    assert!(rustlightast::rustlight_parser::convert_expr(&unsupported).is_err());
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
    assert!(printed.contains("relay<'short, 'long: 'short, T: Clone>"));
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
        fn describe<T: Copy, F>(value: Vec<T>, use_item: F) -> String
        where
            T: Clone,
            F: for<'a> Fn(&'a T) -> Vec<T>,
            Vec<T>: std::fmt::Debug + IntoIterator<Item = T>,
        {
            format!("{value:?}")
        }
    "#;
    let module = parse_rust_source(source, "where_bounds").unwrap();
    let [Item::Function(function)] = module.items.as_slice() else {
        panic!("expected one function");
    };
    let item = function.generics.params[0].type_parameter().unwrap();
    assert!(item.bounds[0].is_plain_name("Copy"));
    assert!(function.generics.type_parameter_bounds(item).any(|bound| bound.is_plain_name("Clone")));
    let callback = function.generics.params[1].type_parameter().unwrap();
    let bounds = function.generics.type_parameter_bounds(callback).collect::<Vec<_>>();
    let [TypeBound::Trait(bound)] = bounds.as_slice() else {
        panic!("expected modeled callable bound");
    };
    assert_eq!(bound.higher_ranked[0].name, "'a");
    assert!(matches!(
        &bound.path[0].arguments,
        TraitBoundArguments::Parenthesized { inputs, output: Some(output) }
            if matches!(&inputs[..], [Type::Reference(inner, Some(lifetime), false)]
                if lifetime == "'a" && matches!(inner.as_ref(), Type::Named(name) if name == "T"))
                && matches!(output.as_ref(), Type::Generic(name, _) if name == "Vec")
    ));
    let predicates = function.generics.non_parameter_where_predicates().collect::<Vec<_>>();
    let [WherePredicate::Type { bounds, .. }] = predicates.as_slice()
    else {
        panic!("expected one where predicate");
    };
    assert!(matches!(&bounds[1], TypeBound::Trait(bound)
        if matches!(&bound.path[0].arguments, TraitBoundArguments::Angle(arguments)
            if matches!(&arguments[..], [TraitBoundArgument::AssociatedType { name, value: Type::Named(value), .. }]
                if name == "Item" && value == "T"))));
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("T: Clone"));
    assert!(printed.contains("F: for<'a> Fn(&'a T) -> Vec<T>"));
    assert!(printed.contains("Vec<T>: std::fmt::Debug"));
    assert_eq!(parse_rust_source(&printed, "where_bounds").unwrap(), module);
    assert!(printed.contains("IntoIterator<Item = T>"));
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

#[test]
fn explicit_method_arguments_survive_source_roundtrip() {
    let source = "fn values() -> Vec<u32> { (0..3).collect::<Vec<_>>() }";
    let module = rustlightast::parse_rust_source(source, "methods").unwrap();
    let rustlightast::Item::Function(values) = &module.items[0] else {
        panic!("expected modeled function");
    };
    let Some(rustlightast::Expr::MethodCall(_, method, _)) = values.body.expr.as_deref() else {
        panic!("expected modeled method call");
    };
    assert_eq!(method.name, "collect");
    assert_eq!(
        method.generic_arguments,
        Some(vec![rustlightast::Type::Generic(
            "Vec".into(),
            vec![rustlightast::Type::Infer.into()],
        )
        .into()]),
    );
    let printed = rustlightast::RustCodeGenerator::new().generate_module_code(&module);
    let file = syn::parse_file(&printed).unwrap();
    let syn::Item::Fn(function) = &file.items[0] else {
        panic!("expected function");
    };
    let syn::Stmt::Expr(syn::Expr::MethodCall(call), None) = &function.block.stmts[0] else {
        panic!("expected method call");
    };
    assert_eq!(call.method, "collect");
    assert_eq!(
        quote::ToTokens::to_token_stream(&call.turbofish).to_string(),
        ":: < Vec < _ > >",
    );
}

#[test]
fn async_blocks_keep_capture_mode_and_body() {
    let source = r#"
        fn futures(value: String) {
            let borrowed = async { value.len() };
            drop(borrowed);
            let moved = async move { value.into_bytes() };
            drop(moved);
        }
    "#;
    let module = parse_rust_source(source, "futures").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("let borrowed = async {"));
    assert!(printed.contains("value.len()"));
    assert!(printed.contains("let moved = async move {"));
    assert!(printed.contains("value.into_bytes()"));
    parse_rust_source(&printed, "roundtrip").unwrap();
}

#[test]
fn parsed_attributes_keep_their_tokens_and_scope() {
    use quote::ToTokens;

    let source = r#"
#[derive(Clone,)]
#[repr(C)]
#[derive(Copy)]
#[doc = include_str!("struct.md")]
struct Data(#[cfg(any())] u8, u8);

#[cfg(any())]
type Alias = u8;

#[doc = "a,b"]
const VALUE: u8 = 0;

#[allow(dead_code)]
impl Data {
    #![allow(unused)]
    #[cfg(any())]
    pub const HIDDEN: u8 = 0;
}

#[export_name = "external_f"]
#[deprecated(note = "a,b")]
fn f() {
    #![allow(unused)]
}
"#;
    let module = parse_rust_source(source, "attributes").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    let actual = syn::parse_file(&printed).expect("printed attributes are valid Rust syntax");
    let expected = syn::parse_file(source).unwrap();
    for (actual, expected) in actual.items.iter().zip(&expected.items) {
        let tokens = |item: &syn::Item| item.to_token_stream().to_string();
        if let (syn::Item::Fn(actual), syn::Item::Fn(expected)) = (actual, expected) {
            let attrs = |function: &syn::ItemFn| {
                function
                    .attrs
                    .iter()
                    .map(|attr| attr.to_token_stream().to_string())
                    .collect::<Vec<_>>()
            };
            assert_eq!(attrs(actual), attrs(expected));
        } else {
            assert_eq!(tokens(actual), tokens(expected));
        }
    }
    assert_eq!(actual.items.len(), expected.items.len());
}

#[test]
fn adding_a_derive_keeps_existing_names_in_their_attribute_groups() {
    use quote::ToTokens;
    use rustlightast::DataAttribute;

    let source = "#[derive(Clone)] #[repr(C)] #[derive(::core::fmt::Debug,)] #[allow(dead_code)] struct S;";
    let mut module = parse_rust_source(source, "derive_order").unwrap();
    let Item::Struct(item) = &mut module.items[0] else {
        panic!("expected structure");
    };
    DataAttribute::push_derive(&mut item.attrs, vec!["Copy".to_owned()]);

    let printed = RustCodeGenerator::new().generate_module_code(&module);
    let actual = syn::parse_str::<syn::ItemStruct>(&printed).unwrap();
    let expected = syn::parse_str::<syn::ItemStruct>(
        "#[derive(Clone)] #[repr(C)] #[derive(::core::fmt::Debug, Copy,)] #[allow(dead_code)] struct S;",
    )
    .unwrap();
    assert_eq!(
        actual.to_token_stream().to_string(),
        expected.to_token_stream().to_string(),
    );
}

#[test]
fn trait_defaults_are_editable_without_changing_other_members() {
    use quote::ToTokens;
    use rustlightast::{Literal, TraitMember};

    let source = r#"
    #[allow(dead_code)]
    pub(crate) unsafe trait View<T: Copy, const N: usize = 2>: Send + Sync
    where Self: 'static {
        #![allow(unused_variables)]
        type Item;
        const SIZE: usize = 2;
        fn required(&self, value: T) -> Self::Item;
        fn combine(self: &Self, (left, right): (u32, u32)) -> u32 { left + right }
        #[inline]
        unsafe fn current(&self) -> u32 { 7 }
    }
    unsafe auto trait Marker {}
    trait WithDefault<T = u32> {
        fn ready() -> u32 { 1 }
    }
    "#;
    let mut module = parse_rust_source(source, "traits").unwrap();
    let Item::Trait(view) = &mut module.items[0] else {
        panic!("expected a modeled trait");
    };
    let [
        TraitMember::AssocType(associated_type),
        TraitMember::AssocConst(associated_const),
        TraitMember::RequiredMethod(required),
        TraitMember::DefaultMethod(combine),
        TraitMember::DefaultMethod(default),
    ] = view.members.as_mut_slice()
    else {
        panic!("expected structural trait members");
    };
    assert_eq!(associated_type.name, "Item");
    assert!(associated_type.default.is_none());
    assert_eq!(associated_const.name, "SIZE");
    assert!(associated_const.default.as_ref().unwrap().expression().is_some());
    assert_eq!(required.name, "required");
    assert!(matches!(required.return_type, Type::Path(ref path) if path == &["Self", "Item"]));
    assert!(matches!(combine.params[1].pattern, Some(rustlightast::Pattern::Tuple(_))));
    assert_eq!(default.name, "current");
    default.body.expr = Some(Box::new(Expr::Literal(Literal::Int(19))));

    let Item::Trait(with_default) = &module.items[2] else {
        panic!("expected a modeled type parameter default");
    };
    assert!(matches!(
        with_default.generics.params[0].type_parameter().unwrap().default,
        Some(Type::Named(ref name)) if name == "u32"
    ));
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    let actual = syn::parse_file(&printed).unwrap();
    let expected = syn::parse_file(source).unwrap();
    let (
        [syn::Item::Trait(actual_view), actual_marker, actual_fallback],
        [syn::Item::Trait(expected_view), expected_marker, expected_fallback],
    ) = (actual.items.as_slice(), expected.items.as_slice())
    else {
        panic!("expected both traits in the printed source");
    };
    let tokens = |node: &dyn ToTokens| node.to_token_stream().to_string();
    let header = |item: &syn::ItemTrait| {
        let mut item = item.clone();
        item.items.clear();
        tokens(&item)
    };
    assert_eq!(header(actual_view), header(expected_view));
    assert_eq!(tokens(actual_marker), tokens(expected_marker));
    let (syn::Item::Trait(actual_fallback), syn::Item::Trait(expected_fallback)) =
        (actual_fallback, expected_fallback)
    else {
        panic!("expected the trait with a type parameter default");
    };
    assert_eq!(header(actual_fallback), header(expected_fallback));
    assert_eq!(actual_view.items.len(), expected_view.items.len());
    for (actual, expected) in actual_view.items[..4].iter().zip(&expected_view.items[..4]) {
        assert_eq!(tokens(actual), tokens(expected));
    }
    let syn::TraitItem::Fn(method) = &actual_view.items[4] else {
        panic!("expected the edited default method");
    };
    assert!(method.sig.unsafety.is_some());
    assert_eq!(tokens(&method.attrs[0]), "# [inline]");
    assert_eq!(tokens(method.default.as_ref().unwrap()), "{ 19 }");
}

#[test]
fn models_and_roundtrips_pointer_types_and_type_parameter_defaults() {
    use quote::ToTokens;
    use rustlightast::{FunctionAbi, PointerKind};

    let source = r#"
        type Callback<T = u32> = for<'a> unsafe extern "C" fn(
            data: *const T, reference: &'a mut T, args: ...
        ) -> *mut T;
        type NoResult = fn(u32);
    "#;
    let module = parse_rust_source(source, "pointer_types").unwrap();
    let Item::TypeAlias(alias) = &module.items[0] else {
        panic!("expected a modeled alias");
    };
    let Type::BareFn(function) = &alias.target else {
        panic!("expected a bare function type");
    };
    assert!(function.unsafety);
    assert!(matches!(&function.abi, Some(FunctionAbi::Named(name)) if name == "C"));
    assert_eq!(function.higher_ranked[0].name, "'a");
    assert!(matches!(function.inputs[0].ty, Type::RawPointer(_, PointerKind::Const)));
    assert!(matches!(function.output, Some(Type::RawPointer(_, PointerKind::Mut))));
    assert_eq!(function.variadic.as_ref().unwrap().name.as_deref(), Some("args"));

    let printed = RustCodeGenerator::new().generate_module_code(&module);
    let tokens = |source: &str| syn::parse_file(source).unwrap().to_token_stream().to_string();
    assert_eq!(tokens(&printed), tokens(source));
}

#[test]
fn local_static_declarations_have_structured_types_and_values() {
    let module = parse_rust_source("fn outer() { static VALUE: u32 = 1; }", "local").unwrap();
    let Item::Function(function) = &module.items[0] else {
        panic!("expected outer function");
    };
    let Statement::Item(item) = &function.body.stmts[0] else {
        panic!("expected local declaration");
    };
    let Item::Static(item) = item.as_ref() else {
        panic!("expected a modeled static");
    };
    assert_eq!(item.name, "VALUE");
    assert_eq!(item.ty, Type::Named("u32".to_owned()));
    assert!(matches!(item.value.expression(), Some(Expr::Literal(_))));
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert!(printed.contains("static VALUE"));
    syn::parse_file(&printed).unwrap();
}
