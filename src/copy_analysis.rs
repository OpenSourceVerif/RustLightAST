use std::collections::{HashMap, HashSet};

use crate::rustlight_ast::*;

/// Result of the copy-analysis pass.
#[derive(Debug, Clone, Default)]
pub struct CopyAnalysis {
    pub copy_types: HashSet<String>,
}

type TypeEnv = HashMap<String, Type>;

#[derive(Debug, Clone)]
struct FieldInfo {
    name: String,
    ty: Type,
}

#[derive(Debug, Clone)]
struct VariantInfo {
    name: String,
    fields: Vec<Type>,
}

#[derive(Debug, Clone)]
enum TypeDefKind {
    Struct(Vec<FieldInfo>),
    Enum(Vec<VariantInfo>),
}

#[derive(Debug, Clone)]
struct TypeDef {
    kind: TypeDefKind,
}

struct CopyContext {
    copy_types: HashSet<String>,
    type_defs: HashMap<String, TypeDef>,
    type_aliases: HashMap<String, Type>,
    variant_owners: HashMap<String, Option<String>>,
    functions: HashMap<String, Type>,
}

/// Infer Copy data types, add Copy derives, and remove redundant `.clone()`
/// calls whose receiver has a statically known Copy type.
pub fn optimize_copy(module: &mut RustModule) -> CopyAnalysis {
    let mut analysis = CopyAnalysis::default();
    optimize_module(module, &mut analysis);
    analysis
}

fn optimize_module(module: &mut RustModule, analysis: &mut CopyAnalysis) {
    let mut ctx = CopyContext::from_items(&module.items);
    ctx.infer_copy_types();
    ctx.apply_copy_derives(&mut module.items);
    ctx.rewrite_items(&mut module.items);

    analysis.copy_types.extend(ctx.copy_types.iter().cloned());

    for item in &mut module.items {
        if let Item::Mod(module) = item {
            optimize_module(module, analysis);
        }
    }
}

impl CopyContext {
    fn from_items(items: &[Item]) -> Self {
        let mut ctx = Self {
            copy_types: HashSet::from(["bool".to_string()]),
            type_defs: HashMap::new(),
            type_aliases: HashMap::new(),
            variant_owners: HashMap::new(),
            functions: HashMap::new(),
        };

        for item in items {
            ctx.collect_item(item);
        }

        ctx
    }

    fn collect_item(&mut self, item: &Item) {
        match item {
            Item::Struct(def) => {
                self.type_defs.insert(
                    def.name.clone(),
                    TypeDef {
                        kind: TypeDefKind::Struct(
                            def.fields
                                .iter()
                                .map(|field| FieldInfo {
                                    name: field.name.clone(),
                                    ty: field.ty.clone(),
                                })
                                .collect(),
                        ),
                    },
                );
            }
            Item::Enum(def) => {
                for variant in &def.variants {
                    self.insert_variant_owner(&variant.name, &def.name);
                }

                self.type_defs.insert(
                    def.name.clone(),
                    TypeDef {
                        kind: TypeDefKind::Enum(
                            def.variants
                                .iter()
                                .map(|variant| VariantInfo {
                                    name: variant.name.clone(),
                                    fields: variant.data.clone().unwrap_or_default(),
                                })
                                .collect(),
                        ),
                    },
                );
            }
            Item::TypeAlias(alias) => {
                self.type_aliases
                    .insert(alias.name.clone(), alias.target.clone());
            }
            Item::Function(function) => {
                self.functions
                    .insert(function.name.clone(), function.return_type.clone());
            }
            Item::Impl(impl_block) => {
                for item in &impl_block.items {
                    if let ImplItem::Method(method) = item {
                        self.functions
                            .insert(method.name.clone(), method.return_type.clone());
                    }
                }
            }
            _ => {}
        }
    }

    fn insert_variant_owner(&mut self, variant_name: &str, owner_name: &str) {
        self.variant_owners
            .entry(variant_name.to_string())
            .and_modify(|existing| {
                if existing.as_deref() != Some(owner_name) {
                    *existing = None;
                }
            })
            .or_insert_with(|| Some(owner_name.to_string()));
    }

