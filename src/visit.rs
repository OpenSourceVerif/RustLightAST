//! Read-only traversal of the structured RustLight AST.

use crate::{
    Block, BuilderMethod, Expr, FunctionDef, GenericParam, Generics, ImplItem, Item, MatchArm,
    PathType, Pattern, QualifiedSelfType, RecordPath, RustModule, Statement, TraitMember, Type,
    WherePredicate,
};

/// A visitor may override a node method and call its `walk_*` function to
/// continue into the children. Raw source and macro tokens remain opaque.
pub trait Visit<'ast> {
    fn visit_module(&mut self, module: &'ast RustModule) {
        walk_module(self, module);
    }

    fn visit_item(&mut self, item: &'ast Item) {
        walk_item(self, item);
    }

    fn visit_function(&mut self, function: &'ast FunctionDef) {
        walk_function(self, function);
    }

    fn visit_generics(&mut self, generics: &'ast Generics) {
        walk_generics(self, generics);
    }

    fn visit_block(&mut self, block: &'ast Block) {
        walk_block(self, block);
    }

    fn visit_statement(&mut self, statement: &'ast Statement) {
        walk_statement(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        walk_expr(self, expression);
    }

    fn visit_match_arm(&mut self, arm: &'ast MatchArm) {
        walk_match_arm(self, arm);
    }

    fn visit_pattern(&mut self, pattern: &'ast Pattern) {
        walk_pattern(self, pattern);
    }

    fn visit_type(&mut self, ty: &'ast Type) {
        walk_type(self, ty);
    }
}

pub fn walk_module<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, module: &'ast RustModule) {
    for item in &module.items {
        visitor.visit_item(item);
    }
}

pub fn walk_item<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, item: &'ast Item) {
    match item {
        Item::Struct(definition) => {
            visitor.visit_generics(&definition.generics);
            for field in definition.fields.as_slice() {
                visitor.visit_type(&field.ty);
            }
        }
        Item::Union(definition) => {
            visitor.visit_generics(&definition.generics);
            for field in &definition.fields {
                visitor.visit_type(&field.ty);
            }
        }
        Item::Enum(definition) => {
            visitor.visit_generics(&definition.generics);
            for variant in &definition.variants {
                for field in variant.fields.as_slice() {
                    visitor.visit_type(&field.ty);
                }
                if let Some(discriminant) = &variant.discriminant {
                    visitor.visit_expr(discriminant);
                }
            }
        }
        Item::Function(function) => visitor.visit_function(function),
        Item::Trait(definition) => {
            visitor.visit_generics(&definition.generics);
            for member in &definition.members {
                match member {
                    TraitMember::DefaultMethod(function) => visitor.visit_function(function),
                    TraitMember::Raw(_) => {}
                }
            }
        }
        Item::Impl(implementation) => {
            visitor.visit_generics(&implementation.generics);
            if let Some(trait_type) = &implementation.trait_impl {
                visitor.visit_type(trait_type);
            }
            visitor.visit_type(&implementation.target);
            for item in &implementation.items {
                match item {
                    ImplItem::Method(function) => visitor.visit_function(function),
                    ImplItem::AssocConst(definition) => {
                        visitor.visit_type(&definition.ty);
                        visitor.visit_expr(&definition.value);
                    }
                    ImplItem::AssocType(alias) => {
                        visitor.visit_generics(&alias.generics);
                        visitor.visit_type(&alias.target);
                    }
                }
            }
        }
        Item::Const(definition) => {
            visitor.visit_type(&definition.ty);
            visitor.visit_expr(&definition.value);
        }
        Item::TypeAlias(alias) => {
            visitor.visit_generics(&alias.generics);
            visitor.visit_type(&alias.target);
        }
        Item::LazyStatic(definition) => {
            visitor.visit_type(&definition.ty);
            visitor.visit_block(&definition.init);
        }
        Item::Mod(module) => visitor.visit_module(module),
        Item::Raw(_) | Item::Use(_) | Item::ModuleDecl(_) => {}
    }
}

pub fn walk_function<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, function: &'ast FunctionDef) {
    visitor.visit_generics(&function.generics);
    for param in &function.params {
        visitor.visit_type(&param.ty);
    }
    visitor.visit_type(&function.return_type);
    visitor.visit_block(&function.body);
}

pub fn walk_generics<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, generics: &'ast Generics) {
    for param in &generics.params {
        match param {
            GenericParam::Const { ty, default, .. } => {
                visitor.visit_type(ty);
                if let Some(default) = default {
                    visitor.visit_expr(default);
                }
            }
            GenericParam::Type(parameter) => {
                if let Some(default) = &parameter.default {
                    visitor.visit_type(default);
                }
            }
            GenericParam::Lifetime(_) => {}
        }
    }
    for predicate in &generics.where_predicates {
        match predicate {
            WherePredicate::Type { subject, .. } => visitor.visit_type(subject),
            WherePredicate::Lifetime { .. } => {}
        }
    }
}

pub fn walk_block<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, block: &'ast Block) {
    for statement in &block.stmts {
        visitor.visit_statement(statement);
    }
    if let Some(tail) = &block.expr {
        visitor.visit_expr(tail);
    }
}

