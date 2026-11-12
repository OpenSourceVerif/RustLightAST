use rustlightast::rustlight_parser::{convert_file_preserving_items, parse_rust_source};
use rustlightast::{
    DataAttribute, Expr, Fields, GenericArgument, GenericParam, Item, Pattern, RustCodeGenerator,
    RustModule, Statement, Type, TypeAlias, TypeParameter, Visibility,
};

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
    assert!(DataAttribute::derive_names(&num.attrs).eq(["Clone"]));
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
        if path == &["Option", "Rec"] && matches!(fields.as_slice(),
            [Pattern::Ident { name, .. }] if name == "p0"))
    );
    let [Statement::Let(binding)] = recursive.body.stmts.as_slice() else {
        panic!("expected a binding for the recursive field");
    };
    assert_eq!(binding.name, "op");
    assert!(matches!(&binding.init, Some(Expr::UnaryOp(op, inner))
        if op == "*" && matches!(inner.as_ref(), Expr::Ident(name) if name == "p0")));
    let Some(Expr::Call(callee, args)) = recursive.body.expr.as_deref() else {
        panic!("expected a recursive call");
    };
    assert!(matches!(callee.as_ref(), Expr::Ident(name) if name == "get"));
    assert!(
        matches!(args.as_slice(), [Expr::MethodCall(receiver, method, args)]
        if method == "clone" && args.is_empty()
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
"##;
    let (_, printed) = parse_and_print(source);

    assert!(printed.contains("1usize"));
    assert!(printed.contains(r#""line\n\"quoted\"""#));
    assert!(printed.contains(r"'\n'"));
    syn::parse_file(&printed).expect("printed literals remain valid Rust");
}

#[test]
fn preserves_modeled_and_unmodeled_qualified_self_paths() {
    let source = r#"
fn one_bigint() -> BigInt { <BigInt as One>::one() }
fn values() -> (usize, usize) {
    (<Model<3>>::VALUE, <Model<'static>::Item>::VALUE)
}
"#;
    let (_, printed) = parse_and_print(source);
    syn::parse_file(&printed).unwrap();
    let compact: String = printed.split_whitespace().collect();
    for path in [
        "<BigIntasOne>::one()",
        "<Model<3>>::VALUE",
        "<Model<'static>::Item>::VALUE",
    ] {
        assert!(compact.contains(path), "{path}: {printed}");
    }
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
            "lifetime", "const"
        ]
    );

    let Item::Function(function) = &mut module.items[0] else {
        panic!("expected function");
    };
    assert!(matches!(
        function.body.stmts.last(),
        Some(Statement::Expr(Expr::Call(callee, _)))
            if matches!(callee.as_ref(), Expr::Ident(name)
                if name.split_whitespace().collect::<String>() == "consume::<Vec<u8>>")
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
    assert!(printed.contains("Self : Sized"));
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