    fn infer_copy_types(&mut self) {
        let mut changed = true;

        while changed {
            changed = false;

            for (name, target) in &self.type_aliases {
                if !self.copy_types.contains(name) && self.type_is_copy(target) {
                    self.copy_types.insert(name.clone());
                    changed = true;
                }
            }

            for (name, def) in &self.type_defs {
                if self.copy_types.contains(name) {
                    continue;
                }

                if self.type_def_is_copy(def) {
                    self.copy_types.insert(name.clone());
                    changed = true;
                }
            }
        }
    }

    fn type_def_is_copy(&self, def: &TypeDef) -> bool {
        match &def.kind {
            TypeDefKind::Struct(fields) => fields.iter().all(|field| self.type_is_copy(&field.ty)),
            TypeDefKind::Enum(variants) => variants
                .iter()
                .all(|variant| variant.fields.iter().all(|ty| self.type_is_copy(ty))),
        }
    }

    fn type_is_copy(&self, ty: &Type) -> bool {
        match ty {
            Type::Named(name) => self.copy_types.contains(name),
            Type::Path(path) => path
                .last()
                .is_some_and(|name| self.copy_types.contains(name)),
            Type::Generic(name, params) => {
                self.copy_types.contains(name)
                    && params.iter().all(|param| self.type_is_copy(param))
            }
            Type::Tuple(types) => types.iter().all(|ty| self.type_is_copy(ty)),
            Type::Array(inner, _) => self.type_is_copy(inner),
            Type::Unit | Type::Never => true,
            Type::Reference(_, _, _) | Type::Slice(_) => false,
        }
    }

    fn apply_copy_derives(&self, items: &mut [Item]) {
        for item in items {
            match item {
                Item::Struct(def) if self.copy_types.contains(&def.name) => {
                    ensure_clone_copy_derives(&mut def.derives);
                }
                Item::Enum(def) if self.copy_types.contains(&def.name) => {
                    ensure_clone_copy_derives(&mut def.derives);
                }
                _ => {}
            }
        }
    }

    fn rewrite_items(&self, items: &mut [Item]) {
        for item in items {
            self.rewrite_item(item);
        }
    }

    fn rewrite_item(&self, item: &mut Item) {
        match item {
            Item::Function(function) => self.rewrite_function(function, None),
            Item::Impl(impl_block) => {
                for item in &mut impl_block.items {
                    match item {
                        ImplItem::Method(method) => {
                            self.rewrite_function(method, Some(&impl_block.target));
                        }
                        ImplItem::AssocConst(_, _, expr) => {
                            self.rewrite_expr(expr, &mut TypeEnv::new());
                        }
                        ImplItem::AssocType(_, _) => {}
                    }
                }
            }
            Item::Const(const_def) => self.rewrite_expr(&mut const_def.value, &mut TypeEnv::new()),
            Item::LazyStatic(lazy_static) => {
                self.rewrite_block(&mut lazy_static.init, &mut TypeEnv::new());
            }
            _ => {}
        }
    }

    fn rewrite_function(&self, function: &mut FunctionDef, impl_target: Option<&Type>) {
        let mut env = TypeEnv::new();

        for param in &function.params {
            if param.name.is_empty() {
                continue;
            }

            let ty = if matches!(&param.ty, Type::Named(name) if name == "Self") {
                impl_target.cloned().unwrap_or_else(|| param.ty.clone())
            } else {
                param.ty.clone()
            };

            if param.name == "self" || param.name.ends_with("self") {
                env.insert("self".to_string(), ty);
            } else {
                env.insert(param.name.clone(), ty);
            }
        }

        self.rewrite_block(&mut function.body, &mut env);
    }

    fn rewrite_block(&self, block: &mut Block, env: &mut TypeEnv) {
        for stmt in &mut block.stmts {
            match stmt {
                Statement::Let(let_stmt) => {
                    if let Some(init) = &mut let_stmt.init {
                        self.rewrite_expr(init, env);
                    }

                    let inferred_ty = let_stmt.ty.clone().or_else(|| {
                        let_stmt
                            .init
                            .as_ref()
                            .and_then(|init| self.infer_expr_type(init, env))
                    });

                    if let Some(ty) = inferred_ty {
                        if is_binding_ident(&let_stmt.name) {
                            env.insert(let_stmt.name.clone(), ty);
                        } else {
                            self.bind_pattern_types(&let_stmt.name, &ty, env);
                        }
                    }
                }
                Statement::Expr(expr) => self.rewrite_expr(expr, env),
                Statement::Item(item) => self.rewrite_item(item),
                Statement::Continue | Statement::Break | Statement::Comment(_) => {}
            }
        }

        if let Some(expr) = &mut block.expr {
            self.rewrite_expr(expr, env);
        }
    }

