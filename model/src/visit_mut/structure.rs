//! Mutable traversal of modeled syntax. Opaque Rust spellings are left intact.

use crate::{
    Block, BuilderMethod, Expr, ForeignMember, FunctionDeclaration, FunctionDef, GenericArgument,
    GenericParam, Generics, ImplItem, Initializer, Item, MatchArm, Param, PathQualification,
    PathType, Pattern, QualifiedSelfType, RecordPath, Statement, TraitBoundArgument,
    TraitBoundArguments, TraitMember, Type, TypeBound, WherePredicate,
};

pub trait VisitMut {
    fn visit_item(&mut self, item: &mut Item) {
        walk_item(self, item);
    }

    fn visit_function(&mut self, function: &mut FunctionDef) {
        walk_function(self, function);
    }

    fn visit_function_declaration(&mut self, function: &mut FunctionDeclaration) {
        walk_function_declaration(self, function);
    }

    fn visit_generics(&mut self, generics: &mut Generics) {
        walk_generics(self, generics);
    }

    fn visit_block(&mut self, block: &mut Block) {
        walk_block(self, block);
    }

    fn visit_statement(&mut self, statement: &mut Statement) {
        walk_statement(self, statement);
    }

    fn visit_expr(&mut self, expression: &mut Expr) {
        walk_expr(self, expression);
    }

    fn visit_match_arm(&mut self, arm: &mut MatchArm) {
        walk_match_arm(self, arm);
    }

    fn visit_pattern(&mut self, pattern: &mut Pattern) {
        walk_pattern(self, pattern);
    }

    fn visit_type(&mut self, ty: &mut Type) {
        walk_type(self, ty);
    }

    fn visit_type_bound(&mut self, bound: &mut TypeBound) {
        walk_type_bound(self, bound);
    }

    fn visit_generic_argument(&mut self, argument: &mut GenericArgument) {
        walk_generic_argument(self, argument);
    }
}

pub fn walk_item<V: VisitMut + ?Sized>(visitor: &mut V, item: &mut Item) {
    match item {
        Item::Function(function) => visitor.visit_function(function),
        Item::Mod(module) => {
            for item in &mut module.items {
                visitor.visit_item(item);
            }
        }
        Item::Struct(data) => {
            visitor.visit_generics(&mut data.generics);
            for field in data.fields.as_mut_slice() {
                visitor.visit_type(&mut field.ty);
            }
        }
        Item::Enum(data) => {
            visitor.visit_generics(&mut data.generics);
            for variant in &mut data.variants {
                for field in variant.fields.as_mut_slice() {
                    visitor.visit_type(&mut field.ty);
                }
                if let Some(discriminant) = &mut variant.discriminant {
                    visitor.visit_expr(discriminant);
                }
            }
        }
        Item::Union(data) => {
            visitor.visit_generics(&mut data.generics);
            for field in &mut data.fields {
                visitor.visit_type(&mut field.ty);
            }
        }
        Item::Impl(block) => {
            visitor.visit_generics(&mut block.generics);
            visitor.visit_type(&mut block.target);
            if let Some(implementation) = &mut block.trait_impl {
                visitor.visit_type(implementation);
            }
            for item in &mut block.items {
                match item {
                    ImplItem::Method(method) => visitor.visit_function(method),
                    ImplItem::AssocConst(item) => {
                        visitor.visit_type(&mut item.ty);
                        visitor.visit_expr(&mut item.value);
                    }
                    ImplItem::AssocType(item) => {
                        visitor.visit_generics(&mut item.generics);
                        visitor.visit_type(&mut item.target);
                    }
                }
            }
        }
        Item::Trait(definition) => {
            visitor.visit_generics(&mut definition.generics);
            for bound in &mut definition.supertraits {
                visitor.visit_type_bound(bound);
            }
            for member in &mut definition.members {
                match member {
                    TraitMember::DefaultMethod(method) => visitor.visit_function(method),
                    TraitMember::RequiredMethod(method) => visitor.visit_function_declaration(method),
                    TraitMember::AssocConst(item) => {
                        visitor.visit_generics(&mut item.generics);
                        visitor.visit_type(&mut item.ty);
                        if let Some(Initializer::Expr(value)) = &mut item.default {
                            visitor.visit_expr(value);
                        }
                    }
                    TraitMember::AssocType(item) => {
                        visitor.visit_generics(&mut item.generics);
                        for bound in &mut item.bounds {
                            visitor.visit_type_bound(bound);
                        }
                        if let Some(ty) = &mut item.default {
                            visitor.visit_type(ty);
                        }
                    }
                    TraitMember::RetainedMethod { .. } | TraitMember::Macro(_) | TraitMember::Raw(_) => {}
                }
            }
        }
        Item::Foreign(block) => {
            for member in &mut block.members {
                match member {
                    ForeignMember::Function(item) => visitor.visit_function_declaration(item),
                    ForeignMember::Static(item) => visitor.visit_type(&mut item.ty),
                    ForeignMember::Type(item) => visitor.visit_generics(&mut item.generics),
                    ForeignMember::Macro(_) | ForeignMember::Raw(_) => {}
                }
            }
        }
        Item::Const(item) => {
            visitor.visit_type(&mut item.ty);
            visitor.visit_expr(&mut item.value);
        }
        Item::Static(item) => {
            visitor.visit_type(&mut item.ty);
            if let Initializer::Expr(value) = &mut item.value {
                visitor.visit_expr(value);
            }
        }
        Item::TypeAlias(item) => {
            visitor.visit_generics(&mut item.generics);
            visitor.visit_type(&mut item.target);
        }
        Item::LazyStatic(item) => {
            visitor.visit_type(&mut item.ty);
            visitor.visit_block(&mut item.init);
        }
        Item::Raw(_) | Item::Retained(_) | Item::Macro(_) | Item::Use(_) | Item::ModuleDecl(_) => {}
    }
}

