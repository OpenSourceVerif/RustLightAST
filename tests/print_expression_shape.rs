use quote::ToTokens;
use rustlightast::{
    Block, Expr, Literal, Member, Pattern, RangeEnd, RecordField, RecordPath, RustCodeGenerator,
    Type,
};

fn name(name: &str) -> Box<Expr> {
    Box::new(Expr::Ident(name.into()))
}

fn binary(left: Box<Expr>, op: &str, right: Box<Expr>) -> Box<Expr> {
    Box::new(Expr::BinaryOp(left, op.into(), right))
}

fn range(start: Box<Expr>, end: Box<Expr>) -> Box<Expr> {
    Box::new(Expr::Range {
        start: Some(start),
        end: RangeEnd::Exclusive(Some(end)),
    })
}

fn unary(op: &str, operand: Box<Expr>) -> Box<Expr> {
    Box::new(Expr::UnaryOp(op.into(), operand))
}

fn check_shape(expr: &Expr, expected: &str) {
    let source = RustCodeGenerator::new().generate_expression_code(expr);
    let parse = |source: &str| {
        syn::parse_str::<syn::Expr>(source)
            .unwrap_or_else(|error| panic!("{source:?}: {error}"))
            .into_token_stream()
            .to_string()
    };
    assert_eq!(parse(&source), parse(expected), "{source}");
}

#[test]
fn preserves_operator_grouping_without_explicit_parentheses() {
    check_shape(
        &binary(binary(name("a"), "+", name("b")), "*", name("c")),
        "(a + b) * c",
    );
    check_shape(
        &binary(name("a"), "-", binary(name("b"), "-", name("c"))),
        "a - (b - c)",
    );
    check_shape(
        &binary(binary(name("a"), "<", name("b")), "==", name("c")),
        "(a < b) == c",
    );
    check_shape(
        &binary(binary(name("a"), "+", name("b")), "+", name("c")),
        "a + b + c",
    );
    check_shape(
        &Expr::Assign(name("a"), binary(name("b"), "+=", name("c"))),
        "a = b += c",
    );
    check_shape(&range(range(name("a"), name("b")), name("c")), "(a..b)..c");
    check_shape(&unary("-", binary(name("a"), "+", name("b"))), "-(a + b)");
    check_shape(
        &Expr::Cast(binary(name("a"), "+", name("b")), Type::Named("i64".into())),
        "((a + b) as i64)",
    );
}

#[test]
fn preserves_postfix_targets() {
    check_shape(&Expr::Call(unary("*", name("f")), vec![]), "(*f)()");
    check_shape(
        &Expr::Call(
            Box::new(Expr::Field {
                base: name("obj"),
                member: Member::Named("callback".into()),
            }),
            vec![],
        ),
        "(obj.callback)()",
    );
    check_shape(
        &Expr::Index(binary(name("a"), "+", name("b")), name("i")),
        "(a + b)[i]",
    );
    check_shape(
        &Expr::MethodCall(unary("*", name("value")), "get".into(), vec![]),
        "(*value).get()",
    );
    check_shape(&Expr::Await(unary("*", name("future"))), "(*future).await");
    check_shape(
        &Expr::MethodCall(
            Box::new(Expr::Literal(Literal::Int(-1))),
            "abs".into(),
            vec![],
        ),
        "(-1).abs()",
    );
}

#[test]
fn preserves_a_leading_block_used_as_a_call_target() {
    let call = Expr::Call(
        Box::new(Expr::Block(Block {
            stmts: vec![],
            expr: Some(name("f")),
        })),
        vec![],
    );
    let render_tail = |expr| {
        RustCodeGenerator::new().generate_block_code(&Block {
            stmts: vec![],
            expr: Some(Box::new(expr)),
        })
    };

    let source = render_tail(call.clone());
    let parsed: syn::Block = syn::parse_str(&source).unwrap();
    assert!(matches!(
        parsed.stmts.as_slice(),
        [syn::Stmt::Expr(syn::Expr::Call(_), None)]
    ));

    let source = render_tail(Expr::Field {
        base: Box::new(call),
        member: Member::Named("n".into()),
    });
    let parsed: syn::Block = syn::parse_str(&source).unwrap();
    assert!(matches!(
        parsed.stmts.as_slice(),
        [syn::Stmt::Expr(syn::Expr::Field(field), None)]
            if matches!(*field.base, syn::Expr::Call(_))
    ));
}

