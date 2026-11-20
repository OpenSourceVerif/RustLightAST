//! Read-only traversal of the structured RustLight AST.

use crate::{
    Attribute, AttributeArg, Block, BuilderMethod, CallableBound, DataAttribute, Expr, Field,
    ForeignMember, FunctionDeclaration, FunctionDef, GenericArgument, GenericParam, Generics,
    ImplItem, Initializer, Item, Literal, MacroInvocation, MatchArm, Member, MemberMacro, Param,
    PathQualification, PathType, Pattern, QualifiedSelfType, RecordPath, RustModule, Statement,
    TraitMember, Type, UseTree, Visibility, WherePredicate,
};

/// A visitor may override a node method and call its `walk_*` function to
/// continue into the children. Raw source and macro tokens remain opaque.
pub trait Visit<'ast> {
    /// Visit a spelling that can contain identifiers, including preserved opaque
    /// tokens. The visitor may inspect the spelling without interpreting opaque syntax.
    fn visit_spelling(&mut self, _source: &'ast str) {}

    fn visit_opaque(&mut self, source: &'ast str) {
        self.visit_spelling(source);
    }

    fn visit_written_attribute(&mut self, source: &'ast str) {
        self.visit_spelling(source);
    }

    fn visit_module(&mut self, module: &'ast RustModule) {
        walk_module(self, module);
    }

    fn visit_item(&mut self, item: &'ast Item) {
        walk_item(self, item);
    }

    fn visit_function(&mut self, function: &'ast FunctionDef) {
        walk_function(self, function);
    }

    fn visit_function_declaration(&mut self, function: &'ast FunctionDeclaration) {
        walk_function_declaration(self, function);
    }

    fn visit_trait_member(&mut self, member: &'ast TraitMember) {
        walk_trait_member(self, member);
    }

    fn visit_impl_item(&mut self, item: &'ast ImplItem) {
        walk_impl_item(self, item);
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        walk_attribute(self, attribute);
    }

    fn visit_data_attribute(&mut self, attribute: &'ast DataAttribute) {
        walk_data_attribute(self, attribute);
    }

    fn visit_macro_invocation(&mut self, invocation: &'ast MacroInvocation) {
        walk_macro_invocation(self, invocation);
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

    fn visit_type_bound(&mut self, bound: &'ast crate::TypeBound) {
        walk_type_bound(self, bound);
    }

    fn visit_generic_argument(&mut self, argument: &'ast GenericArgument) {
        walk_generic_argument(self, argument);
    }
}

pub fn walk_module<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, module: &'ast RustModule) {
    walk_attributes(visitor, &module.attrs);
    walk_visibility(visitor, &module.vis);
    for item in &module.items {
        visitor.visit_item(item);
    }
}