pub fn walk_function<V: VisitMut + ?Sized>(visitor: &mut V, function: &mut FunctionDef) {
    visitor.visit_generics(&mut function.generics);
    walk_params(visitor, &mut function.params);
    visitor.visit_type(&mut function.return_type);
    visitor.visit_block(&mut function.body);
}

pub fn walk_function_declaration<V: VisitMut + ?Sized>(visitor: &mut V, declaration: &mut FunctionDeclaration) {
    visitor.visit_generics(&mut declaration.generics);
    for parameter in &mut declaration.params {
        walk_params(visitor, std::slice::from_mut(&mut parameter.param));
    }
    if let Some(pattern) = declaration.variadic.as_mut().and_then(|item| item.pattern.as_mut()) {
        visitor.visit_pattern(pattern);
    }
    visitor.visit_type(&mut declaration.return_type);
}

fn walk_params<V: VisitMut + ?Sized>(visitor: &mut V, params: &mut [Param]) {
    for parameter in params {
        if let Some(pattern) = &mut parameter.pattern {
            visitor.visit_pattern(pattern);
        }
        visitor.visit_type(&mut parameter.ty);
    }
}

pub fn walk_generics<V: VisitMut + ?Sized>(visitor: &mut V, generics: &mut Generics) {
    for parameter in &mut generics.params {
        match parameter {
            GenericParam::Type(parameter) => {
                for bound in &mut parameter.bounds {
                    visitor.visit_type_bound(bound);
                }
                if let Some(default) = &mut parameter.default {
                    visitor.visit_type(default);
                }
            }
            GenericParam::Const { ty, default, .. } => {
                visitor.visit_type(ty);
                if let Some(default) = default {
                    visitor.visit_expr(default);
                }
            }
            GenericParam::Lifetime(_) => {}
        }
    }
    for predicate in &mut generics.where_predicates {
        if let WherePredicate::Type { subject, bounds } = predicate {
            visitor.visit_type(subject);
            for bound in bounds {
                visitor.visit_type_bound(bound);
            }
        }
    }
}