#[test]
fn groups_struct_literals_exposed_in_control_flow_heads() {
    let record = || {
        Box::new(Expr::Record {
            path: RecordPath::Segments(vec!["S".into()]),
            fields: vec![RecordField {
                member: Member::Named("n".into()),
                value: Expr::Literal(Literal::Int(1)),
                shorthand: false,
            }],
            rest: None,
        })
    };
    let empty = || Block {
        stmts: vec![],
        expr: None,
    };
    let field = Box::new(Expr::Field {
        base: record(),
        member: Member::Named("n".into()),
    });
    let condition = binary(name("expected"), "==", field);
    check_shape(
        &Expr::If {
            condition: condition.clone(),
            then_branch: empty(),
            else_branch: None,
        },
        "if expected == (S { n: 1 }).n {}",
    );
    check_shape(
        &Expr::While {
            label: None,
            condition,
            body: empty(),
        },
        "while expected == (S { n: 1 }).n {}",
    );
    check_shape(
        &Expr::For {
            label: None,
            pattern: Pattern::Wildcard,
            iter: Box::new(Expr::MethodCall(record(), "iter".into(), vec![])),
            body: empty(),
        },
        "for _ in (S { n: 1 }).iter() {}",
    );
    check_shape(
        &Expr::IfLet {
            pattern: rustlightast::parse_rust_pattern("S { n: 1 }").unwrap(),
            value: record(),
            then_branch: empty(),
            else_branch: None,
        },
        "if let S { n: 1 } = (S { n: 1 }) {}",
    );
    check_shape(
        &Expr::Match {
            expr: record(),
            arms: vec![],
        },
        "match (S { n: 1 }) {}",
    );
    check_shape(
        &Expr::If {
            condition: Box::new(Expr::Call(name("ok"), vec![*record()])),
            then_branch: empty(),
            else_branch: None,
        },
        "if ok(S { n: 1 }) {}",
    );
}

#[test]
fn keeps_lazy_boolean_operators_inside_let_scrutinees() {
    check_shape(
        &Expr::IfLet {
            pattern: Pattern::Literal(Literal::Bool(false)),
            value: binary(name("a"), "&&", name("b")),
            then_branch: Block {
                stmts: vec![],
                expr: None,
            },
            else_branch: None,
        },
        "if let false = (a && b) {}",
    );
}

#[test]
fn preserves_async_closure_and_if_let_patterns() {
    use rustlightast::rustlight_parser::convert_expr;

    let source = "async move || -> u32 { if let Some(n) = value { n } else { 0 } }";
    let parsed = syn::parse_str::<syn::Expr>(source).unwrap();
    let modeled = convert_expr(&parsed).unwrap();
    let Expr::AsyncClosure {
        body,
        move_capture: true,
        return_type: Some(_),
        ..
    } = &modeled
    else {
        panic!("expected a typed move async closure");
    };
    assert!(matches!(
        body.as_ref(),
        Expr::Block(block) if matches!(
            block.expr.as_deref(),
            Some(Expr::IfLet { pattern: Pattern::TupleStruct { .. }, .. })
        )
    ));
    check_shape(&modeled, source);

    let opaque = "if let pattern!() = value {}";
    let modeled = convert_expr(&syn::parse_str(opaque).unwrap()).unwrap();
    assert!(matches!(
        &modeled,
        Expr::IfLet {
            pattern: Pattern::Opaque(_),
            ..
        }
    ));
    check_shape(&modeled, opaque);
}