pub fn walk_item<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, item: &'ast Item) {
    match item {
        Item::Struct(definition) => {
            visitor.visit_spelling(&definition.name);
            walk_data_attributes(visitor, &definition.attrs);
            walk_visibility(visitor, &definition.vis);
            visitor.visit_generics(&definition.generics);
            for field in definition.fields.as_slice() {
                walk_field(visitor, field);
            }
        }
        Item::Union(definition) => {
            visitor.visit_spelling(&definition.name);
            walk_data_attributes(visitor, &definition.attrs);
            walk_visibility(visitor, &definition.vis);
            visitor.visit_generics(&definition.generics);
            for field in &definition.fields {
                walk_field(visitor, field);
            }
        }
        Item::Enum(definition) => {
            visitor.visit_spelling(&definition.name);
            walk_data_attributes(visitor, &definition.attrs);
            walk_visibility(visitor, &definition.vis);
            visitor.visit_generics(&definition.generics);
            for variant in &definition.variants {
                visitor.visit_spelling(&variant.name);
                walk_written_attributes(visitor, &variant.attrs);
                for field in variant.fields.as_slice() {
                    walk_field(visitor, field);
                }
                if let Some(discriminant) = &variant.discriminant {
                    visitor.visit_expr(discriminant);
                }
            }
        }
        Item::Function(function) => visitor.visit_function(function),
        Item::Trait(definition) => {
            visitor.visit_spelling(&definition.name);
            walk_attributes(visitor, &definition.attrs);
            walk_visibility(visitor, &definition.vis);
            visitor.visit_generics(&definition.generics);
            for bound in &definition.supertraits {
                visitor.visit_type_bound(bound);
            }
            for member in &definition.members {
                visitor.visit_trait_member(member);
            }
        }
        Item::Foreign(block) => {
            walk_attributes(visitor, &block.attrs);
            for member in &block.members {
                match member {
                    ForeignMember::Function(declaration) => visitor.visit_function_declaration(declaration),
                    ForeignMember::Static(declaration) => {
                        walk_attributes(visitor, &declaration.attrs);
                        walk_visibility(visitor, &declaration.vis);
                        visitor.visit_spelling(&declaration.name);
                        visitor.visit_type(&declaration.ty);
                    }
                    ForeignMember::Type(declaration) => {
                        walk_attributes(visitor, &declaration.attrs);
                        walk_visibility(visitor, &declaration.vis);
                        visitor.visit_spelling(&declaration.name);
                        visitor.visit_generics(&declaration.generics);
                    }
                    ForeignMember::Macro(mac) => walk_member_macro(visitor, mac),
                    ForeignMember::Raw(source) => visitor.visit_opaque(source),
                }
            }
        }
        Item::Impl(implementation) => {
            walk_attributes(visitor, &implementation.attrs);
            visitor.visit_generics(&implementation.generics);
            if let Some(trait_type) = &implementation.trait_impl {
                visitor.visit_type(trait_type);
            }
            visitor.visit_type(&implementation.target);
            for item in &implementation.items {
                visitor.visit_impl_item(item);
            }
        }
        Item::Const(definition) => {
            visitor.visit_spelling(&definition.name);
            walk_attributes(visitor, &definition.attrs);
            walk_visibility(visitor, &definition.vis);
            visitor.visit_type(&definition.ty);
            visitor.visit_expr(&definition.value);
        }
        Item::Static(definition) => {
            visitor.visit_spelling(&definition.name);
            walk_attributes(visitor, &definition.attrs);
            walk_visibility(visitor, &definition.vis);
            visitor.visit_type(&definition.ty);
            walk_initializer(visitor, &definition.value);
        }
        Item::TypeAlias(alias) => {
            visitor.visit_spelling(&alias.name);
            walk_attributes(visitor, &alias.attrs);
            walk_visibility(visitor, &alias.vis);
            visitor.visit_generics(&alias.generics);
            visitor.visit_type(&alias.target);
        }
        Item::LazyStatic(definition) => {
            visitor.visit_spelling(&definition.name);
            walk_visibility(visitor, &definition.vis);
            visitor.visit_type(&definition.ty);
            visitor.visit_block(&definition.init);
        }
        Item::Mod(module) => {
            visitor.visit_spelling(&module.name);
            visitor.visit_module(module);
        }
        Item::Raw(source) => visitor.visit_opaque(source),
        Item::Retained(item) => visitor.visit_spelling(item.source()),
        Item::Macro(mac) => walk_macro_statement(visitor, mac),
        Item::Use(import) => {
            walk_written_attributes(visitor, &import.attrs);
            walk_visibility(visitor, &import.vis);
            walk_use_tree(visitor, &import.tree);
        }
        Item::ModuleDecl(module) => {
            visitor.visit_spelling(&module.name);
            walk_written_attributes(visitor, &module.attrs);
            walk_visibility(visitor, &module.vis);
        }
    }
}

fn walk_initializer<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, initializer: &'ast Initializer) {
    match initializer {
        Initializer::Expr(value) => visitor.visit_expr(value),
        Initializer::Raw(source) => visitor.visit_opaque(source),
    }
}