pub fn walk_statement<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, statement: &'ast Statement) {
    match statement {
        Statement::Let(local) => {
            if let Some(ty) = &local.ty {
                visitor.visit_type(ty);
            }
            if let Some(init) = &local.init {
                visitor.visit_expr(init);
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

pub fn walk_expr<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, expression: &'ast Expr) {
    match expression {
        Expr::Ident(_) | Expr::Macro(_) | Expr::Literal(_) | Expr::Continue(_) => {}
        Expr::Path(_, kind) => match kind {
            PathType::Namespace => {}
            PathType::NamespaceWithTypeArguments(segments) => {
                for segment in segments {
                    for argument in &segment.arguments {
                        visitor.visit_type(argument);
                    }
                }
            }
            PathType::Qualified(qualification) => match &qualification.self_type {
                QualifiedSelfType::Type(ty) => visitor.visit_type(ty),
                QualifiedSelfType::Verbatim(_) => {}
            },
        },
        Expr::Field { base, .. }
        | Expr::Try(base)
        | Expr::Await(base)
        | Expr::Reference(base, _, _)
        | Expr::UnaryOp(_, base)
        | Expr::Parenthesized(base) => visitor.visit_expr(base),
        Expr::Record { path, fields, rest } => {
            match path {
                RecordPath::Segments(_) => {}
                RecordPath::WithTypeArguments { arguments, .. } => {
                    for argument in arguments {
                        visitor.visit_type(argument);
                    }
                }
            }
            for field in fields {
                visitor.visit_expr(&field.value);
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
            for bound in start.as_deref().into_iter().chain(end.expression()) {
                visitor.visit_expr(bound);
            }
        }
        Expr::Call(callee, args) | Expr::MethodCall(callee, _, args) => {
            visitor.visit_expr(callee);
            for arg in args {
                visitor.visit_expr(arg);
            }
        }
        Expr::Block(block)
        | Expr::LabeledBlock { body: block, .. }
        | Expr::Async { body: block, .. } => visitor.visit_block(block),
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
        Expr::Closure(params, body, _)
        | Expr::TypedClosure(params, _, body, _)
        | Expr::AsyncClosure { params, body, .. } => {
            for param in params {
                if let Some(ty) = &param.ty {
                    visitor.visit_type(ty);
                }
            }
            if let Expr::TypedClosure(_, output, _, _)
            | Expr::AsyncClosure { return_type: Some(output), .. } = expression
            {
                visitor.visit_type(output);
            }
            visitor.visit_expr(body);
        }
        Expr::BuilderChain(methods) => {
            for method in methods {
                match method {
                    BuilderMethod::Spawn { closure, .. } => visitor.visit_expr(closure),
                    BuilderMethod::Named(_) => {}
                }
            }
        }
        Expr::If {
            condition: value,
            then_branch,
            else_branch,
        }
        | Expr::IfLet {
            value,
            then_branch,
            else_branch,
            ..
        } => {
            if let Expr::IfLet { pattern, .. } = expression {
                visitor.visit_pattern(pattern);
            }
            visitor.visit_expr(value);
            visitor.visit_block(then_branch);
            if let Some(block) = else_branch {
                visitor.visit_block(block);
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
    }
}

pub fn walk_match_arm<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, arm: &'ast MatchArm) {
    visitor.visit_pattern(&arm.pattern);
    if let Some(guard) = &arm.guard {
        visitor.visit_expr(guard);
    }
    visitor.visit_block(&arm.body);
}

pub fn walk_pattern<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, pattern: &'ast Pattern) {
    match pattern {
        Pattern::Ident { subpattern, .. } => {
            if let Some(subpattern) = subpattern {
                visitor.visit_pattern(subpattern);
            }
        }
        Pattern::Tuple(items)
        | Pattern::TupleStruct { fields: items, .. }
        | Pattern::Slice(items)
        | Pattern::Or(items) => {
            for item in items {
                visitor.visit_pattern(item);
            }
        }
        Pattern::Record { fields, .. } => {
            for field in fields {
                visitor.visit_pattern(&field.pattern);
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
            for bound in start.as_deref().into_iter().chain(end.expression()) {
                visitor.visit_expr(bound);
            }
        }
        Pattern::Path(_) | Pattern::Literal(_) | Pattern::Opaque(_) | Pattern::Wildcard | Pattern::Rest => {}
    }
}

pub fn walk_type<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, ty: &'ast Type) {
    match ty {
        Type::Generic(_, arguments) | Type::Tuple(arguments) => {
            for argument in arguments {
                visitor.visit_type(argument);
            }
        }
        Type::CallableTrait(callable) => {
            for argument in &callable.args {
                visitor.visit_type(argument);
            }
            visitor.visit_type(&callable.return_type);
        }
        Type::ImplTrait {
            associated_types, ..
        } => {
            for binding in associated_types {
                visitor.visit_type(&binding.ty);
            }
        }
        Type::BareFn(function) => {
            for input in &function.inputs {
                visitor.visit_type(&input.ty);
            }
            if let Some(output) = &function.output {
                visitor.visit_type(output);
            }
        }
        Type::Reference(ty, _, _) | Type::RawPointer(ty, _)
        | Type::Slice(ty) | Type::Array(ty, _) => {
            visitor.visit_type(ty);
        }
        Type::Path(_) | Type::Named(_) | Type::Unit | Type::Never | Type::Infer => {}
    }
}