    fn rewrite_expr(&self, expr: &mut Expr, env: &mut TypeEnv) {
        match expr {
            Expr::Tuple(items) => {
                for item in items {
                    self.rewrite_expr(item, env);
                }
            }
            Expr::Call(callee, args) => {
                self.rewrite_expr(callee, env);
                for arg in args {
                    self.rewrite_expr(arg, env);
                }
            }
            Expr::MethodCall(receiver, method, args) => {
                self.rewrite_expr(receiver, env);
                for arg in &mut *args {
                    self.rewrite_expr(arg, env);
                }

                if method == "clone"
                    && args.is_empty()
                    && self
                        .infer_expr_type(receiver, env)
                        .is_some_and(|ty| self.type_is_copy(&ty))
                {
                    *expr = receiver.as_ref().clone();
                }
            }
            Expr::Block(block) => {
                let mut block_env = env.clone();
                self.rewrite_block(block, &mut block_env);
            }
            Expr::Loop(block) => {
                let mut block_env = env.clone();
                self.rewrite_block(block, &mut block_env);
            }
            Expr::Closure(params, body) => {
                let mut closure_env = env.clone();
                for param in params {
                    closure_env.remove(param);
                }
                self.rewrite_expr(body, &mut closure_env);
            }
            Expr::Unsafe(block) => {
                let mut block_env = env.clone();
                self.rewrite_block(block, &mut block_env);
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.rewrite_expr(condition, env);

                let mut then_env = env.clone();
                self.rewrite_block(then_branch, &mut then_env);

                if let Some(else_branch) = else_branch {
                    let mut else_env = env.clone();
                    self.rewrite_block(else_branch, &mut else_env);
                }
            }
            Expr::IfLet {
                pattern,
                value,
                then_branch,
                else_branch,
            } => {
                self.rewrite_expr(value, env);

                let mut then_env = env.clone();
                if let Some(value_ty) = self.infer_expr_type(value, env) {
                    self.bind_pattern_types(pattern, &value_ty, &mut then_env);
                }
                self.rewrite_block(then_branch, &mut then_env);

                if let Some(else_branch) = else_branch {
                    let mut else_env = env.clone();
                    self.rewrite_block(else_branch, &mut else_env);
                }
            }
            Expr::Match { expr, arms } => {
                self.rewrite_expr(expr, env);
                let scrutinee_ty = self.infer_expr_type(expr, env);

                for arm in arms {
                    let mut arm_env = env.clone();

                    if let Some(ty) = &scrutinee_ty {
                        self.bind_pattern_types(&arm.pattern, ty, &mut arm_env);
                    }
                    if let Some(guard) = &mut arm.guard {
                        self.rewrite_expr(guard, &mut arm_env);
                    }

                    self.rewrite_block(&mut arm.body, &mut arm_env);
                }
            }
            Expr::Reference(inner, _, _)
            | Expr::Await(inner)
            | Expr::UnaryOp(_, inner)
            | Expr::Parenthesized(inner) => self.rewrite_expr(inner, env),
            Expr::BinaryOp(left, _, right)
            | Expr::Assign(left, right)
            | Expr::Index(left, right) => {
                self.rewrite_expr(left, env);
                self.rewrite_expr(right, env);
            }
            Expr::BuilderChain(methods) => {
                for method in methods {
                    if let BuilderMethod::Spawn { closure, .. } = method {
                        self.rewrite_expr(closure, env);
                    }
                }
            }
            Expr::Ident(_) | Expr::Path(_, _) | Expr::Literal(_) => {}
        }
    }