pub fn walk_trait_member<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, member: &'ast TraitMember) {
    match member {
        TraitMember::DefaultMethod(function) => visitor.visit_function(function),
        TraitMember::RequiredMethod(function) => visitor.visit_function_declaration(function),
        TraitMember::RetainedMethod { declaration, source } => {
            visitor.visit_function_declaration(declaration);
            visitor.visit_opaque(source);
        }
        TraitMember::AssocConst(associated) => {
            walk_attributes(visitor, &associated.attrs);
            visitor.visit_spelling(&associated.name);
            visitor.visit_generics(&associated.generics);
            visitor.visit_type(&associated.ty);
            if let Some(value) = &associated.default {
                walk_initializer(visitor, value);
            }
        }
        TraitMember::AssocType(associated) => {
            walk_attributes(visitor, &associated.attrs);
            visitor.visit_spelling(&associated.name);
            visitor.visit_generics(&associated.generics);
            for bound in &associated.bounds {
                visitor.visit_type_bound(bound);
            }
            if let Some(ty) = &associated.default {
                visitor.visit_type(ty);
            }
        }
        TraitMember::Macro(mac) => walk_member_macro(visitor, mac),
        TraitMember::Raw(source) => visitor.visit_opaque(source),
    }
}

pub fn walk_impl_item<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, item: &'ast ImplItem) {
    match item {
        ImplItem::Method(function) => visitor.visit_function(function),
        ImplItem::AssocConst(definition) => {
            visitor.visit_spelling(&definition.name);
            walk_attributes(visitor, &definition.attrs);
            walk_visibility(visitor, &definition.vis);
            visitor.visit_type(&definition.ty);
            visitor.visit_expr(&definition.value);
        }
        ImplItem::AssocType(alias) => {
            visitor.visit_spelling(&alias.name);
            walk_attributes(visitor, &alias.attrs);
            walk_visibility(visitor, &alias.vis);
            visitor.visit_generics(&alias.generics);
            visitor.visit_type(&alias.target);
        }
    }
}

fn walk_member_macro<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, mac: &'ast MemberMacro) {
    walk_attributes(visitor, &mac.attrs);
    visitor.visit_macro_invocation(&mac.invocation);
}

fn walk_macro_statement<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, mac: &'ast crate::MacroStmt) {
    walk_written_attributes(visitor, &mac.attrs);
    walk_optional_spelling(visitor, &mac.name);
    visitor.visit_macro_invocation(&mac.invocation);
}

pub fn walk_macro_invocation<'ast, V: Visit<'ast> + ?Sized>(
    visitor: &mut V, invocation: &'ast MacroInvocation,
) {
    walk_spellings(visitor, &invocation.path);
    if let Some(source) = invocation.tokens.written_source() {
        visitor.visit_spelling(source);
    } else {
        walk_spellings(visitor, invocation.tokens.identifiers_in_tokens());
    }
}

fn walk_use_tree<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, tree: &'ast UseTree) {
    match tree {
        UseTree::Path { segment, tree } => {
            visitor.visit_spelling(segment);
            walk_use_tree(visitor, tree);
        }
        UseTree::Name(name) => visitor.visit_spelling(name),
        UseTree::Rename { name, alias } => {
            visitor.visit_spelling(name);
            visitor.visit_spelling(alias);
        }
        UseTree::Group(trees) => {
            for tree in trees {
                walk_use_tree(visitor, tree);
            }
        }
        UseTree::Glob => {}
    }
}

pub fn walk_function<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, function: &'ast FunctionDef) {
    visitor.visit_spelling(&function.name);
    walk_attributes(visitor, &function.attrs);
    walk_visibility(visitor, &function.vis);
    visitor.visit_generics(&function.generics);
    walk_params(visitor, &function.params);
    visitor.visit_type(&function.return_type);
    visitor.visit_block(&function.body);
}

pub fn walk_function_declaration<'ast, V: Visit<'ast> + ?Sized>(
    visitor: &mut V, declaration: &'ast FunctionDeclaration,
) {
    visitor.visit_spelling(&declaration.name);
    walk_attributes(visitor, &declaration.attrs);
    walk_visibility(visitor, &declaration.vis);
    visitor.visit_generics(&declaration.generics);
    for parameter in &declaration.params {
        walk_attributes(visitor, &parameter.attrs);
        walk_params(visitor, std::slice::from_ref(&parameter.param));
    }
    if let Some(variadic) = &declaration.variadic {
        walk_attributes(visitor, &variadic.attrs);
        if let Some(pattern) = &variadic.pattern {
            visitor.visit_pattern(pattern);
        }
    }
    visitor.visit_type(&declaration.return_type);
}