pub fn walk_block<V: VisitMut + ?Sized>(visitor: &mut V, block: &mut Block) {
    for statement in &mut block.stmts {
        visitor.visit_statement(statement);
    }
    if let Some(tail) = &mut block.expr {
        visitor.visit_expr(tail);
    }
}

pub fn walk_statement<V: VisitMut + ?Sized>(visitor: &mut V, statement: &mut Statement) {
    match statement {
        Statement::Let(local) => {
            visitor.visit_pattern(&mut local.pattern);
            if let Some(ty) = &mut local.ty {
                visitor.visit_type(ty);
            }
            if let Some(initializer) = &mut local.init {
                visitor.visit_expr(initializer);
            }
        }
        Statement::LetElse {
            pattern,
            value,
            else_block,
            ..
        } => {
            visitor.visit_pattern(pattern);
            visitor.visit_expr(value);
            visitor.visit_block(else_block);
        }
        Statement::Expr(expression) => visitor.visit_expr(expression),
        Statement::Item(item) => visitor.visit_item(item),
        Statement::Macro(_) | Statement::Comment(_) => {}
    }
}

pub fn walk_expr<V: VisitMut + ?Sized>(visitor: &mut V, expression: &mut Expr) {
    match expression {
        Expr::Path(_, kind) => match kind {
            PathType::Namespace => {}
            PathType::NamespaceWithTypeArguments(segments) => {
                for segment in segments {
                    for argument in &mut segment.arguments {
                        visitor.visit_generic_argument(argument);
                    }
                }
            }
            PathType::Qualified(path) => walk_path_qualification(visitor, path),
        },
        Expr::Field { base, .. }
        | Expr::Try(base)
        | Expr::Await(base)
        | Expr::Reference(base, ..)
        | Expr::UnaryOp(_, base)
        | Expr::Parenthesized(base) => {
            visitor.visit_expr(base);
        }
        Expr::Record { path, fields, rest } => {
            if let RecordPath::WithTypeArguments { arguments, .. } = path {
                for argument in arguments {
                    visitor.visit_generic_argument(argument);
                }
            }
            for field in fields {
                visitor.visit_expr(&mut field.value);
            }
            if let Some(rest) = rest {
                visitor.visit_expr(rest);
            }
        }
        Expr::Array(items) | Expr::Tuple(items) => {
            for item in items {
                visitor.visit_expr(item);
            }
        }
        Expr::Repeat { value, len } => {
            visitor.visit_expr(value);
            visitor.visit_expr(len);
        }
        Expr::Range { start, end } => {
            if let Some(start) = start {
                visitor.visit_expr(start);
            }
            if let Some(end) = end.expression_mut() {
                visitor.visit_expr(end);
            }
        }
        Expr::Call(callee, arguments) => {
            visitor.visit_expr(callee);
            for argument in arguments {
                visitor.visit_expr(argument);
            }
        }
        Expr::MethodCall(receiver, method, arguments) => {
            visitor.visit_expr(receiver);
            for argument in method.generic_arguments.iter_mut().flatten() {
                visitor.visit_generic_argument(argument);
            }
            for argument in arguments {
                visitor.visit_expr(argument);
            }
        }
        Expr::Block(block)
        | Expr::Async { body: block, .. }
        | Expr::LabeledBlock { body: block, .. } => visitor.visit_block(block),
        Expr::Loop(block, _) | Expr::Unsafe(block) => visitor.visit_block(block),
        Expr::While {
            condition, body, ..
        } => {
            visitor.visit_expr(condition);
            visitor.visit_block(body);
        }
        Expr::For {
            pattern,
            iter,
            body,
            ..
        } => {
            visitor.visit_pattern(pattern);
            visitor.visit_expr(iter);
            visitor.visit_block(body);
        }
        Expr::Let { pattern, value } => {
            visitor.visit_pattern(pattern);
            visitor.visit_expr(value);
        }
        Expr::Return(value) | Expr::Break { value, .. } => {
            if let Some(value) = value {
                visitor.visit_expr(value);
            }
        }
        Expr::Closure(_, _, _) | Expr::TypedClosure(_, _, _, _) | Expr::AsyncClosure { .. } => {
            let (params, body, output) = match expression {
                Expr::Closure(params, body, _) => (params, body, None),
                Expr::TypedClosure(params, ty, body, _) => (params, body, Some(ty)),
                Expr::AsyncClosure {
                    params,
                    body,
                    return_type,
                    ..
                } => (params, body, return_type.as_mut()),
                _ => unreachable!("a closure has parameters and a body"),
            };
            for parameter in params {
                visitor.visit_pattern(&mut parameter.pattern);
                if let Some(ty) = &mut parameter.ty {
                    visitor.visit_type(ty);
                }
            }
            if let Some(output) = output {
                visitor.visit_type(output);
            }
            visitor.visit_expr(body);
        }
        Expr::BuilderChain(methods) => {
            for method in methods {
                if let BuilderMethod::Spawn { closure, .. } = method {
                    visitor.visit_expr(closure);
                }
            }
        }
        Expr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            visitor.visit_expr(condition);
            visitor.visit_block(then_branch);
            if let Some(otherwise) = else_branch {
                visitor.visit_block(otherwise);
            }
        }
        Expr::IfLet {
            pattern,
            value,
            then_branch,
            else_branch,
        } => {
            visitor.visit_pattern(pattern);
            visitor.visit_expr(value);
            visitor.visit_block(then_branch);
            if let Some(otherwise) = else_branch {
                visitor.visit_block(otherwise);
            }
        }
        Expr::Match { expr, arms } => {
            visitor.visit_expr(expr);
            for arm in arms {
                visitor.visit_match_arm(arm);
            }
        }
        Expr::BinaryOp(left, _, right) | Expr::Index(left, right) | Expr::Assign(left, right) => {
            visitor.visit_expr(left);
            visitor.visit_expr(right);
        }
        Expr::Cast(value, ty) => {
            visitor.visit_expr(value);
            visitor.visit_type(ty);
        }
        Expr::Ident(_) | Expr::Macro(_) | Expr::Literal(_) | Expr::Continue(_) => {}
    }
}