    fn infer_expr_type(&self, expr: &Expr, env: &TypeEnv) -> Option<Type> {
        match expr {
            Expr::Ident(name) => env.get(name).cloned(),
            Expr::Path(path, PathType::Namespace) => self
                .owner_for_variant_path(path)
                .map(Type::Named)
                .or_else(|| {
                    path.last()
                        .and_then(|name| self.functions.get(name).cloned())
                }),
            Expr::Path(path, PathType::Member) => self.infer_member_path_type(path, env),
            Expr::Literal(Literal::Bool(_)) => Some(Type::Named("bool".to_string())),
            Expr::Literal(_) => None,
            Expr::Tuple(items) => {
                let mut types = Vec::new();
                for item in items {
                    types.push(self.infer_expr_type(item, env)?);
                }

                if types.is_empty() {
                    Some(Type::Unit)
                } else {
                    Some(Type::Tuple(types))
                }
            }
            Expr::Call(callee, _) => self.infer_call_type(callee),
            Expr::MethodCall(receiver, method, args) if method == "clone" && args.is_empty() => {
                self.infer_expr_type(receiver, env)
            }
            Expr::Reference(inner, is_reference, mutable) => self
                .infer_expr_type(inner, env)
                .map(|ty| Type::Reference(Box::new(ty), *is_reference, *mutable)),
            Expr::Parenthesized(inner) => self.infer_expr_type(inner, env),
            Expr::Block(block) => block
                .expr
                .as_ref()
                .and_then(|expr| self.infer_expr_type(expr, env)),
            Expr::BinaryOp(_, op, _) if binary_op_returns_bool(op) => {
                Some(Type::Named("bool".to_string()))
            }
            _ => None,
        }
    }

    fn infer_call_type(&self, callee: &Expr) -> Option<Type> {
        match callee {
            Expr::Ident(name) => self
                .owner_for_variant_name(name)
                .map(Type::Named)
                .or_else(|| self.functions.get(name).cloned()),
            Expr::Path(path, PathType::Namespace) => self
                .owner_for_variant_path(path)
                .map(Type::Named)
                .or_else(|| {
                    path.last()
                        .and_then(|name| self.functions.get(name).cloned())
                }),
            Expr::Parenthesized(inner) => self.infer_call_type(inner),
            _ => None,
        }
    }

    fn infer_member_path_type(&self, path: &[String], env: &TypeEnv) -> Option<Type> {
        let (head, tail) = path.split_first()?;
        let mut current_ty = env.get(head)?.clone();

        for member in tail {
            current_ty = self.field_type(&current_ty, member)?;
        }

        Some(current_ty)
    }

    fn field_type(&self, ty: &Type, member: &str) -> Option<Type> {
        let type_name = local_type_name(ty)?;
        let def = self.type_defs.get(type_name)?;

        match &def.kind {
            TypeDefKind::Struct(fields) => fields.iter().find_map(|field| {
                if field.name == member {
                    Some(field.ty.clone())
                } else {
                    None
                }
            }),
            TypeDefKind::Enum(_) => None,
        }
    }

    fn bind_pattern_types(&self, pattern: &str, expected: &Type, env: &mut TypeEnv) {
        let pattern = pattern.trim();
        if pattern.is_empty() || pattern == "_" || pattern == ".." {
            return;
        }

        if let Some(inner) = strip_prefix_word(pattern, "box") {
            let inner_ty = match expected {
                Type::Generic(name, params) if name == "Box" && params.len() == 1 => &params[0],
                _ => expected,
            };
            self.bind_pattern_types(inner, inner_ty, env);
            return;
        }

        let pattern = strip_binding_modifiers(pattern);

        if let Some(inner) = outer_parens_inner(pattern) {
            let parts = split_top_level_commas(inner);
            if parts.len() > 1 {
                if let Type::Tuple(types) = expected {
                    for (part, ty) in parts.iter().zip(types) {
                        self.bind_pattern_types(part, ty, env);
                    }
                }
                return;
            }
        }

        if let Some((constructor, args)) = split_constructor_pattern(pattern) {
            if let Some(field_types) = self.pattern_payload_types(constructor, expected) {
                for (arg, ty) in args.iter().zip(field_types.iter()) {
                    self.bind_pattern_types(arg, ty, env);
                }
            }
            return;
        }

        if pattern.contains("::") || matches!(pattern, "true" | "false") {
            return;
        }

        if is_binding_ident(pattern) {
            env.insert(pattern.to_string(), expected.clone());
        }
    }