fn walk_params<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, params: &'ast [Param]) {
    for param in params {
        if let Some(pattern) = &param.pattern {
            visitor.visit_pattern(pattern);
        } else {
            visitor.visit_spelling(&param.name);
        }
        visitor.visit_type(&param.ty);
    }
}

pub fn walk_generics<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, generics: &'ast Generics) {
    for param in &generics.params {
        visitor.visit_spelling(param.name());
        match param {
            GenericParam::Const { ty, default, .. } => {
                visitor.visit_type(ty);
                if let Some(default) = default {
                    visitor.visit_expr(default);
                }
            }
            GenericParam::Type(parameter) => {
                for bound in &parameter.bounds {
                    visitor.visit_type_bound(bound);
                }
                if let Some(default) = &parameter.default {
                    visitor.visit_type(default);
                }
            }
            GenericParam::Lifetime(parameter) => walk_spellings(visitor, &parameter.bounds),
        }
    }
    for predicate in &generics.where_predicates {
        match predicate {
            WherePredicate::Type { subject, bounds } => {
                visitor.visit_type(subject);
                for bound in bounds {
                    visitor.visit_type_bound(bound);
                }
            }
            WherePredicate::Lifetime { subject, bounds } => {
                visitor.visit_spelling(subject);
                walk_spellings(visitor, bounds);
            }
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
            visitor.visit_pattern(&local.pattern);
            walk_written_attributes(visitor, &local.attrs);
            if let Some(ty) = &local.ty {
                visitor.visit_type(ty);
            }
            if let Some(init) = &local.init {
                visitor.visit_expr(init);
            }
        }
        Statement::LetElse {
            attrs,
            pattern,
            value,
            else_block,
        } => {
            walk_written_attributes(visitor, attrs);
            visitor.visit_pattern(pattern);
            visitor.visit_expr(value);
            visitor.visit_block(else_block);
        }
        Statement::Expr(expression) => visitor.visit_expr(expression),
        Statement::Item(item) => visitor.visit_item(item),
        Statement::Macro(mac) => walk_macro_statement(visitor, mac),
        Statement::Comment(_) => {}
    }
}