pub fn walk_match_arm<V: VisitMut + ?Sized>(visitor: &mut V, arm: &mut MatchArm) {
    visitor.visit_pattern(&mut arm.pattern);
    if let Some(guard) = &mut arm.guard {
        visitor.visit_expr(guard);
    }
    visitor.visit_block(&mut arm.body);
}

pub fn walk_pattern<V: VisitMut + ?Sized>(visitor: &mut V, pattern: &mut Pattern) {
    match pattern {
        Pattern::Ident { subpattern, .. } => {
            if let Some(subpattern) = subpattern {
                visitor.visit_pattern(subpattern);
            }
        }
        Pattern::Tuple(items)
        | Pattern::Slice(items)
        | Pattern::Or(items) => {
            for item in items {
                visitor.visit_pattern(item);
            }
        }
        Pattern::TupleStruct { path, fields } => {
            walk_pattern_path(visitor, path);
            for field in fields {
                visitor.visit_pattern(field);
            }
        }
        Pattern::Record { path, fields, .. } => {
            walk_pattern_path(visitor, path);
            for field in fields {
                visitor.visit_pattern(&mut field.pattern);
            }
        }
        Pattern::Typed { pattern, ty } => {
            visitor.visit_pattern(pattern);
            visitor.visit_type(ty);
        }
        Pattern::Reference { pattern, .. } | Pattern::Parenthesized(pattern) => {
            visitor.visit_pattern(pattern);
        }
        Pattern::Range { start, end } => {
            if let Some(start) = start {
                visitor.visit_expr(start);
            }
            if let Some(end) = end.expression_mut() {
                visitor.visit_expr(end);
            }
        }
        Pattern::Path(path) => walk_pattern_path(visitor, path),
        Pattern::Literal(_)
        | Pattern::Opaque(_)
        | Pattern::Wildcard
        | Pattern::Rest => {}
    }
}