    fn pattern_payload_types(&self, constructor: &str, expected: &Type) -> Option<Vec<Type>> {
        let variant_name = constructor
            .rsplit("::")
            .next()
            .unwrap_or(constructor)
            .trim();

        if let Some(expected_name) = local_type_name(expected) {
            if let Some(def) = self.type_defs.get(expected_name) {
                match &def.kind {
                    TypeDefKind::Enum(variants) => {
                        if let Some(variant) =
                            variants.iter().find(|variant| variant.name == variant_name)
                        {
                            return Some(variant.fields.clone());
                        }
                    }
                    TypeDefKind::Struct(fields)
                        if variant_name == expected_name || constructor.trim() == expected_name =>
                    {
                        return Some(fields.iter().map(|field| field.ty.clone()).collect());
                    }
                    TypeDefKind::Struct(_) => {}
                }
            }
        }

        let owner = self.owner_for_constructor(constructor)?;
        let def = self.type_defs.get(&owner)?;
        match &def.kind {
            TypeDefKind::Enum(variants) => variants
                .iter()
                .find(|variant| variant.name == variant_name)
                .map(|variant| variant.fields.clone()),
            TypeDefKind::Struct(fields) => {
                Some(fields.iter().map(|field| field.ty.clone()).collect())
            }
        }
    }

    fn owner_for_constructor(&self, constructor: &str) -> Option<String> {
        let parts = constructor
            .split("::")
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();

        if parts.len() >= 2 {
            let owner = parts[parts.len() - 2];
            if self.type_defs.contains_key(owner) {
                return Some(owner.to_string());
            }
        }

        parts
            .last()
            .and_then(|variant_name| self.owner_for_variant_name(variant_name))
    }

    fn owner_for_variant_path(&self, path: &[String]) -> Option<String> {
        if path.len() >= 2 {
            let owner = &path[path.len() - 2];
            let variant_name = path.last()?;

            if self
                .type_defs
                .get(owner)
                .is_some_and(|def| match &def.kind {
                    TypeDefKind::Enum(variants) => {
                        variants.iter().any(|variant| &variant.name == variant_name)
                    }
                    TypeDefKind::Struct(_) => false,
                })
            {
                return Some(owner.clone());
            }
        }

        path.last()
            .and_then(|variant_name| self.owner_for_variant_name(variant_name))
    }

    fn owner_for_variant_name(&self, variant_name: &str) -> Option<String> {
        self.variant_owners.get(variant_name).cloned().flatten()
    }
}

fn ensure_clone_copy_derives(derives: &mut Vec<String>) {
    if !derives.iter().any(|derive| derive == "Clone") {
        derives.push("Clone".to_string());
    }

    if derives.iter().any(|derive| derive == "Copy") {
        return;
    }

    let insert_at = derives
        .iter()
        .position(|derive| derive == "Clone")
        .map_or(derives.len(), |idx| idx + 1);
    derives.insert(insert_at, "Copy".to_string());
}

fn local_type_name(ty: &Type) -> Option<&str> {
    match ty {
        Type::Named(name) => Some(name.as_str()),
        Type::Path(path) => path.last().map(String::as_str),
        Type::Generic(name, _) => Some(name.as_str()),
        _ => None,
    }
}

fn is_binding_ident(input: &str) -> bool {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && input != "_"
        && !is_reserved_pattern_word(input)
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_reserved_pattern_word(input: &str) -> bool {
    matches!(input, "box" | "false" | "mut" | "ref" | "self" | "true")
}

fn binary_op_returns_bool(op: &str) -> bool {
    matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||")
}

fn strip_prefix_word<'a>(input: &'a str, word: &str) -> Option<&'a str> {
    let rest = input.strip_prefix(word)?;
    if rest.starts_with(char::is_whitespace) {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn strip_binding_modifiers(mut input: &str) -> &str {
    loop {
        let trimmed = input.trim_start();
        if let Some(rest) = strip_prefix_word(trimmed, "ref") {
            input = rest;
        } else if let Some(rest) = strip_prefix_word(trimmed, "mut") {
            input = rest;
        } else {
            return trimmed;
        }
    }
}

fn outer_parens_inner(input: &str) -> Option<&str> {
    if !input.starts_with('(') || !input.ends_with(')') {
        return None;
    }

    let mut depth = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 && idx != input.len() - 1 {
                    return None;
                }
            }
            _ => {}
        }
    }

    if depth == 0 {
        input.strip_prefix('(')?.strip_suffix(')')
    } else {
        None
    }
}

