#![allow(clippy::only_used_in_recursion)]
use crate::rustlight_ast::*;

mod pattern;

const MAX_FUNCTION_SIGNATURE_WIDTH: usize = 80;

// Rust code generator
pub struct RustCodeGenerator {
    buffer: String,
    indent_level: usize,
}

impl Default for RustCodeGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl RustCodeGenerator {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            indent_level: 0,
        }
    }

    // Main entry: generate full module code
    pub fn generate_module_code(&mut self, module: &RustModule) -> String {
        self.buffer.clear();

        for doc in &module.docs {
            self.writeln(doc);
        }
        for attr in &module.attrs {
            self.generate_attribute(attr);
        }
        if !module.docs.is_empty() || !module.attrs.is_empty() {
            self.writeln("");
        }

        // Generate module contents
        self.generate_items(&module.items);

        self.buffer.clone()
    }

    // Generate multiple items
    fn generate_items(&mut self, items: &[Item]) {
        for (idx, item) in items.iter().enumerate() {
            self.generate_item(item);
            if should_separate_after_use(item, items.get(idx + 1)) {
                self.writeln("");
            }
        }
    }

    // Generate a single item
    fn generate_item(&mut self, item: &Item) {
        match item {
            Item::Raw(raw) => self.generate_raw(raw),
            Item::Struct(s) => self.generate_struct(s),
            Item::Enum(e) => self.generate_enum(e),
            Item::Union(u) => self.generate_union(u), // New
            Item::Function(f) => self.generate_function(f),
            Item::Impl(i) => self.generate_impl(i),
            Item::Const(c) => self.generate_const(c),
            Item::TypeAlias(t) => self.generate_type_alias(t),
            Item::Use(u) => self.generate_use(u),
            Item::Mod(m) => self.generate_nested_module(m),
            Item::ModuleDecl(module) => {
                for attr in &module.attrs {
                    self.writeln(attr);
                }
                self.writeln(&format!("{}mod {};", self.visibility(&module.vis), module.name));
            }
            Item::LazyStatic(l) => self.generate_lazy_static(l),
        }
    }

    fn generate_raw(&mut self, raw: &str) {
        if !raw.is_empty() {
            // Reindenting the contents would change multiline string literals.
            self.writeln(raw.strip_suffix('\n').unwrap_or(raw));
        }
    }

    fn generate_nested_module(&mut self, m: &RustModule) {
        // Generate the module declaration line
        match &m.vis {
            Visibility::Public => self.write("pub "),
            Visibility::Private => (), // Do not add a modifier for private modules
            Visibility::Restricted(paths) => self.write(&format!("pub(in {} ) ", paths.join("::"))),
            Visibility::None => (),
        }

        self.writeln(&format!("mod {} {{", m.name));
        self.indent();

        // Module-level docs and attributes
        for doc in &m.docs {
            self.writeln(doc);
        }
        for attr in &m.attrs {
            self.generate_attribute(attr);
        }

        // Module contents
        self.generate_items(&m.items);

        self.dedent();
        self.writeln("}");
        self.writeln("");
    }

    fn generate_struct(&mut self, s: &StructDef) {
        // Documentation comments
        for doc in &s.docs {
            self.writeln(doc);
        }

        // Derive attributes
        if !s.derives.is_empty() {
            self.write("#[derive(");
            for (i, derive) in s.derives.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.write(derive);
            }
            self.writeln(")]");
        }

        let tuple_struct =
            !s.fields.is_empty() && s.fields.iter().all(|field| field.name.is_empty());

        // Struct definition
        self.write(&format!("{}struct {} ", self.visibility(&s.vis), s.name));

        self.write(&self.generic_params_to_string(&s.generics.params));

        if tuple_struct {
            self.write("(");
            for (i, field) in s.fields.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.write(&self.visibility(&field.vis));
                self.write(&self.type_to_string(&field.ty));
            }
            self.write(")");
            self.write_where_predicates(&s.generics.where_predicates);
            self.writeln(";");
            self.writeln("");
            return;
        }

        self.write_where_predicates(&s.generics.where_predicates);
        self.writeln(" {");

        self.indent();

        for field in &s.fields {
            self.generate_field(field);
        }

        self.dedent();
        self.writeln("}");
        self.writeln("");
    }

    fn generate_field(&mut self, field: &Field) {
        for doc in &field.docs {
            self.writeln(doc);
        }
        for attr in &field.attrs {
            self.generate_attribute(attr);
        }
        self.writeln(&format!(
            "{}{}: {},",
            self.visibility(&field.vis),
            field.name,
            self.type_to_string(&field.ty)
        ));
    }

    fn generate_impl(&mut self, i: &ImplBlock) {
        self.write("impl");

        self.write(&self.generic_params_to_string(&i.generics.params));

        // Trait implementation
        if let Some(trait_ty) = &i.trait_impl {
            self.write(&format!(" {} for", self.type_to_string(trait_ty)));
        }
        // Target type
        self.write(&format!(" {}", self.type_to_string(&i.target)));
        self.write_where_predicates(&i.generics.where_predicates);

        self.writeln(" {");
        self.indent();

        for item in &i.items {
            match item {
                ImplItem::Method(m) => self.generate_function(m),
                ImplItem::AssocConst(name, ty, expr) => {
                    self.writeln(&format!("const {}: {} = ", name, self.type_to_string(ty)));
                    self.generate_expr(expr);
                    self.writeln(";");
                }
                ImplItem::AssocType(name, ty) => {
                    self.writeln(&format!("type {} = {};", name, self.type_to_string(ty)));
                }
            }
        }

        self.dedent();
        self.writeln("}");
        self.writeln("");
    }

    fn generate_function(&mut self, f: &FunctionDef) {
        // Documentation comments
        for doc in &f.docs {
            self.writeln(doc);
        }

        // Attributes
        for attr in &f.attrs {
            self.generate_attribute(attr);
        }

        let head = format!(
            "{}{}fn {}{}",
            self.visibility(&f.vis),
            if f.asyncness { "async " } else { "" },
            f.name,
            self.generic_names_to_string(&f.generics.params)
        );
        let params_and_return = format!(
            "({}) -> {}",
            self.params_to_string(&f.params),
            self.type_to_string(&f.return_type)
        );
        let signature = format!("{}{}", head, params_and_return);
        let where_clauses = self.function_where_clauses(&f.generics);
        if !where_clauses.is_empty() {
            if signature.len() <= MAX_FUNCTION_SIGNATURE_WIDTH {
                self.writeln(&signature);
            } else {
                self.writeln(&head);
                self.indent();
                self.writeln(&params_and_return);
                self.dedent();
            }
            self.writeln("where");
            self.indent();
            for (position, clause) in where_clauses.iter().enumerate() {
                let comma = if position + 1 < where_clauses.len() { "," } else { "" };
                self.writeln(&format!("{clause}{comma}"));
            }
            self.dedent();
            self.writeln("{");
        } else if signature.len() <= MAX_FUNCTION_SIGNATURE_WIDTH {
            self.writeln(&format!("{signature} {{"));
        } else {
            self.writeln(&head);
            self.indent();
            self.writeln(&format!("{params_and_return} {{"));
            self.dedent();
        }

        self.indent();
        self.generate_block(&f.body);
        self.dedent();
        self.writeln("}");
        self.writeln("");
    }

    fn generic_names_to_string(&self, generics: &[GenericParam]) -> String {
        if generics.is_empty() {
            return String::new();
        }

        let mut out = String::from("<");
        for (i, generic) in generics.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            match generic {
                GenericParam::Type(parameter) => out.push_str(&parameter.name),
                GenericParam::Lifetime(_) => out.push_str(&self.generic_param_to_string(generic)),
            }
        }
        out.push('>');
        out
    }

    fn generic_param_to_string(&self, generic: &GenericParam) -> String {
        bounded_name(generic.name(), generic.bounds())
    }

    fn params_to_string(&self, params: &[Param]) -> String {
        params
            .iter()
            .map(|param| self.param_to_string(param))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn param_to_string(&self, param: &Param) -> String {
        if param.name.is_empty() {
            self.type_to_string(&param.ty)
        } else {
            let modifier = if param.mutable { "mut " } else { "" };
            format!("{modifier}{}: {}", param.name, self.type_to_string(&param.ty))
        }
    }

    fn function_where_clauses(&self, generics: &Generics) -> Vec<String> {
        let mut bounded_generics = generics
            .params
            .iter()
            .filter_map(GenericParam::type_parameter)
            .filter(|generic| !generic.bounds.is_empty())
            .collect::<Vec<_>>();
        bounded_generics.sort_by(|left, right| left.name.cmp(&right.name));
        bounded_generics
            .into_iter()
            .map(|param| format!("{}: {}", param.name, ordered_bounds_to_string(&param.bounds)))
            .chain(
                generics
                    .where_predicates
                    .iter()
                    .map(|predicate| self.where_predicate_to_string(predicate)),
            )
            .collect()
    }

    fn where_predicate_to_string(&self, predicate: &WherePredicate) -> String {
        let (subject, bounds) = match predicate {
            WherePredicate::Type { subject, bounds } => (self.type_to_string(subject), bounds),
            WherePredicate::Lifetime { subject, bounds } => (subject.clone(), bounds),
        };
        format!("{subject}: {}", ordered_bounds_to_string(bounds))
    }

    fn write_where_predicates(&mut self, predicates: &[WherePredicate]) {
        if !predicates.is_empty() {
            self.write(" where ");
            for (position, predicate) in predicates.iter().enumerate() {
                if position > 0 {
                    self.write(", ");
                }
                self.write(&self.where_predicate_to_string(predicate));
            }
        }
    }

    fn generate_block(&mut self, block: &Block) {
        for stmt in &block.stmts {
            self.generate_statement(stmt);
        }

        if let Some(expr) = &block.expr {
            self.generate_expr(expr);
            self.writeln("");
        }
    }

    // Dedicated method for generating match arm bodies
    fn generate_match_arm_body(&mut self, block: &Block) {
        for stmt in &block.stmts {
            self.generate_statement(stmt);
        }

        if let Some(expr) = &block.expr {
            self.generate_expr(expr);
            self.writeln("");
            // The last expression in a match arm should never end with a semicolon, since it is the return value
        }
    }

    fn generate_statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Let(ls) => {
                for attr in &ls.attrs {
                    self.writeln(attr);
                }
                self.write(&format!(
                    "{} {}",
                    if ls.ifmut { "let mut" } else { "let" },
                    ls.name
                ));
                if let Some(ty) = &ls.ty {
                    self.write(&format!(": {}", self.type_to_string(ty)));
                }
                if let Some(init) = &ls.init {
                    self.write(" = ");
                    self.generate_expr(init);
                }
                self.writeln(";");
            }
            Statement::LetElse {
                attrs,
                pattern,
                value,
                else_block,
            } => {
                for attr in attrs {
                    self.writeln(attr);
                }
                self.write("let ");
                self.generate_pattern(pattern);
                self.write(" = ");
                self.generate_expr(value);
                self.writeln(" else {");
                self.indent();
                self.generate_block(else_block);
                self.dedent();
                self.writeln("};");
            }
            Statement::Expr(expr) => {
                self.generate_expr(expr);
                self.writeln(";");
            }
            Statement::Macro(statement) => {
                for attr in &statement.attrs {
                    self.writeln(attr);
                }
                let (open, close) = match statement.delimiter {
                    crate::MacroDelimiter::Parenthesis => ("(", ")"),
                    crate::MacroDelimiter::Brace => ("{", "}"),
                    crate::MacroDelimiter::Bracket => ("[", "]"),
                };
                self.write(&statement.path);
                self.write("!");
                self.write(open);
                self.write(&statement.tokens);
                self.write(close);
                self.writeln(if statement.semicolon { ";" } else { "" });
            }
            Statement::Item(item) => self.generate_item(item),
            Statement::Comment(comment) => {
                self.writeln(&format!("// {}", comment));
            }
        }
    }

    fn generate_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(id) => self.write(id),
            Expr::Macro(source) => self.write(source),
            Expr::Path(path, PathType::Namespace) => self.generate_path(path),
            Expr::Field { base, member } => {
                self.generate_postfix_base(base);
                self.write(".");
                self.generate_member(member);
            }
            Expr::Record { path, fields, rest } => {
                self.generate_path(path);
                self.write(" {");
                for (index, field) in fields.iter().enumerate() {
                    self.write(if index == 0 { " " } else { ", " });
                    self.generate_member(&field.member);
                    let shorthand = field.shorthand
                        && matches!(
                            (&field.member, &field.value),
                            (Member::Named(name), Expr::Ident(value)) if name == value
                        );
                    if !shorthand {
                        self.write(": ");
                        self.generate_expr(&field.value);
                    }
                }
                if let Some(rest) = rest {
                    self.write(if fields.is_empty() { " .." } else { ", .." });
                    self.generate_expr(rest);
                }
                self.write(if fields.is_empty() && rest.is_none() { "}" } else { " }" });
            }
            Expr::Literal(lit) => self.generate_literal(lit),
            Expr::Array(items) => {
                self.write("[");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.generate_expr(item);
                }
                self.write("]");
            }
            Expr::Repeat { value, len } => {
                self.write("[");
                self.generate_expr(value);
                self.write("; ");
                self.generate_expr(len);
                self.write("]");
            }
            Expr::Range { start, end } => {
                if let Some(start) = start {
                    self.generate_expr(start);
                }
                self.write(match end {
                    RangeEnd::Exclusive(_) => "..",
                    RangeEnd::Inclusive(_) => "..=",
                });
                if let Some(end) = end.expression() {
                    self.generate_expr(end);
                }
            }
            Expr::Try(operand) => {
                self.generate_postfix_base(operand);
                self.write("?");
            }
            Expr::Tuple(items) => {
                self.write("(");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.generate_expr(item);
                }
                if items.len() == 1 {
                    self.write(",");
                }
                self.write(")");
            }
            Expr::Call(callee, args) => {
                self.generate_expr(callee);
                self.write("(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.generate_call_argument(arg);
                }
                self.write(")");
            }
            Expr::MethodCall(receiver, method, args) => {
                self.generate_expr(receiver);
                if !method.is_empty() {
                    self.write(&format!(".{}", method));
                }
                self.write("(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.generate_call_argument(arg);
                }
                self.write(")");
            }
            Expr::Block(block) => {
                self.writeln("{");
                self.indent();
                self.generate_block(block);
                self.dedent();
                self.write("}");
            }
            Expr::Loop(block) => {
                self.writeln("loop {");
                self.indent();
                self.generate_block(block);
                self.dedent();
                self.write("}");
            }
            Expr::While { condition, body } => {
                self.write("while ");
                self.generate_expr(condition);
                self.writeln(" {");
                self.indent();
                self.generate_block(body);
                self.dedent();
                self.write("}");
            }
            Expr::For {
                pattern,
                iter,
                body,
            } => {
                self.write("for ");
                self.generate_pattern(pattern);
                self.write(" in ");
                self.generate_expr(iter);
                self.writeln(" {");
                self.indent();
                self.generate_block(body);
                self.dedent();
                self.write("}");
            }
            Expr::Let { pattern, value } => {
                self.write("let ");
                self.generate_pattern(pattern);
                self.write(" = ");
                self.generate_expr(value);
            }
            Expr::Return(value) => self.generate_jump_expr("return", value.as_deref()),
            Expr::Break(value) => self.generate_jump_expr("break", value.as_deref()),
            Expr::Continue => self.write("continue"),
            Expr::Await(expr) => {
                self.generate_expr(expr);
                self.write(".await");
            }
            // The call chain for creating threads inside a process is currently hard-coded
            Expr::BuilderChain(methods) => {
                self.writeln("thread::Builder::new()");
                for method in methods {
                    match method {
                        BuilderMethod::Named(name) => {
                            self.writeln(&format!("    .name({})", name));
                        }
                        // BuilderMethod::StackSize(expr) => {
                        //     self.write("    .stack_size(");
                        //     self.generate_expr(expr);
                        //     self.writeln(" as usize)");
                        // },
                        BuilderMethod::Spawn { closure, move_kw } => {
                            self.write("    .spawn(");
                            if *move_kw {
                                self.write("move ");
                            }
                            self.generate_expr(closure);
                            self.write(")");
                        }
                    }
                }
            }
            Expr::Closure(params, body, is_move) => {
                if *is_move {
                    self.write("move ");
                }
                self.write("|");
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.generate_closure_param(param);
                }
                self.write("| ");
                match body.as_ref() {
                    Expr::Block(_) => self.generate_expr(body),
                    _ => {
                        self.write("{ ");
                        self.generate_expr(body);
                        self.write(" }");
                    }
                }
            }
            Expr::TypedClosure(params, return_type, body, is_move) => {
                if *is_move {
                    self.write("move ");
                }
                self.write("|");
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.generate_closure_param(param);
                }
                self.write("| -> ");
                self.write(&self.type_to_string(return_type));
                self.write(" ");
                match body.as_ref() {
                    Expr::Block(_) => self.generate_expr(body),
                    _ => {
                        self.write("{ ");
                        self.generate_expr(body);
                        self.write(" }");
                    }
                }
            }
            Expr::Match { expr, arms } => {
                self.write("match ");
                self.generate_expr(expr);
                self.writeln(" {");
                self.indent();
                for arm in arms {
                    self.generate_pattern(&arm.pattern);
                    if let Some(guard) = &arm.guard {
                        self.write(" if ");
                        self.generate_expr(guard);
                    }
                    self.writeln(" => {");
                    self.indent();
                    // Generate arm body, but do not add a semicolon to the final expression
                    self.generate_match_arm_body(&arm.body);
                    self.dedent();
                    self.writeln("},");
                }
                self.dedent();
                self.write("}");
            }
            Expr::Unsafe(block) => {
                self.write("unsafe ");
                // Choose formatting strategy based on block contents
                if block.stmts.len() == 1 && block.expr.is_none() {
                    // Compact formatting for a single-statement unsafe block
                    self.write("{ ");
                    self.generate_block(block);
                    self.write(" }");
                } else {
                    // Expanded formatting for a multi-statement unsafe block
                    self.writeln("{");
                    self.indent();
                    self.generate_block(block);
                    self.dedent();
                    self.write("}");
                }
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.write("if ");
                self.generate_expr(condition);
                self.write(" ");
                self.writeln("{");
                self.indent();
                self.generate_block(then_branch);
                self.dedent();
                self.write("}");

                if let Some(else_branch) = else_branch {
                    if else_branch.stmts.is_empty()
                        && matches!(else_branch.expr.as_deref(), Some(Expr::If { .. }))
                    {
                        self.write(" else ");
                        self.generate_expr(
                            else_branch
                                .expr
                                .as_deref()
                                .expect("checked nested if expression"),
                        );
                    } else {
                        self.write(" else ");
                        self.writeln("{");
                        self.indent();
                        self.generate_block(else_branch);
                        self.dedent();
                        self.write("}");
                    }
                }
            }
            Expr::IfLet {
                pattern,
                value,
                then_branch,
                else_branch,
            } => {
                self.write("if let ");
                self.write(pattern);
                self.write(" = ");
                self.generate_expr(value);
                self.write(" {\n");
                self.indent();
                self.generate_block(then_branch);
                self.dedent();
                self.write("}");

                if let Some(else_branch) = else_branch {
                    self.write(" else {\n");
                    self.indent();
                    self.generate_block(else_branch);
                    self.dedent();
                    self.write("}");
                }
            }
            Expr::Reference(inner_expr, is_reference, mutable) => {
                if *is_reference {
                    self.write("&");
                }
                if *mutable {
                    self.write("mut ");
                }
                if matches!(
                    inner_expr.as_ref(),
                    Expr::BinaryOp(_, _, _)
                        | Expr::Assign(_, _)
                        | Expr::Cast(_, _)
                        | Expr::Range { .. }
                ) {
                    self.write("(");
                    self.generate_expr(inner_expr);
                    self.write(")");
                } else {
                    self.generate_expr(inner_expr);
                }
            }
            Expr::BinaryOp(left, op, right) => {
                self.generate_expr(left);
                self.write(" ");
                self.write(op);
                self.write(" ");
                self.generate_expr(right);
            }
            Expr::Assign(left, right) => {
                self.generate_expr(left);
                self.write(" = ");
                self.generate_expr(right);
            }
            Expr::UnaryOp(op, expr) => {
                self.write(op);
                self.generate_expr(expr);
            }
            Expr::Index(array, index) => {
                self.generate_expr(array);
                self.write("[");
                self.generate_expr(index);
                self.write("]");
            }
            Expr::Parenthesized(expr) => {
                if matches!(expr.as_ref(), Expr::Cast(_, _)) {
                    self.generate_expr(expr);
                } else {
                    self.write("(");
                    self.generate_expr(expr);
                    self.write(")");
                }
            }
            Expr::Cast(expr, ty) => {
                self.generate_cast_expr(expr, ty, true);
            }
        }
    }

    fn generate_jump_expr(&mut self, keyword: &str, value: Option<&Expr>) {
        self.write(keyword);
        if let Some(value) = value {
            self.write(" ");
            self.generate_expr(value);
        }
    }

    fn generate_postfix_base(&mut self, base: &Expr) {
        let needs_parens = matches!(
            base,
            Expr::Assign(..)
                | Expr::BinaryOp(..)
                | Expr::Closure(..)
                | Expr::TypedClosure(..)
                | Expr::Return(_)
                | Expr::Break(_)
                | Expr::Reference(..)
                | Expr::UnaryOp(..)
                | Expr::Range { .. }
        );
        if needs_parens {
            self.write("(");
        }
        self.generate_expr(base);
        if needs_parens {
            self.write(")");
        }
    }

    fn generate_path(&mut self, path: &[String]) {
        for (index, segment) in path.iter().enumerate() {
            if index != 0 {
                self.write("::");
            }
            self.write(segment);
        }
    }

    fn generate_member(&mut self, member: &Member) {
        match member {
            Member::Named(name) => self.write(name),
            Member::Index(index) => self.write(&index.to_string()),
        }
    }

    fn generate_call_argument(&mut self, expr: &Expr) {
        // The call delimiters already delimit a complete argument expression,
        // so an explicit outer `Parenthesized` wrapper is redundant here.
        // Keep parentheses owned by the expression itself (for example, a
        // tuple still prints as `(a, b)`) while avoiding shapes such as
        // `f(({ ... }))` and `f((-x))`.
        let expr = Self::strip_parenthesized(expr);
        if let Expr::Cast(cast_expr, ty) = expr {
            self.generate_cast_expr(cast_expr, ty, false);
        } else {
            self.generate_expr(expr);
        }
    }

    fn strip_parenthesized(expr: &Expr) -> &Expr {
        match expr {
            Expr::Parenthesized(inner) => Self::strip_parenthesized(inner),
            _ => expr,
        }
    }

    fn generate_cast_expr(&mut self, expr: &Expr, ty: &Type, parenthesized: bool) {
        if parenthesized {
            self.write("(");
        }
        self.generate_expr(expr);
        self.write(" as ");
        self.write(&self.type_to_string(ty));
        if parenthesized {
            self.write(")");
        }
    }

    fn generate_literal(&mut self, lit: &Literal) {
        match lit {
            Literal::Raw(source) => self.write(source),
            Literal::Int(i) => self.write(&i.to_string()),
            Literal::Float(f) => self.write(&f.to_string()),
            Literal::Str(s) => self.write(&format!("\"{}\"", s)),
            Literal::Bool(b) => self.write(&b.to_string()),
            Literal::Char(c) => self.write(&format!("'{}'", c)),
        }
    }

    fn generate_type_alias(&mut self, t: &TypeAlias) {
        for doc in &t.docs {
            self.writeln(doc);
        }
        self.write(&format!(
            "{}type {}{}",
            self.visibility(&t.vis),
            t.name,
            self.generic_params_to_string(&t.generics.params),
        ));
        self.write_where_predicates(&t.generics.where_predicates);
        self.writeln(&format!(" = {};", self.type_to_string(&t.target)));
        self.writeln("");
    }

    fn generate_closure_param(&mut self, param: &ClosureParam) {
        self.write(&param.pattern);
        if let Some(ty) = &param.ty {
            self.write(": ");
            self.write(&self.type_to_string(ty));
        }
    }

    fn generic_params_to_string(&self, generics: &[GenericParam]) -> String {
        if generics.is_empty() {
            return String::new();
        }

        format!(
            "<{}>",
            generics
                .iter()
                .map(|generic| self.generic_param_to_string(generic))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    fn generate_enum(&mut self, e: &EnumDef) {
        for doc in &e.docs {
            self.writeln(doc);
        }

        if !e.derives.is_empty() {
            self.write("#[derive(");
            for (i, derive) in e.derives.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.write(derive);
            }
            self.writeln(")]");
        }

        self.write(&format!("{}enum {} ", self.visibility(&e.vis), e.name));

        self.write(&self.generic_params_to_string(&e.generics.params));
        self.write_where_predicates(&e.generics.where_predicates);
        self.writeln(" {");

        self.indent();
        for variant in &e.variants {
            for doc in &variant.docs {
                self.writeln(doc);
            }
            self.write(&variant.name);
            if let Some(types) = &variant.data {
                self.write("(");
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(&self.type_to_string(ty));
                }
                self.write(")");
            }
            self.writeln(",");
        }
        self.dedent();
        self.writeln("}");
        self.writeln("");
    }

    fn generate_const(&mut self, c: &ConstDef) {
        for doc in &c.docs {
            self.writeln(doc);
        }
        self.write(&format!(
            "{}const {}: {} = ",
            self.visibility(&c.vis),
            c.name,
            self.type_to_string(&c.ty)
        ));
        self.generate_expr(&c.value);
        self.writeln(";");
        self.writeln("");
    }

    fn generate_use(&mut self, import: &UseStatement) {
        for attr in &import.attrs {
            self.writeln(attr);
        }
        self.write(&self.visibility(&import.vis));
        self.write("use ");
        if import.absolute {
            self.write("::");
        }
        self.write(&import.path.join("::"));
        let separator = if import.path.is_empty() { "" } else { "::" };
        match &import.kind {
            UseKind::Simple => self.writeln(";"),
            UseKind::Glob => self.writeln(&format!("{separator}*;")),
            UseKind::Nested(items) => {
                self.writeln(&format!("{separator}{{{}}};", items.join(", ")));
            }
        }
    }

    fn generate_attribute(&mut self, attr: &Attribute) {
        self.write(&format!("#[{}", attr.name));
        if !attr.args.is_empty() {
            self.write("(");
            for (i, arg) in attr.args.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                match arg {
                    AttributeArg::Ident(id) => self.write(id),
                    AttributeArg::Literal(lit) => self.generate_literal(lit),
                    AttributeArg::KeyValue(k, v) => {
                        self.write(&format!("{} = ", k));
                        self.generate_literal(v);
                    }
                }
            }
            self.write(")");
        }
        self.writeln("]");
    }

    fn type_to_string(&self, ty: &Type) -> String {
        match ty {
            Type::Path(path) => path.join("::"),
            Type::Named(name) => name.clone(),
            Type::Generic(name, params) => {
                let mut s = name.clone();
                s.push('<');
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    s.push_str(&self.type_to_string(param));
                }
                s.push('>');
                s
            }
            Type::ImplTrait {
                trait_path,
                associated_types,
            } => {
                let arguments = associated_types
                    .iter()
                    .map(|binding| format!("{} = {}", binding.name, self.type_to_string(&binding.ty)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("impl {}<{arguments}>", trait_path.join("::"))
            }
            Type::CallableTrait(callable) => {
                let qualifier = match callable.qualifier {
                    CallableTraitQualifier::Dyn => "dyn",
                    CallableTraitQualifier::Impl => "impl",
                };
                let args = callable
                    .args
                    .iter()
                    .map(|arg| self.type_to_string(arg))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut source = format!("{qualifier} ");
                for bound in &callable.leading_bounds {
                    source.push_str(&callable_bound_to_string(bound));
                    source.push_str(" + ");
                }
                if !callable.higher_ranked.is_empty() {
                    let binder = callable
                        .higher_ranked
                        .iter()
                        .map(|parameter| bounded_name(&parameter.name, &parameter.bounds))
                        .collect::<Vec<_>>()
                        .join(", ");
                    source.push_str(&format!("for<{binder}> "));
                }
                source.push_str(&format!(
                    "{}({args}) -> {}",
                    callable.trait_path.join("::"),
                    self.type_to_string(&callable.return_type)
                ));
                for bound in &callable.trailing_bounds {
                    source.push_str(" + ");
                    source.push_str(&callable_bound_to_string(bound));
                }
                source
            }
            Type::Reference(inner, lifetime, mutable) => {
                let (lifetime, separator) = lifetime.as_deref().map_or(("", ""), |name| (name, " "));
                let mutable = if *mutable { "mut " } else { "" };
                let source = self.type_to_string(inner);
                if matches!(inner.as_ref(), Type::CallableTrait(callable)
                        if !callable.leading_bounds.is_empty() || !callable.trailing_bounds.is_empty())
                {
                    format!("&{lifetime}{separator}{mutable}({source})")
                } else {
                    format!("&{lifetime}{separator}{mutable}{source}")
                }
            }
            Type::Tuple(types) => {
                let mut s = "(".to_string();
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    s.push_str(&self.type_to_string(ty));
                }
                if types.len() == 1 {
                    s.push(',');
                }
                s.push(')');
                s
            }
            Type::Slice(inner) => format!("[{}]", self.type_to_string(inner)),
            Type::Array(inner, size) => format!("[{}; {}]", self.type_to_string(inner), size),
            Type::Unit => "()".to_string(),
            Type::Never => "!".to_string(),
            Type::Infer => "_".to_string(),
        }
    }

    fn visibility(&self, vis: &Visibility) -> String {
        match vis {
            Visibility::Public => "pub ".to_string(),
            Visibility::Private => "".to_string(),
            Visibility::Restricted(path) => match path.as_slice() {
                [scope] if matches!(scope.as_str(), "crate" | "self" | "super") => {
                    format!("pub({scope}) ")
                }
                _ => format!("pub(in {}) ", path.join("::")),
            },
            Visibility::None => "".to_string(),
        }
    }

    fn generate_lazy_static(&mut self, l: &LazyStaticDef) {
        // Documentation comments
        for doc in &l.docs {
            self.writeln(doc);
        }

        // Generate lazy_static! macro
        self.writeln("lazy_static! {");
        self.indent();

        // static ref NAME: TYPE = { ... };
        self.write("static ref ");
        self.write(&l.name);
        self.write(": ");
        self.write(&self.type_to_string(&l.ty));
        self.write(" = ");

        // Generate initializer block with braces
        self.writeln("{");
        self.indent();
        self.generate_block(&l.init);
        self.dedent();
        self.write("}");
        self.write(";");
        self.writeln("");

        self.dedent();
        self.writeln("}");
        self.writeln("");
    }

    fn generate_union(&mut self, u: &UnionDef) {
        // Documentation comments
        for doc in &u.docs {
            self.writeln(doc);
        }

        // Derive attributes
        if !u.derives.is_empty() {
            self.write("#[derive(");
            for (i, derive) in u.derives.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.write(derive);
            }
            self.writeln(")]");
        }

        // Union definition
        self.write(&format!("{}union {} ", self.visibility(&u.vis), u.name));

        self.write(&self.generic_params_to_string(&u.generics.params));
        self.write_where_predicates(&u.generics.where_predicates);
        self.writeln(" {");

        self.indent();

        // Union fields
        for field in &u.fields {
            self.generate_field(field);
        }

        self.dedent();
        self.writeln("}");
        self.writeln("");
    }

    // Helper methods
    fn writeln(&mut self, s: &str) {
        self.write(s);
        self.buffer.push('\n');
    }

    fn write(&mut self, s: &str) {
        if self.buffer.ends_with('\n') || self.buffer.is_empty() {
            self.buffer.push_str(&"    ".repeat(self.indent_level));
        }
        self.buffer.push_str(s);
    }

    fn indent(&mut self) {
        self.indent_level += 1;
    }

    fn dedent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }
}

