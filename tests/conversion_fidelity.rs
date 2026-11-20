use rustlightast::rustlight_parser::{parse_rust_source, ParsedSourceFile};
use rustlightast::{
    DataAttribute, Expr, Fields, GenericArgument, GenericParam, Item, Literal, PathType, Pattern,
    RustCodeGenerator, RustModule, Statement, Type, TypeAlias, TypeParameter, Visibility,
};

fn parse_and_print(source: &str) -> (RustModule, String) {
    let module = parse_rust_source(source, "test").unwrap();
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    (module, printed)
}

#[test]
fn function_parameter_patterns_survive_conversion_and_printing() {
    let (module, printed) = parse_and_print(
        "struct Pair(u8, u8); fn combine(Pair(a, mut b): Pair, (c, _): (u8, u8), mut d: u8) -> u8 { a + b + c + d }",
    );
    let Item::Function(function) = &module.items[1] else { panic!("missing function") };
    assert!(matches!(function.params[0].pattern, Some(Pattern::TupleStruct { .. })));
    assert!(matches!(function.params[1].pattern, Some(Pattern::Tuple(_))));
    assert_eq!(function.params[1].binding_name(), None);
    assert_eq!(function.params[2].binding_name(), Some("d"));
    assert!(function.params[2].mutable);
    let (round_trip, _) = parse_and_print(&printed);
    assert_eq!(round_trip, module);
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
        fn explicit_unit() -> () {}
    "#;
    let (module, printed) = parse_and_print(source);
    assert!(
        printed.contains(r#"pub const unsafe extern "C" fn top<const N: usize>"#),
        "{printed}"
    );
    assert!(printed.contains("const fn local"), "{printed}");
    assert!(printed.contains("unsafe impl Send for S"), "{printed}");
    assert!(printed.contains("fn outer() {"), "{printed}");
    assert!(printed.contains("fn explicit_unit() -> () {"), "{printed}");
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
fn conversion_errors_reach_the_file_boundary() {
    let cases = [
        "fn f(#[cfg(any())] x: u8) {}",
        "fn f() { struct S; impl !Send for S {} }",
        "fn f(x: u8) { let _p = &raw const x; }",
        "mod nested { static N: u32 = const { 7 }; }",
        "trait Example { fn f() { const { () } } }",
        "fn f() { Opaque::<{ const { 3 } }> {}; }",
        "fn f() { if let pattern!() = value {} }",
        "fn f() -> impl for<#[allow(unused)] 'a> Fn(&'a u8) { |_| () }",
    ];
    for source in cases {
        assert!(syn::parse_file(source).is_ok(), "{source}");
        assert!(ParsedSourceFile::parse(source).is_err(), "{source}");
    }
}

#[test]
fn parses_rec_get_sample_and_prints_it() {
    let source = r#"
use crate::Int::*;

#[derive(Clone)]
pub enum Num {
One,
Bit0(Box<Num>),
Bit1(Box<Num>),
}

#[derive(Clone)]
pub enum Int {
ZeroInt,
Pos(Num),
Neg(Num),
}

#[derive(Clone)]
pub enum Option {
None,
Some(Int),
Rec(Box<Option>),
}

pub fn get(x0: Option) -> Int {
match x0 {
    Option::Some(x) => x.clone(),
    Option::None => Int::ZeroInt,
    Option::Rec(p0) => {
        let op = *p0;
        get(op.clone())
    },
}
}
"#;
    let (module, printed) = parse_and_print(source);
    let [Item::Use(_), Item::Enum(num), Item::Enum(_), Item::Enum(_), Item::Function(get)] =
        module.items.as_slice()
    else {
        panic!("expected the import, three enums and the modeled function");
    };
    assert!(DataAttribute::has_unqualified_derive(&num.attrs, "Clone"));
    let Fields::Tuple(fields) = &num.variants[1].fields else {
        panic!("expected tuple variant");
    };
    assert!(matches!(&fields[0].ty, Type::Generic(name, args)
        if name == "Box" && matches!(args.as_slice(), [GenericArgument::Type(Type::Named(name))] if name == "Num")));

    let Some(Expr::Match { expr, arms }) = get.body.expr.as_deref() else {
        panic!("expected a modeled match");
    };
    assert!(matches!(expr.as_ref(), Expr::Ident(name) if name == "x0"));
    let [_, _, recursive] = arms.as_slice() else {
        panic!("expected three match arms");
    };
    assert!(
        matches!(&recursive.pattern, Pattern::TupleStruct { path, fields }
        if path.segments == ["Option", "Rec"] && path.arguments.is_empty() && matches!(fields.as_slice(),
            [Pattern::Ident { name, .. }] if name == "p0"))
    );
    let [Statement::Let(binding)] = recursive.body.stmts.as_slice() else {
        panic!("expected a binding for the recursive field");
    };
    assert_eq!(binding.pattern.plain_binding_name(), Some("op"));
    assert!(matches!(&binding.init, Some(Expr::UnaryOp(op, inner))
        if op == "*" && matches!(inner.as_ref(), Expr::Ident(name) if name == "p0")));
    let Some(Expr::Call(callee, args)) = recursive.body.expr.as_deref() else {
        panic!("expected a recursive call");
    };
    assert!(matches!(callee.as_ref(), Expr::Ident(name) if name == "get"));
    assert!(
        matches!(args.as_slice(), [Expr::MethodCall(receiver, method, args)]
        if method.is_plain("clone") && args.is_empty()
            && matches!(receiver.as_ref(), Expr::Ident(name) if name == "op"))
    );

    assert!(printed.contains("enum Num"));
    assert!(printed.contains("pub fn get(x0: Option) -> Int"));
    assert!(printed.contains("Option::Rec(p0) => {"));
    assert!(printed.contains("let op = *p0;"));
    assert!(printed.contains("get(op.clone())"));
}

#[test]
fn preserves_expression_macros_as_opaque_rust() {
    let source = r#"
pub fn unwrap_or_panic(x0: Option<bool>) -> bool {
    match x0 {
        Some(x) => x,
        _ => panic!("non-exhaustive match"),
    }
}
"#;
    let (_, printed) = parse_and_print(source);

    assert!(printed.contains(r#"panic!("non-exhaustive match")"#));
}

#[test]
fn preserves_array_expressions_without_allocating_vecs() {
    let source = r#"
pub fn bytes(x: u8) -> [u8; 3] {
    [1, x, 3]
}
"#;
    let (module, printed) = parse_and_print(source);

    let Item::Function(function) = &module.items[0] else {
        panic!("expected bytes function");
    };
    assert!(matches!(
        function.body.expr.as_deref(),
        Some(Expr::Array(items)) if items.len() == 3
    ));
    assert!(printed.contains("[1, x, 3]"));
    assert!(!printed.contains("vec!"));
    syn::parse_file(&printed).expect("printed array remains valid Rust");
}

#[test]
fn preserves_literal_spelling_and_escaping() {
    let source = r##"
pub fn shift(x: u64) -> u64 {
    x << 1usize
}

pub fn escaped() {
    let _s = "line\n\"quoted\"";
    let _c = '\n';
}

pub fn hexadecimal() -> u16 { 0xff_u16 }
pub fn byte() -> u8 { b'\x41' }
pub fn float() -> f32 { 1.25e+1_f32 }
"##;
    let (module, printed) = parse_and_print(source);

    assert!(printed.contains("1usize"));
    assert!(printed.contains("0xff_u16"));
    assert!(printed.contains(r#""line\n\"quoted\"""#));
    assert!(printed.contains(r"'\n'"));
    let Item::Function(escaped) = &module.items[1] else {
        panic!("expected a modeled function containing escaped literals");
    };
    let rustlightast::Statement::Let(character) = &escaped.body.stmts[1] else {
        panic!("expected a modeled character binding");
    };
    assert_eq!(
        character.init.as_ref(),
        Some(&Expr::Literal(Literal::CharToken { spelling: r"'\n'".into(), value: '\n' })),
    );
    let Item::Function(hexadecimal) = &module.items[2] else {
        panic!("expected a modeled hexadecimal function");
    };
    let Some(Expr::Literal(literal @ Literal::Integer { digits, suffix })) =
        hexadecimal.body.expr.as_deref()
    else {
        panic!("expected a modeled integer literal");
    };
    assert_eq!(digits, "0xff_");
    assert_eq!(suffix.as_deref(), Some("u16"));
    assert_eq!(literal.integer_value(), Some(255));
    let Item::Function(byte) = &module.items[3] else {
        panic!("expected a modeled byte function");
    };
    assert_eq!(
        byte.body.expr.as_deref(),
        Some(&Expr::Literal(Literal::Byte { spelling: r"b'\x41'".into(), value: 65 })),
    );
    let Item::Function(float) = &module.items[4] else {
        panic!("expected a modeled float function");
    };
    assert_eq!(
        float.body.expr.as_deref(),
        Some(&Expr::Literal(Literal::FloatToken { digits: "1.25e+1_".into(), suffix: Some("f32".into()) })),
    );
    assert!(printed.contains(r"b'\x41'"));
    assert!(printed.contains("1.25e+1_f32"));
    syn::parse_file(&printed).expect("printed literals remain valid Rust");
}

#[test]
fn preserves_qualified_self_paths() {
    let source = r#"
fn one_bigint() -> BigInt { <BigInt as One>::one() }
fn qualified<T>() {
    <T as ::Traits::Make<'static, u8, 3>>::value::<u16, { 1 + 1 }>();
}

fn values() -> (usize, usize) {
    (<Model<3>>::VALUE, <Model<'static>::Item>::VALUE)
}
"#;
    let (module, printed) = parse_and_print(source);
    let Item::Function(function) = &module.items[1] else {
        panic!("expected a modeled function");
    };
    let [Statement::Expr(Expr::Call(callee, _))] = function.body.stmts.as_slice() else {
        panic!("expected a call");
    };
    let Expr::Path(member, rustlightast::PathType::Qualified(path)) = callee.as_ref() else {
        panic!("expected a qualified callee");
    };
    assert_eq!(path.trait_path.as_deref().unwrap(), ["", "Traits", "Make"]);
    assert_eq!(path.trait_arguments[0].segment, 2);
    assert_eq!(path.trait_arguments[0].arguments.len(), 3);
    assert_eq!(member, &["value"]);
    assert_eq!(path.associated_arguments[0].segment, 0);
    assert_eq!(path.associated_arguments[0].arguments.len(), 2);
    syn::parse_file(&printed).unwrap();
    let compact: String = printed.split_whitespace().collect();
    for path in [
        "<BigIntasOne>::one()",
        "<Tas::Traits::Make<'static,u8,3>>::value::<u16,{1+1}>()",
        "<Model<3>>::VALUE",
        "<Model<'static>::Item>::VALUE",
    ] {
        assert!(compact.contains(path), "{path}: {printed}");
    }
}

#[test]
fn preserves_type_paths_bounds_and_grouping() {
    let source = r#"
type Assoc<T> = <T as ::Traits::Make<u8>>::Item<'static>;
type Selected<T> = Model<T>::Item;
type Object<'a> = &'a (dyn ::std::fmt::Debug + Send + 'a);
type Callable = dyn Fn(u8);
type Macro = opaque_type!(u8);
fn produces() -> impl Send + Sync { () }
"#;
    let (module, printed) = parse_and_print(source);
    let alias = |index| match &module.items[index] {
        Item::TypeAlias(alias) => &alias.target,
        other => panic!("expected a modeled alias, got {other:?}"),
    };
    assert!(matches!(alias(0), Type::AssociatedPath(path, qualification)
        if path == &["Item"] && qualification.trait_arguments[0].segment == 2));
    assert!(matches!(alias(1), Type::PathWithArguments(path, arguments)
        if path == &["Model", "Item"] && arguments[0].segment == 0));
    assert!(matches!(alias(2), Type::Reference(target, _, false)
        if matches!(target.as_ref(), Type::Parenthesized(inner)
            if matches!(inner.as_ref(), Type::TraitObject { dyn_token: true, bounds } if bounds.len() == 3))));
    assert!(matches!(alias(3), Type::CallableTrait(callable) if !callable.explicit_return_type));
    assert!(matches!(alias(4), Type::Macro(mac) if mac.path == ["opaque_type"]));
    assert!(matches!(&module.items[5], Item::Function(function)
        if matches!(&function.return_type, Type::ImplTraitBounds(bounds) if bounds.len() == 2)));
    let (reparsed, _) = parse_and_print(&printed);
    assert_eq!(module.items, reparsed.items);
}
#[test]
fn roundtrips_lifetime_type_and_const_arguments() {
    let source = r#"
pub struct Sample<'a, T, const N: i32> { value: &'a T }
pub type Fixed<'a> = Sample<'a, u8, { 2 + 3 }>;

impl<'a, T, const N: i32> Sample<'a, T, N> {
    const WIDTH: i32 = N;
}

fn preserve<'a, const N: i32>(sample: Sample<'a, u8, N>) -> Sample<'a, u8, N> {
    sample
}

fn width<T, const N: i32>() -> i32 {
    Sample::<T, { N }>::WIDTH
}

pub fn run(value: &u8) -> (u8, i32, i32) {
    let sample: Fixed<'_> = Sample::<'_, u8, { 2 + 3 }> { value };
    let sample = preserve(sample);
    (*sample.value, width::<u8, -2>(), <Sample<'_, u8, 7>>::WIDTH)
}
"#;
    let (module, printed) = parse_and_print(source);
    let Item::TypeAlias(alias) = &module.items[1] else {
        panic!("expected type alias");
    };
    let Type::Generic(name, arguments) = &alias.target else {
        panic!("expected generic type");
    };
    assert_eq!(name, "Sample");
    assert!(matches!(
        arguments.as_slice(),
        [GenericArgument::Lifetime(lifetime), GenericArgument::Type(Type::Named(ty)), GenericArgument::Const(expression)]
            if lifetime == "'a" && ty == "u8" && matches!(expression.as_ref(), Expr::Block(_))
    ));

    let Item::Function(preserve) = &module.items[3] else {
        panic!("expected preserve function");
    };
    let Type::Generic(_, arguments) = &preserve.return_type else {
        panic!("expected generic return type");
    };
    assert!(matches!(arguments[2].as_type(), Some(Type::Named(name)) if name == "N"));

    let compact: String = printed.split_whitespace().collect();
    for expected in [
        "Sample<'a,u8,{2+3}>",
        "Sample<'a,u8,N>",
        "Sample::<T,{N}>::WIDTH",
        "Sample::<'_,u8,{2+3}>",
        "width::<u8,-2>()",
        "<Sample<'_,u8,7>>::WIDTH",
    ] {
        assert!(compact.contains(expected), "{expected}: {printed}");
    }
    let (_, reprinted) = parse_and_print(&printed);
    assert_eq!(printed, reprinted);
}

#[test]
fn traverses_const_argument_expressions_in_types_and_paths() {
    use rustlightast::visit::{self, Visit};

    struct ConstArguments(Vec<&'static str>);

    impl<'ast> Visit<'ast> for ConstArguments {
        fn visit_generic_argument(&mut self, argument: &'ast GenericArgument) {
            let kind = match argument {
                GenericArgument::Type(_) => "type",
                GenericArgument::Lifetime(_) => "lifetime",
                GenericArgument::Const(_) => "const",
            };
            self.0.push(kind);
            visit::walk_generic_argument(self, argument);
        }
    }

    let source = r#"
fn expressions() {
    let _: Option<Row<'static, { ORIGINAL }>> = None;
    let _ = Row::<{ ORIGINAL }> {};
    let _ = Row::<{ ORIGINAL }>::WIDTH;
    let _ = <Row<{ ORIGINAL }>>::WIDTH;
    consume::<u8, { ORIGINAL }>();
    consume::<Row<'static, { ORIGINAL }>>();
    consume::<Vec<u8>>();
}
"#;
    let mut module = parse_rust_source(source, "visits").unwrap();
    let mut arguments = ConstArguments(Vec::new());
    arguments.visit_module(&module);
    assert_eq!(
        arguments.0,
        [
            "type", "lifetime", "const", "const", "const", "const", "type", "const", "type",
            "lifetime", "const", "type", "type"
        ]
    );

    let Item::Function(function) = &mut module.items[0] else {
        panic!("expected function");
    };
    assert!(matches!(
        function.body.stmts.last(),
        Some(Statement::Expr(Expr::Call(callee, _)))
            if matches!(callee.as_ref(), Expr::Path(path, PathType::NamespaceWithTypeArguments(args))
                if path == &["consume"] && args.len() == 1 && args[0].segment == 0
                    && matches!(&args[0].arguments[..],
                        [GenericArgument::Type(Type::Generic(name, inner))]
                        if name == "Vec" && matches!(&inner[..],
                            [GenericArgument::Type(Type::Named(element))] if element == "u8")))
    ));
    rustlightast::visit_mut::visit_block(&mut function.body, &mut |expr| {
        assert!(!matches!(expr, Expr::Ident(name) if name == "ORIGINAL"));
        true
    });
    let Statement::Let(local) = &mut function.body.stmts[0] else {
        panic!("expected local with type annotation");
    };
    let Some(Type::Generic(_, outer)) = &mut local.ty else {
        panic!("expected outer generic type");
    };
    let [GenericArgument::Type(Type::Generic(_, inner))] = outer.as_mut_slice() else {
        panic!("expected inner generic type");
    };
    let [GenericArgument::Lifetime(_), GenericArgument::Const(expression)] = inner.as_mut_slice()
    else {
        panic!("expected const argument");
    };
    rustlightast::visit_mut::visit_expr(expression, &mut |expr| {
        if let Expr::Ident(name) = expr {
            assert_eq!(name, "ORIGINAL");
            *name = "REPLACEMENT".into();
        }
        true
    });
    let printed = RustCodeGenerator::new().generate_module_code(&module);
    assert_eq!(printed.matches("REPLACEMENT").count(), 1);
    parse_rust_source(&printed, "roundtrip").unwrap();
}

#[test]
fn preserves_required_trait_methods_and_renamed_uses() {
    let source = r#"
use num_traits::sign::Signed as _;

pub trait Zero {
    fn zero() -> Self where Self: Sized;
}
"#;
    let (_, printed) = parse_and_print(source);

    assert!(printed.contains("use num_traits::sign::Signed as _;"));
    assert!(printed.contains("pub trait Zero"));
    assert!(printed.contains("fn zero"));
    assert!(printed.contains("fn zero() -> Self where Self: Sized;"));
}

#[test]
fn parses_callable_trait_types() {
    let source = r#"
use std::rc::Rc;

pub fn apply<A, B>(f: Rc<dyn Fn(A) -> B>, x: A) -> B {
    (*f)(x)
}

pub fn call_impl<A, B>(f: impl Fn(A) -> B, x: A) -> B {
    f(x)
}
"#;
    let (module, printed) = parse_and_print(source);

    match &module.items[1] {
        Item::Function(def) => match &def.params[0].ty {
            Type::Generic(name, params) => {
                assert_eq!(name, "Rc");
                assert!(matches!(params[0].as_type(), Some(Type::CallableTrait(callable))
                    if callable.trait_path == ["Fn"]
                        && callable.args.len() == 1
                        && matches!(callable.return_type.as_ref(), Type::Named(name) if name == "B")));
            }
            other => panic!("unexpected callable parameter type: {other:?}"),
        },
        other => panic!("unexpected item: {other:?}"),
    }

    assert!(printed.contains("Rc<dyn Fn(A) -> B>"));
    assert!(printed.contains("impl Fn(A) -> B"));
}

#[test]
fn parses_and_prints_captured_closure_cast() {
    let source = r"
use std::rc::Rc;

pub fn make_pair(y: Int) -> (Rc<dyn Fn(Int) -> Int>, Int) {
    ((({
        let y_cap = y.clone();
        Rc::new(move |x: Int| {
            plus_int(x.clone(), y_cap.clone())
        })
    }) as Rc<dyn Fn(Int) -> Int>), y.clone())
}
";
    let (module, printed) = parse_and_print(source);

    let Item::Function(function) = &module.items[1] else {
        panic!("expected make_pair function");
    };
    let Expr::Tuple(items) = function.body.expr.as_deref().expect("function tail") else {
        panic!("expected tuple tail");
    };
    let Expr::Parenthesized(cast) = &items[0] else {
        panic!("expected parenthesized cast");
    };
    let Expr::Cast(_, Type::Generic(name, params)) = cast.as_ref() else {
        panic!("expected structured cast");
    };
    assert_eq!(name, "Rc");
    assert!(matches!(params[0].as_type(), Some(Type::CallableTrait(callable))
        if callable.trait_path == ["Fn"]
            && callable.args.len() == 1
            && matches!(callable.return_type.as_ref(), Type::Named(name) if name == "Int")));

    assert!(printed.contains("as Rc<dyn Fn(Int) -> Int>"));
    syn::parse_file(&printed).expect("printed cast remains valid Rust");
}

#[test]
fn preserves_explicit_closure_return_types() {
    let source = r#"
use std::rc::Rc;

pub fn partial() -> Rc<dyn Fn(Int) -> Pred<Unit>> {
    Rc::new(move |x: Int| -> Pred<Unit> {
        panic!("partial")
    })
}
"#;
    let (_, printed) = parse_and_print(source);

    assert!(printed.contains("move |x: Int| -> Pred<Unit>"));
    syn::parse_file(&printed).expect("printed typed closure remains valid Rust");
}

#[test]
fn prints_generic_type_alias_parameters_and_bounds() {
    let item = Item::TypeAlias(TypeAlias {
        attrs: vec![],
        name: "Callback".into(),
        target: Type::Generic("Rc".into(), vec![Type::Named("T".into()).into()]),
        generics: vec![GenericParam::Type(TypeParameter {
            name: "T".into(),
            bounds: vec!["Clone".into()],
            default: None,
        })]
        .into(),
        vis: Visibility::Public,
        docs: vec![],
    });
    let printed = RustCodeGenerator::new().generate_item_code(&item);
    assert_eq!(printed.trim(), "pub type Callback<T: Clone> = Rc<T>;");
}