fn split_constructor_pattern(input: &str) -> Option<(&str, Vec<String>)> {
    let mut depth = 0usize;
    let mut start = None;

    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth += 1;
            }
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 && idx != input.len() - 1 {
                    return None;
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        return None;
    }

    let start = start?;
    let constructor = input[..start].trim();
    if constructor.is_empty() {
        return None;
    }

    let inner = input[start + 1..input.len() - 1].trim();
    Some((constructor, split_top_level_commas(inner)))
}

fn split_top_level_commas(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;

    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                parts.push(input[start..idx].trim().to_string());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    let last = input[start..].trim();
    if !last.is_empty() {
        parts.push(last.to_string());
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::optimize_copy;
    use crate::rustlight_parser::parse_rust_source;
    use crate::rustlight_print::RustCodeGenerator;

    fn optimize_source(source: &str) -> String {
        let mut module = parse_rust_source(source, "CopyTest").expect("source parses");
        optimize_copy(&mut module);

        let mut generator = RustCodeGenerator::new();
        generator.generate_module_code(&module)
    }

    #[test]
    fn derives_copy_and_removes_bool_clones() {
        let printed = optimize_source(
            r#"
#[derive(Clone)]
pub enum FlagPair {
    FlagPair(bool, bool),
}

pub fn swap_flag_pair(x0: FlagPair) -> FlagPair {
    match x0 {
        FlagPair::FlagPair(x, y) => FlagPair::FlagPair(y.clone(), x.clone()),
    }
}
"#,
        );

        assert!(printed.contains("#[derive(Clone, Copy)]"));
        assert!(printed.contains("FlagPair::FlagPair(y, x)"));
        assert!(!printed.contains(".clone()"));
    }

    #[test]
    fn propagates_copy_through_user_defined_fields() {
        let printed = optimize_source(
            r#"
#[derive(Clone)]
pub enum Color {
    Red,
    Green,
    Blue,
}

#[derive(Clone)]
pub enum Pixel {
    Pixel(Color, Color, Color),
}

pub fn rotate_pixel(x0: Pixel) -> Pixel {
    match x0 {
        Pixel::Pixel(r, g, b) => Pixel::Pixel(g.clone(), b.clone(), r.clone()),
    }
}
"#,
        );

        assert!(printed.contains("pub enum Color"));
        assert!(printed.contains("pub enum Pixel"));
        assert_eq!(printed.matches("#[derive(Clone, Copy)]").count(), 2);
        assert!(printed.contains("Pixel::Pixel(g, b, r)"));
    }

    #[test]
    fn binds_tuple_match_patterns() {
        let printed = optimize_source(
            r#"
#[derive(Clone)]
pub enum Color {
    Red,
    Green,
}

#[derive(Clone)]
pub enum Pixel {
    Pixel(Color, Color),
}

pub fn replace_first_color(x0: Pixel, c: Color) -> Pixel {
    match (x0, c) {
        (Pixel::Pixel(_, old), c) => Pixel::Pixel(c.clone(), old.clone()),
    }
}
"#,
        );

        assert!(printed.contains("match (x0, c)"));
        assert!(printed.contains("Pixel::Pixel(c, old)"));
        assert!(!printed.contains(".clone()"));
    }

    #[test]
    fn keeps_clones_for_non_copy_recursive_box_types() {
        let printed = optimize_source(
            r#"
#[derive(Clone)]
pub enum Option {
    None,
    Some(Box<Option>),
}

pub fn get(x0: Option) -> Box<Option> {
    match x0 {
        Option::Some(x) => x.clone(),
        Option::None => Box::new(Option::None),
    }
}
"#,
        );

        assert!(printed.contains("#[derive(Clone)]"));
        assert!(printed.contains("x.clone()"));
        assert!(!printed.contains("#[derive(Clone, Copy)]"));
    }
}