pub fn walk_expr<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, expression: &'ast Expr) {
    match expression {
        Expr::Ident(name) => visitor.visit_spelling(name),
        Expr::Macro(mac) => {
            walk_written_attributes(visitor, &mac.attrs);
            visitor.visit_macro_invocation(&mac.invocation);
        }
        Expr::Literal(literal) => walk_literal(visitor, literal),
        Expr::Continue(label) => walk_optional_spelling(visitor, label),
        Expr::Path(path, kind) => {
            walk_spellings(visitor, path);
            match kind {
                PathType::Namespace => {}
                PathType::NamespaceWithTypeArguments(segments) => {
                    for segment in segments {
                        for argument in &segment.arguments {
                            visitor.visit_generic_argument(argument);
                        }
                    }
                }
                PathType::Qualified(qualification) => {
                    walk_path_qualification(visitor, qualification);
                }
            }
        }
        Expr::Field { base, member } => {
            visitor.visit_expr(base);
            walk_member(visitor, member);
        }
        Expr::Try(base)
        | Expr::Await(base)
        | Expr::Reference(base, _, _)
        | Expr::UnaryOp(_, base)
        | Expr::Parenthesized(base) => visitor.visit_expr(base),
        Expr::Record { path, fields, rest } => {
            match path {
                RecordPath::Segments(segments) => walk_spellings(visitor, segments),
                RecordPath::WithTypeArguments {
                    arguments,
                    segments,
                } => {
                    walk_spellings(visitor, segments);
                    for argument in arguments {
                        visitor.visit_generic_argument(argument);
                    }
                }
            }
            for field in fields {
                walk_member(visitor, &field.member);
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
            if let Expr::MethodCall(_, method, _) = expression {
                visitor.visit_spelling(&method.name);
                if let Some(arguments) = &method.generic_arguments {
                    for argument in arguments {
                        visitor.visit_generic_argument(argument);
                    }
                }
            }
            visitor.visit_expr(callee);
            for arg in args {
                visitor.visit_expr(arg);
            }
        }
        Expr::Block(block) | Expr::Async { body: block, .. } => visitor.visit_block(block),
        Expr::LabeledBlock { label, body } => {
            visitor.visit_spelling(label);
            visitor.visit_block(body);
        }
        Expr::Loop(block, label) => {
            walk_optional_spelling(visitor, label);
            visitor.visit_block(block);
        }
        Expr::Unsafe(block) => visitor.visit_block(block),
        Expr::While {
            label,
            condition,
            body,
        } => {
            walk_optional_spelling(visitor, label);
            visitor.visit_expr(condition);
            visitor.visit_block(body);
        }
        Expr::For {
            label,
            pattern,
            iter,
            body,
        } => {
            walk_optional_spelling(visitor, label);
            visitor.visit_pattern(pattern);
            visitor.visit_expr(iter);
            visitor.visit_block(body);
        }
        Expr::Let { pattern, value } => {
            visitor.visit_pattern(pattern);
            visitor.visit_expr(value);
        }
        Expr::Return(value) | Expr::Break { value, .. } => {
            if let Expr::Break { label, .. } = expression {
                walk_optional_spelling(visitor, label);
            }
            if let Some(value) = value {
                visitor.visit_expr(value);
            }
        }
        Expr::Closure(params, body, _)
        | Expr::TypedClosure(params, _, body, _)
        | Expr::AsyncClosure { params, body, .. } => {
            for param in params {
                visitor.visit_pattern(&param.pattern);
                if let Some(ty) = &param.ty {
                    visitor.visit_type(ty);
                }
            }
            if let Expr::TypedClosure(_, output, _, _)
            | Expr::AsyncClosure {
                return_type: Some(output),
                ..
            } = expression
            {
                visitor.visit_type(output);
            }
            visitor.visit_expr(body);
        }
        Expr::BuilderChain(methods) => {
            for method in methods {
                match method {
                    BuilderMethod::Spawn { closure, .. } => visitor.visit_expr(closure),
                    BuilderMethod::Named(source) => visitor.visit_spelling(source),
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
        Pattern::Ident {
            name, subpattern, ..
        } => {
            visitor.visit_spelling(name);
            if let Some(subpattern) = subpattern {
                visitor.visit_pattern(subpattern);
            }
        }
        Pattern::Tuple(items)
        | Pattern::TupleStruct { fields: items, .. }
        | Pattern::Slice(items)
        | Pattern::Or(items) => {
            if let Pattern::TupleStruct { path, .. } = pattern {
                walk_pattern_path(visitor, path);
            }
            for item in items {
                visitor.visit_pattern(item);
            }
        }
        Pattern::Record { path, fields, .. } => {
            walk_pattern_path(visitor, path);
            for field in fields {
                walk_member(visitor, &field.member);
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
        Pattern::Path(path) => walk_pattern_path(visitor, path),
        Pattern::Literal(literal) => walk_literal(visitor, literal),
        Pattern::Opaque(source) => visitor.visit_opaque(source),
        Pattern::Wildcard | Pattern::Rest => {}
    }
}

fn walk_pattern_path<'ast, V: Visit<'ast> + ?Sized>(
    visitor: &mut V,
    path: &'ast crate::PatternPath,
) {
    walk_spellings(visitor, &path.segments);
    for list in &path.arguments {
        for argument in &list.arguments {
            visitor.visit_generic_argument(argument);
        }
    }
}

pub fn walk_type<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, ty: &'ast Type) {
    match ty {
        Type::Generic(name, arguments) => {
            visitor.visit_spelling(name);
            for argument in arguments {
                visitor.visit_generic_argument(argument);
            }
        }
        Type::Tuple(arguments) => {
            for argument in arguments {
                visitor.visit_type(argument);
            }
        }
        Type::PathWithArguments(path, arguments) => {
            walk_spellings(visitor, path);
            for list in arguments {
                for argument in &list.arguments {
                    visitor.visit_generic_argument(argument);
                }
            }
        }
        Type::AssociatedPath(path, qualification) => {
            walk_spellings(visitor, path);
            walk_path_qualification(visitor, qualification);
        }
        Type::TraitObject { bounds, .. } | Type::ImplTraitBounds(bounds) => {
            for bound in bounds {
                visitor.visit_type_bound(bound);
            }
        }
        Type::CallableTrait(callable) => {
            walk_spellings(visitor, &callable.trait_path);
            for bound in callable
                .leading_bounds
                .iter()
                .chain(&callable.trailing_bounds)
            {
                match bound {
                    CallableBound::Lifetime(lifetime) => visitor.visit_spelling(lifetime),
                    CallableBound::Trait(path) => walk_spellings(visitor, path),
                }
            }
            for argument in &callable.args {
                visitor.visit_type(argument);
            }
            visitor.visit_type(&callable.return_type);
        }
        Type::ImplTrait {
            trait_path,
            associated_types,
        } => {
            walk_spellings(visitor, trait_path);
            for binding in associated_types {
                visitor.visit_spelling(&binding.name);
                visitor.visit_type(&binding.ty);
            }
        }
        Type::BareFn(function) => {
            for parameter in &function.higher_ranked {
                visitor.visit_spelling(&parameter.name);
                walk_spellings(visitor, &parameter.bounds);
            }
            for input in &function.inputs {
                walk_attributes(visitor, &input.attrs);
                walk_optional_spelling(visitor, &input.name);
                visitor.visit_type(&input.ty);
            }
            if let Some(variadic) = &function.variadic {
                walk_attributes(visitor, &variadic.attrs);
                walk_optional_spelling(visitor, &variadic.name);
            }
            if let Some(output) = &function.output {
                visitor.visit_type(output);
            }
        }
        Type::Reference(ty, lifetime, _) => {
            walk_optional_spelling(visitor, lifetime);
            visitor.visit_type(ty);
        }
        Type::RawPointer(ty, _) | Type::Slice(ty) | Type::Parenthesized(ty) => {
            visitor.visit_type(ty);
        }
        Type::Array(ty, length) => {
            visitor.visit_type(ty);
            visitor.visit_expr(length);
        }
        Type::Path(path) => walk_spellings(visitor, path),
        Type::Named(name) => visitor.visit_spelling(name),
        Type::Macro(invocation) => visitor.visit_macro_invocation(invocation),
        Type::Unit | Type::Never | Type::Infer => {}
    }
}

fn walk_path_qualification<'ast, V: Visit<'ast> + ?Sized>(
    visitor: &mut V,
    qualification: &'ast PathQualification,
) {
    match &qualification.self_type {
        QualifiedSelfType::Type(ty) => visitor.visit_type(ty),
        QualifiedSelfType::Verbatim(source) => visitor.visit_opaque(source),
    }
    if let Some(path) = &qualification.trait_path {
        walk_spellings(visitor, path);
    }
    for list in qualification.trait_arguments.iter().chain(&qualification.associated_arguments) {
        for argument in &list.arguments {
            visitor.visit_generic_argument(argument);
        }
    }
}

pub fn walk_generic_argument<'ast, V: Visit<'ast> + ?Sized>(
    visitor: &mut V,
    argument: &'ast GenericArgument,
) {
    match argument {
        GenericArgument::Type(ty) => visitor.visit_type(ty),
        GenericArgument::Lifetime(lifetime) => visitor.visit_spelling(lifetime),
        GenericArgument::Const(expression) => visitor.visit_expr(expression),
    }
}

pub fn walk_type_bound<'ast, V: Visit<'ast> + ?Sized>(
    visitor: &mut V,
    bound: &'ast crate::TypeBound,
) {
    use crate::{TraitBoundArguments, TypeBound};
    let bound = match bound {
        TypeBound::Lifetime(source) => {
            visitor.visit_spelling(source);
            return;
        }
        TypeBound::Opaque(source) => {
            visitor.visit_opaque(source);
            return;
        }
        TypeBound::Trait(bound) => bound,
    };
    for parameter in &bound.higher_ranked {
        visitor.visit_spelling(&parameter.name);
        walk_spellings(visitor, &parameter.bounds);
    }
    for segment in &bound.path {
        visitor.visit_spelling(&segment.name);
        match &segment.arguments {
            TraitBoundArguments::None => {}
            TraitBoundArguments::Angle(arguments) => walk_type_bound_arguments(visitor, arguments),
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

fn walk_type_bound_arguments<'ast, V: Visit<'ast> + ?Sized>(
    visitor: &mut V,
    arguments: &'ast [crate::TraitBoundArgument],
) {
    for argument in arguments {
        match argument {
            crate::TraitBoundArgument::Generic(argument) => {
                visitor.visit_generic_argument(argument)
            }
            crate::TraitBoundArgument::AssociatedType {
                name,
                arguments,
                value,
            } => {
                visitor.visit_spelling(name);
                if let Some(arguments) = arguments {
                    walk_type_bound_arguments(visitor, arguments);
                }
                visitor.visit_type(value);
            }
        }
    }
}

fn walk_spellings<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, values: &'ast [String]) {
    for value in values {
        visitor.visit_spelling(value);
    }
}

fn walk_optional_spelling<'ast, V: Visit<'ast> + ?Sized>(
    visitor: &mut V,
    value: &'ast Option<String>,
) {
    if let Some(value) = value {
        visitor.visit_spelling(value);
    }
}

fn walk_member<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, member: &'ast Member) {
    if let Member::Named(name) = member {
        visitor.visit_spelling(name);
    }
}

fn walk_field<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, field: &'ast Field) {
    visitor.visit_spelling(&field.name);
    walk_attributes(visitor, &field.attrs);
    walk_visibility(visitor, &field.vis);
    visitor.visit_type(&field.ty);
}

fn walk_visibility<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, visibility: &'ast Visibility) {
    if let Visibility::Restricted(path) = visibility {
        walk_spellings(visitor, path);
    }
}

fn walk_data_attributes<'ast, V: Visit<'ast> + ?Sized>(
    visitor: &mut V,
    attrs: &'ast [DataAttribute],
) {
    for attr in attrs {
        visitor.visit_data_attribute(attr);
    }
}

pub fn walk_data_attribute<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, attr: &'ast DataAttribute) {
    match attr {
        DataAttribute::Derive { paths, .. } => {
            for path in paths {
                walk_spellings(visitor, path);
            }
        }
        DataAttribute::Other(attr) => visitor.visit_attribute(attr),
    }
}

fn walk_attributes<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, attrs: &'ast [Attribute]) {
    for attr in attrs {
        visitor.visit_attribute(attr);
    }
}

fn walk_written_attributes<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, attrs: &'ast [String]) {
    for attr in attrs {
        visitor.visit_written_attribute(attr);
    }
}

pub fn walk_attribute<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, attr: &'ast Attribute) {
    match attr {
        Attribute::Structured { name, args } => {
            visitor.visit_spelling(name);
            for arg in args {
                match arg {
                    AttributeArg::Ident(name) => visitor.visit_spelling(name),
                    AttributeArg::Literal(literal) => walk_literal(visitor, literal),
                    AttributeArg::KeyValue(key, literal) => {
                        visitor.visit_spelling(key);
                        walk_literal(visitor, literal);
                    }
                }
            }
        }
        Attribute::Source { meta, .. } => visitor.visit_spelling(meta),
    }
}

fn walk_literal<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, literal: &'ast Literal) {
    match literal {
        Literal::Raw(source)
        | Literal::Byte { spelling: source, .. }
        | Literal::StringToken { spelling: source, .. }
        | Literal::CharToken { spelling: source, .. } => {
            visitor.visit_spelling(source);
        }
        Literal::FloatToken { digits, suffix } => {
            visitor.visit_spelling(digits);
            walk_optional_spelling(visitor, suffix);
        }
        _ => {}
    }
}