fn walk_pattern_path<V: VisitMut + ?Sized>(visitor: &mut V, path: &mut crate::PatternPath) {
    for list in &mut path.arguments {
        for argument in &mut list.arguments {
            visitor.visit_generic_argument(argument);
        }
    }
}

pub fn walk_type<V: VisitMut + ?Sized>(visitor: &mut V, ty: &mut Type) {
    match ty {
        Type::Generic(_, arguments) => {
            for argument in arguments {
                visitor.visit_generic_argument(argument);
            }
        }
        Type::Tuple(types) => {
            for ty in types {
                visitor.visit_type(ty);
            }
        }
        Type::PathWithArguments(_, arguments) => {
            for list in arguments {
                for argument in &mut list.arguments {
                    visitor.visit_generic_argument(argument);
                }
            }
        }
        Type::AssociatedPath(_, qualification) => walk_path_qualification(visitor, qualification),
        Type::TraitObject { bounds, .. } | Type::ImplTraitBounds(bounds) => {
            for bound in bounds {
                visitor.visit_type_bound(bound);
            }
        }
        Type::Reference(ty, ..)
        | Type::RawPointer(ty, _)
        | Type::Slice(ty)
        | Type::Parenthesized(ty) => visitor.visit_type(ty),
        Type::Array(ty, length) => {
            visitor.visit_type(ty);
            visitor.visit_expr(length);
        }
        Type::CallableTrait(callable) => {
            for ty in &mut callable.args {
                visitor.visit_type(ty);
            }
            visitor.visit_type(&mut callable.return_type);
        }
        Type::ImplTrait {
            associated_types, ..
        } => {
            for binding in associated_types {
                visitor.visit_type(&mut binding.ty);
            }
        }
        Type::BareFn(function) => {
            for input in &mut function.inputs {
                visitor.visit_type(&mut input.ty);
            }
            if let Some(output) = &mut function.output {
                visitor.visit_type(output);
            }
        }
        Type::Path(_) | Type::Named(_) | Type::Macro(_) | Type::Unit | Type::Never | Type::Infer => {}
    }
}

fn walk_path_qualification<V: VisitMut + ?Sized>(visitor: &mut V, qualification: &mut PathQualification) {
    if let QualifiedSelfType::Type(ty) = &mut qualification.self_type {
        visitor.visit_type(ty);
    }
    for list in qualification.trait_arguments.iter_mut().chain(&mut qualification.associated_arguments) {
        for argument in &mut list.arguments {
            visitor.visit_generic_argument(argument);
        }
    }
}

pub fn walk_type_bound<V: VisitMut + ?Sized>(visitor: &mut V, bound: &mut TypeBound) {
    let TypeBound::Trait(bound) = bound else {
        return;
    };
    for segment in &mut bound.path {
        match &mut segment.arguments {
            TraitBoundArguments::None => {}
            TraitBoundArguments::Angle(arguments) => walk_trait_bound_arguments(visitor, arguments),
            TraitBoundArguments::Parenthesized { inputs, output } => {
                for input in inputs {
                    visitor.visit_type(input);
                }
                if let Some(output) = output {
                    visitor.visit_type(output);
                }
            }
        }
    }
}

fn walk_trait_bound_arguments<V: VisitMut + ?Sized>(
    visitor: &mut V,
    arguments: &mut [TraitBoundArgument],
) {
    for argument in arguments {
        match argument {
            TraitBoundArgument::Generic(argument) => visitor.visit_generic_argument(argument),
            TraitBoundArgument::AssociatedType {
                arguments, value, ..
            } => {
                if let Some(arguments) = arguments {
                    walk_trait_bound_arguments(visitor, arguments);
                }
                visitor.visit_type(value);
            }
        }
    }
}

pub fn walk_generic_argument<V: VisitMut + ?Sized>(
    visitor: &mut V,
    argument: &mut GenericArgument,
) {
    match argument {
        GenericArgument::Type(ty) => visitor.visit_type(ty),
        GenericArgument::Const(value) => visitor.visit_expr(value),
        GenericArgument::Lifetime(_) => {}
    }
}