fn should_separate_after_use(item: &Item, next: Option<&Item>) -> bool {
    match (item, next) {
        (Item::Use(current), Some(Item::Use(next))) => use_root(current) != use_root(next),
        (Item::Use(_), Some(_)) => true,
        _ => false,
    }
}

fn use_root(use_stmt: &UseStatement) -> Option<&str> {
    use_stmt.path.first().map(String::as_str)
}

fn callable_bound_to_string(bound: &CallableBound) -> String {
    match bound {
        CallableBound::Lifetime(lifetime) => lifetime.clone(),
        CallableBound::Trait(path) => path.join("::"),
    }
}

fn bounded_name(name: &str, bounds: &[String]) -> String {
    if bounds.is_empty() {
        name.to_string()
    } else {
        format!("{name}: {}", ordered_bounds_to_string(bounds))
    }
}

fn ordered_bounds_to_string(bounds: &[String]) -> String {
    bounds
        .iter()
        .filter(|bound| bound.as_str() != "'static")
        .chain(bounds.iter().filter(|bound| bound.as_str() == "'static"))
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
mod tests {
    use super::RustCodeGenerator;
    use crate::{BindingMode, Pattern};
    use crate::rustlight_ast::{
        Block, CallableTraitQualifier, CallableTraitType, ClosureParam, Expr, FunctionDef,
        GenericParam, Item, Literal, Param, PathType, RustModule, Statement, Type, TypeAlias, TypeParameter,
        Visibility,
    };

    fn callable_target() -> Type {
        Type::Generic(
            "Rc".to_string(),
            vec![Type::CallableTrait(CallableTraitType {
                qualifier: CallableTraitQualifier::Dyn,
                trait_path: vec!["Fn".to_string()],
                args: vec![Type::Named("Int".to_string())],
                return_type: Box::new(Type::Named("Int".to_string())),
                leading_bounds: Vec::new(),
                trailing_bounds: Vec::new(),
                higher_ranked: Vec::new(),
            })],
        )
    }

    fn print_function_body(expr: Expr, target: Type) -> String {
        let module = RustModule {
            name: "Cast_Test".to_string(),
            docs: Vec::new(),
            items: vec![Item::Function(FunctionDef {
                name: "cast_closure".to_string(),
                params: vec![Param {
                    name: "f".to_string(),
                    ty: target.clone(),
                    mutable: false,
                }],
                return_type: target.clone(),
                generics: Default::default(),
                body: Block {
                    stmts: Vec::new(),
                    expr: Some(Box::new(expr)),
                },
                asyncness: false,
                vis: Visibility::Public,
                docs: Vec::new(),
                attrs: Vec::new(),
            })],
            attrs: Vec::new(),
            vis: Visibility::Private,
        };

        RustCodeGenerator::new().generate_module_code(&module)
    }

    #[test]
    fn prints_structured_cast_expression() {
        let target = callable_target();
        let printed = print_function_body(
            Expr::Cast(Box::new(Expr::Ident("f".to_string())), target.clone()),
            target,
        );
        assert!(printed.contains("(f as Rc<dyn Fn(Int) -> Int>)"));
    }

    #[test]
    fn avoids_duplicate_parentheses_around_cast() {
        let target = callable_target();
        let printed = print_function_body(
            Expr::Parenthesized(Box::new(Expr::Cast(
                Box::new(Expr::Ident("f".to_string())),
                target.clone(),
            ))),
            target,
        );
        assert!(printed.contains("(f as Rc<dyn Fn(Int) -> Int>)"));
        assert!(!printed.contains("((f as Rc<dyn Fn(Int) -> Int>))"));
    }

    #[test]
    fn omits_outer_cast_parentheses_in_call_arguments() {
        let target = callable_target();
        let cast = Expr::Parenthesized(Box::new(Expr::Cast(
            Box::new(Expr::Ident("f".to_string())),
            target.clone(),
        )));
        let printed = print_function_body(
            Expr::Call(
                Box::new(Expr::Path(
                    vec!["Reg".to_string(), "Reg".to_string()],
                    PathType::Namespace,
                )),
                vec![cast],
            ),
            target,
        );
        assert!(printed.contains("Reg::Reg(f as Rc<dyn Fn(Int) -> Int>)"));
        assert!(!printed.contains("Reg::Reg((f as Rc<dyn Fn(Int) -> Int>))"));
    }

    #[test]
    fn omits_redundant_parentheses_around_block_call_arguments() {
        let printed = print_function_body(
            Expr::Call(
                Box::new(Expr::Ident("f".to_string())),
                vec![Expr::Parenthesized(Box::new(Expr::Block(Block {
                    stmts: Vec::new(),
                    expr: Some(Box::new(Expr::Ident("x".to_string()))),
                })))],
            ),
            Type::Named("Int".to_string()),
        );
        assert!(printed.contains("f({"));
        assert!(!printed.contains("f(({"));
    }

    #[test]
    fn omits_redundant_parentheses_around_unary_call_arguments() {
        let printed = print_function_body(
            Expr::Call(
                Box::new(Expr::Ident("f".to_string())),
                vec![Expr::Parenthesized(Box::new(Expr::UnaryOp(
                    "-".to_string(),
                    Box::new(Expr::Ident("i".to_string())),
                )))],
            ),
            Type::Named("Int".to_string()),
        );
        assert!(printed.contains("f(-i)"));
        assert!(!printed.contains("f((-i))"));
    }

    #[test]
    fn preserves_tuple_parentheses_in_call_arguments() {
        let printed = print_function_body(
            Expr::Call(
                Box::new(Expr::Ident("f".to_string())),
                vec![Expr::Parenthesized(Box::new(Expr::Tuple(vec![
                    Expr::Ident("a".to_string()),
                    Expr::Ident("b".to_string()),
                ])))],
            ),
            Type::Named("Int".to_string()),
        );
        assert!(printed.contains("f((a, b))"));
    }

    #[test]
    fn parenthesizes_borrowed_binary_expressions() {
        let printed = print_function_body(
            Expr::Reference(
                Box::new(Expr::BinaryOp(
                    Box::new(Expr::Ident("n".to_string())),
                    "+".to_string(),
                    Box::new(Expr::Literal(Literal::Int(1))),
                )),
                true,
                false,
            ),
            Type::Named("BigInt".to_string()),
        );
        assert!(printed.contains("&(n + 1)"));
    }

    #[test]
    fn prints_explicit_closure_return_types() {
        let printed = print_function_body(
            Expr::TypedClosure(
                vec![ClosureParam::typed("x", Type::Named("Int".to_string()))],
                Type::Named("Pred<Unit>".to_string()),
                Box::new(Expr::Macro("panic!(\"partial\")".to_string())),
                true,
            ),
            callable_target(),
        );
        assert!(printed.contains("move |x: Int| -> Pred<Unit> { panic!(\"partial\") }"));
    }

    #[test]
    fn prints_generic_type_alias_parameters_and_bounds() {
        let module = RustModule {
            name: "Alias_Test".to_string(),
            docs: Vec::new(),
            items: vec![Item::TypeAlias(TypeAlias {
                name: "Callback".to_string(),
                target: Type::Generic("Rc".to_string(), vec![Type::Named("T".to_string())]),
                generics: vec![GenericParam::Type(TypeParameter {
                    name: "T".to_string(),
                    bounds: vec!["Clone".to_string()],
                })].into(),
                vis: Visibility::Public,
                docs: Vec::new(),
            })],
            attrs: Vec::new(),
            vis: Visibility::Private,
        };
        let mut generator = RustCodeGenerator::new();
        let printed = generator.generate_module_code(&module);

        assert!(printed.contains("pub type Callback<T: Clone> = Rc<T>;"));
    }

    #[test]
    fn prints_bare_return_statement() {
        let module = RustModule {
            name: "Return_Test".to_string(),
            docs: Vec::new(),
            items: vec![Item::Function(FunctionDef {
                name: "early_exit".to_string(),
                params: Vec::new(),
                return_type: Type::Unit,
                generics: Default::default(),
                body: Block {
                    stmts: vec![Statement::Expr(Expr::Return(None))],
                    expr: None,
                },
                asyncness: false,
                vis: Visibility::Public,
                docs: Vec::new(),
                attrs: Vec::new(),
            })],
            attrs: Vec::new(),
            vis: Visibility::Private,
        };

        let printed = RustCodeGenerator::new().generate_module_code(&module);
        assert!(printed.contains("return;"));
    }

    #[test]
    fn prints_return_statement_with_value() {
        let module = RustModule {
            name: "Return_Test".to_string(),
            docs: Vec::new(),
            items: vec![Item::Function(FunctionDef {
                name: "return_value".to_string(),
                params: Vec::new(),
                return_type: Type::Named("Int".to_string()),
                generics: Default::default(),
                body: Block {
                    stmts: vec![Statement::Expr(Expr::Return(Some(Box::new(Expr::Ident(
                        "value".to_string(),
                    )))))],
                    expr: None,
                },
                asyncness: false,
                vis: Visibility::Public,
                docs: Vec::new(),
                attrs: Vec::new(),
            })],
            attrs: Vec::new(),
            vis: Visibility::Private,
        };

        let printed = RustCodeGenerator::new().generate_module_code(&module);
        assert!(printed.contains("return value;"));
    }

    #[test]
    fn prints_nested_else_if_without_an_extra_block() {
        let bool_block = |value| Block {
            stmts: Vec::new(),
            expr: Some(Box::new(Expr::Literal(Literal::Bool(value)))),
        };
        let nested_if = Expr::If {
            condition: Box::new(Expr::Ident("inner".to_string())),
            then_branch: bool_block(true),
            else_branch: Some(bool_block(false)),
        };
        let printed = print_function_body(
            Expr::If {
                condition: Box::new(Expr::Ident("outer".to_string())),
                then_branch: bool_block(true),
                else_branch: Some(Block {
                    stmts: Vec::new(),
                    expr: Some(Box::new(nested_if)),
                }),
            },
            Type::Named("bool".to_string()),
        );

        assert!(printed.contains("} else if inner {"));
        assert!(!printed.contains("} else {\n        if inner"));
    }

    #[test]
    fn prints_while_loop() {
        let printed = print_function_body(
            Expr::While {
                condition: Box::new(Expr::BinaryOp(
                    Box::new(Expr::Ident("i".to_string())),
                    "<".to_string(),
                    Box::new(Expr::Literal(Literal::Int(10))),
                )),
                body: Block {
                    stmts: vec![Statement::Expr(Expr::Break(None))],
                    expr: None,
                },
            },
            Type::Unit,
        );

        assert!(printed.contains("while i < 10 {\n        break;\n    }"));
    }

    #[test]
    fn prints_for_loop() {
        let printed = print_function_body(
            Expr::For {
                pattern: Pattern::Ident {
                    name: "i".to_string(),
                    mode: BindingMode::Value,
                    subpattern: None,
                },
                iter: Box::new(Expr::Array(vec![
                    Expr::Literal(Literal::Int(1)),
                    Expr::Literal(Literal::Int(2)),
                    Expr::Literal(Literal::Int(3)),
                ])),
                body: Block {
                    stmts: vec![Statement::Expr(Expr::Continue)],
                    expr: None,
                },
            },
            Type::Unit,
        );

        assert!(printed.contains("for i in [1, 2, 3] {\n        continue;\n    }"));
    }
}
