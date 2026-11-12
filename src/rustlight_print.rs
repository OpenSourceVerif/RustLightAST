#![allow(clippy::only_used_in_recursion)]
use crate::rustlight_ast::*;
use std::borrow::Cow;

mod pattern;

const MAX_FUNCTION_SIGNATURE_WIDTH: usize = 80;

// In increasing binding strength, following the Rust Reference's expression table.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Precedence {
    Jump,
    Assignment,
    Range,
    Or,
    And,
    Comparison,
    BitOr,
    BitXor,
    BitAnd,
    Shift,
    Sum,
    Product,
    Cast,
    Prefix,
    Postfix,
    Atom,
}

impl Precedence {
    fn binary(op: &str) -> Self {
        match op {
            "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" => {
                Self::Assignment
            }
            "||" => Self::Or,
            "&&" => Self::And,
            "==" | "!=" | "<" | ">" | "<=" | ">=" => Self::Comparison,
            "|" => Self::BitOr,
            "^" => Self::BitXor,
            "&" => Self::BitAnd,
            "<<" | ">>" => Self::Shift,
            "+" | "-" => Self::Sum,
            "*" | "/" | "%" => Self::Product,
            _ => Self::Jump,
        }
    }

    fn of(expr: &Expr) -> Self {
        match expr {
            Expr::Closure(..)
            | Expr::TypedClosure(..)
            | Expr::Return(..)
            | Expr::Break { .. }
            | Expr::Continue(..) => Self::Jump,
            Expr::Assign(..) => Self::Assignment,
            Expr::Range { .. } => Self::Range,
            Expr::BinaryOp(_, op, _) => Self::binary(op),
            Expr::Reference(inner, false, false) => Self::of(inner),
            Expr::Reference(..) | Expr::UnaryOp(..) => Self::Prefix,
            Expr::Literal(Literal::Int(value)) if *value < 0 => Self::Prefix,
            Expr::Literal(Literal::Float(value)) if value.is_sign_negative() => Self::Prefix,
            Expr::Call(..)
            | Expr::MethodCall(..)
            | Expr::Index(..)
            | Expr::Field { .. }
            | Expr::Await(..)
            | Expr::Try(..) => Self::Postfix,
            // Casts print with their own parentheses except in call arguments.
            _ => Self::Atom,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExpressionPosition {
    Free,
    StatementHead,
    BeforeBlock,
}

impl ExpressionPosition {
    fn without_statement_head(self) -> Self {
        match self {
            Self::StatementHead => Self::Free,
            position => position,
        }
    }
}

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

    /// Print one modeled Rust type at the source boundary.
    pub fn generate_type_code(&self, ty: &Type) -> String {
        self.type_to_string(ty)
    }

    /// Render namespace segments for consumers that resolve paths by spelling.
    pub fn namespace_path_segments<'a>(
        path: &'a [String],
        kind: &PathType,
    ) -> Option<Cow<'a, [String]>> {
        match kind {
            PathType::Namespace => Some(Cow::Borrowed(path)),
            PathType::NamespaceWithTypeArguments(arguments) => {
                let printer = Self::new();
                let mut segments = path.to_vec();
                for list in arguments {
                    let types = list.arguments
                        .iter()
                        .map(|ty| printer.generate_type_code(ty))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let segment = &mut segments[list.segment];
                    *segment = format!("{segment}::<{types}>");
                }
                Some(Cow::Owned(segments))
            }
            PathType::Qualified(_) => None,
        }
    }

    /// Render a record constructor path with its explicit type arguments.
    pub fn generate_record_path_code(&self, path: &RecordPath) -> String {
        match path {
            RecordPath::Segments(segments) => segments.join("::"),
            RecordPath::WithTypeArguments { segments, arguments } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.type_to_string(argument))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}::<{arguments}>", segments.join("::"))
            }
        }
    }

    /// Print an expression for placement at an existing source node.
    pub fn generate_expression_code(&mut self, expression: &Expr) -> String {
        self.buffer.clear();
        self.indent_level = 0;
        self.generate_expr(expression);
        self.buffer.clone()
    }

    /// Render a braced block at the source boundary.
    pub fn generate_block_code(&mut self, block: &Block) -> String {
        self.buffer.clear();
        self.indent_level = 0;
        self.generate_braced_block(block);
        self.buffer.clone()
    }

    /// Render one declaration for publication at its existing source scope.
    pub fn generate_item_code(&mut self, item: &Item) -> String {
        self.buffer.clear();
        self.indent_level = 0;
        self.generate_item(item);
        self.buffer.clone()
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
            Item::Trait(t) => self.generate_trait(t),
            Item::Const(c) => self.generate_const(c),
            Item::TypeAlias(t) => self.generate_type_alias(t),
            Item::Use(u) => self.generate_use(u),
            Item::Mod(m) => self.generate_nested_module(m),
            Item::ModuleDecl(module) => {
                for attr in &module.attrs {
                    self.writeln(attr);
                }
                self.writeln(&format!(
                    "{}mod {};",
                    self.visibility(&module.vis),
                    module.name
                ));
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
        for doc in &m.docs {
            self.writeln(doc);
        }
        for attr in &m.attrs {
            self.generate_attribute(attr);
        }

        // Generate the module declaration line
        match &m.vis {
            Visibility::Public => self.write("pub "),
            Visibility::Private => (), // Do not add a modifier for private modules
            Visibility::Restricted(paths) => self.write(&format!("pub(in {} ) ", paths.join("::"))),
            Visibility::None => (),
        }

        self.writeln(&format!("mod {} {{", m.name));
        self.indent();

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
        self.generate_data_attributes(&s.attrs);

        self.write(&format!("{}struct {} ", self.visibility(&s.vis), s.name));
        self.write(&self.generic_params_to_string(&s.generics.params));

        match &s.fields {
            Fields::Unit => {
                self.write_where_predicates(&s.generics.where_predicates);
                self.writeln(";");
            }
            Fields::Tuple(fields) => {
                self.generate_tuple_fields(fields);
                self.write_where_predicates(&s.generics.where_predicates);
                self.writeln(";");
            }
            Fields::Named(fields) => {
                self.write_where_predicates(&s.generics.where_predicates);
                self.generate_named_fields(fields);
                self.writeln("");
            }
        }
        self.writeln("");
    }

    fn generate_tuple_fields(&mut self, fields: &[Field]) {
        self.write("(");
        for (i, field) in fields.iter().enumerate() {
            if i > 0 {
                self.write(", ");
            }
            self.generate_field_metadata(field);
            self.write(&self.visibility(&field.vis));
            self.write(&self.type_to_string(&field.ty));
        }
        self.write(")");
    }

    fn generate_named_fields(&mut self, fields: &[Field]) {
        self.writeln(" {");
        self.indent();
        for field in fields {
            self.generate_field(field);
        }
        self.dedent();
        self.write("}");
    }

    fn generate_field_metadata(&mut self, field: &Field) {
        for doc in &field.docs {
            self.writeln(doc);
        }
        for attr in &field.attrs {
            self.generate_attribute(attr);
        }
    }

    fn generate_field(&mut self, field: &Field) {
        self.generate_field_metadata(field);
        self.writeln(&format!(
            "{}{}: {},",
            self.visibility(&field.vis),
            field.name,
            self.type_to_string(&field.ty)
        ));
    }

    fn generate_impl(&mut self, i: &ImplBlock) {
        self.generate_attributes(&i.attrs, false);
        self.write(if i.unsafety { "unsafe impl" } else { "impl" });

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
        self.generate_attributes(&i.attrs, true);

        for item in &i.items {
            match item {
                ImplItem::Method(m) => self.generate_function(m),
                ImplItem::AssocConst(c) => self.generate_const(c),
                ImplItem::AssocType(t) => self.generate_type_alias(t),
            }
        }

        self.dedent();
        self.writeln("}");
        self.writeln("");
    }

    fn generate_trait(&mut self, trait_def: &TraitDef) {
        self.generate_attributes(&trait_def.attrs, false);
        self.write(&format!(
            "{}{}{}trait {}{}",
            self.visibility(&trait_def.vis),
            if trait_def.unsafety { "unsafe " } else { "" },
            if trait_def.auto { "auto " } else { "" },
            trait_def.name,
            self.generic_params_to_string(&trait_def.generics.params)
        ));
        if !trait_def.supertraits.is_empty() {
            self.write(": ");
            self.write(&trait_def.supertraits.join(" + "));
        }
        self.write_where_predicates(&trait_def.generics.where_predicates);
        self.writeln(" {");
        self.indent();
        self.generate_attributes(&trait_def.attrs, true);
        for member in &trait_def.members {
            match member {
                TraitMember::DefaultMethod(function) => self.generate_function(function),
                TraitMember::Raw(source) => self.generate_raw(source),
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

        self.generate_attributes(&f.attrs, false);

        let abi = match &f.qualifiers.abi {
            Some(FunctionAbi::Default) => "extern ".to_string(),
            Some(FunctionAbi::Named(name)) => format!("extern {name:?} "),
            None => String::new(),
        };
        let head = format!(
            "{}{}{}{}{}fn {}{}",
            self.visibility(&f.vis),
            if f.qualifiers.constness { "const " } else { "" },
            if f.asyncness { "async " } else { "" },
            if f.qualifiers.unsafety { "unsafe " } else { "" },
            abi,
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
                let comma = if position + 1 < where_clauses.len() {
                    ","
                } else {
                    ""
                };
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
        self.generate_attributes(&f.attrs, true);
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
                GenericParam::Lifetime(_) | GenericParam::Const { .. } => {
                    out.push_str(&self.generic_param_to_string(generic));
                }
            }
        }
        out.push('>');
        out
    }

    fn generic_param_to_string(&self, generic: &GenericParam) -> String {
        match generic {
            GenericParam::Const { name, ty, default } => {
                let mut parameter = format!("const {name}: {}", self.type_to_string(ty));
                if let Some(value) = default {
                    parameter.push_str(" = ");
                    parameter.push_str(&Self::new().generate_expression_code(value));
                }
                parameter
            }
            _ => bounded_name(generic.name(), generic.bounds()),
        }
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
            format!(
                "{modifier}{}: {}",
                param.name,
                self.type_to_string(&param.ty)
            )
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
            .map(|param| {
                format!(
                    "{}: {}",
                    param.name,
                    ordered_bounds_to_string(&param.bounds)
                )
            })
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
            self.generate_expr_at(expr, ExpressionPosition::StatementHead);
            self.writeln("");
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
                self.generate_expr_at(expr, ExpressionPosition::StatementHead);
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
        self.generate_expr_at(expr, ExpressionPosition::Free);
    }

    fn generate_expr_at(&mut self, expr: &Expr, position: ExpressionPosition) {
        match expr {
            Expr::Ident(id) => self.write(id),
            Expr::Macro(source) => self.write(source),
            Expr::Path(path, PathType::Namespace) => self.generate_path(path),
            Expr::Path(path, kind @ PathType::NamespaceWithTypeArguments(_)) => {
                let segments = Self::namespace_path_segments(path, kind)
                    .expect("modeled type arguments are on a namespace path");
                self.generate_path(&segments);
            }
            Expr::Path(path, PathType::Qualified(qualification)) => {
                self.write("<");
                match &qualification.self_type {
                    QualifiedSelfType::Type(ty) => self.write(&self.type_to_string(ty)),
                    QualifiedSelfType::Verbatim(source) => self.write(source),
                }
                if let Some(trait_path) = &qualification.trait_path {
                    self.write(" as ");
                    self.generate_path(trait_path);
                }
                self.write(">::");
                self.generate_path(path);
            }
            Expr::Field { base, member } => {
                self.generate_postfix_base(base, position);
                self.write(".");
                self.generate_member(member);
            }
            // A struct literal exposed before a control-flow body needs grouping.
            Expr::Record { .. } if position == ExpressionPosition::BeforeBlock => {
                self.generate_parenthesized(expr);
            }
            Expr::Record { path, fields, rest } => {
                self.write(&self.generate_record_path_code(path));
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
                self.write(if fields.is_empty() && rest.is_none() {
                    "}"
                } else {
                    " }"
                });
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
                    self.generate_operand(start, Precedence::Range, false, position);
                }
                self.write(match end {
                    RangeEnd::Exclusive(_) => "..",
                    RangeEnd::Inclusive(_) => "..=",
                });
                if let Some(end) = end.expression() {
                    self.generate_operand(
                        end,
                        Precedence::Range,
                        false,
                        position.without_statement_head(),
                    );
                }
            }
            Expr::Try(operand) => {
                self.generate_postfix_base(operand, position);
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
                self.generate_callee(callee, position);
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
                if method.is_empty() {
                    self.generate_callee(receiver, position);
                } else {
                    self.generate_postfix_base(receiver, position);
                    self.write(".");
                    self.write(method);
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
            Expr::Block(block) => self.generate_braced_block(block),
            Expr::LabeledBlock { label, body } => {
                self.generate_label(Some(label));
                self.generate_braced_block(body);
            }
            Expr::Loop(block, label) => {
                self.generate_label(label.as_deref());
                self.writeln("loop {");
                self.indent();
                self.generate_block(block);
                self.dedent();
                self.write("}");
            }
            Expr::While {
                label,
                condition,
                body,
            } => {
                self.generate_label(label.as_deref());
                self.write("while ");
                self.generate_expr_at(condition, ExpressionPosition::BeforeBlock);
                self.writeln(" {");
                self.indent();
                self.generate_block(body);
                self.dedent();
                self.write("}");
            }
            Expr::For {
                label,
                pattern,
                iter,
                body,
            } => {
                self.generate_label(label.as_deref());
                self.write("for ");
                self.generate_pattern(pattern);
                self.write(" in ");
                self.generate_expr_at(iter, ExpressionPosition::BeforeBlock);
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
                self.generate_let_scrutinee(value, position.without_statement_head());
            }
            Expr::Return(value) => self.generate_jump_expr("return", None, value.as_deref()),
            Expr::Break { label, value } => {
                self.generate_jump_expr("break", label.as_deref(), value.as_deref())
            }
            Expr::Continue(label) => self.generate_jump_expr("continue", label.as_deref(), None),
            Expr::Async { body, move_capture } => {
                self.write(if *move_capture {
                    "async move "
                } else {
                    "async "
                });
                self.generate_braced_block(body);
            }
            Expr::Await(expr) => {
                self.generate_postfix_base(expr, position);
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
                self.generate_expr_at(body, position.without_statement_head());
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
                self.generate_expr_at(expr, ExpressionPosition::BeforeBlock);
                self.writeln(" {");
                self.indent();
                for (index, arm) in arms.iter().enumerate() {
                    self.generate_pattern(&arm.pattern);
                    if let Some(guard) = &arm.guard {
                        self.write(" if ");
                        self.generate_expr(guard);
                    }
                    self.write(" => ");
                    let needs_comma = match &arm.body.expr {
                        Some(expr) if !arm.braced && arm.body.stmts.is_empty() => {
                            self.generate_expr(expr);
                            !matches!(
                                expr.as_ref(),
                                Expr::Block(_)
                                    | Expr::LabeledBlock { .. }
                                    | Expr::Async { .. }
                                    | Expr::Unsafe(_)
                                    | Expr::Loop(..)
                                    | Expr::While { .. }
                                    | Expr::For { .. }
                                    | Expr::If { .. }
                                    | Expr::IfLet { .. }
                                    | Expr::Match { .. }
                            )
                        }
                        _ => {
                            self.writeln("{");
                            self.indent();
                            self.generate_block(&arm.body);
                            self.dedent();
                            self.write("}");
                            false
                        }
                    };
                    self.writeln(if arm.comma || (needs_comma && index + 1 < arms.len()) {
                        ","
                    } else {
                        ""
                    });
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
                self.generate_expr_at(condition, ExpressionPosition::BeforeBlock);
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
                self.generate_let_scrutinee(value, ExpressionPosition::BeforeBlock);
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
            Expr::Reference(inner, is_reference, mutable) => {
                if *is_reference {
                    self.write("&");
                }
                if *mutable {
                    self.write("mut ");
                }
                if *is_reference || *mutable {
                    self.generate_operand(
                        inner,
                        Precedence::Prefix,
                        true,
                        position.without_statement_head(),
                    );
                } else {
                    self.generate_expr_at(inner, position);
                }
            }
            Expr::BinaryOp(left, op, right) => {
                let precedence = Precedence::binary(op);
                let left_associative =
                    !matches!(precedence, Precedence::Assignment | Precedence::Comparison);
                self.generate_operand(left, precedence, left_associative, position);
                self.write(" ");
                self.write(op);
                self.write(" ");
                self.generate_operand(
                    right,
                    precedence,
                    precedence == Precedence::Assignment,
                    position.without_statement_head(),
                );
            }
            Expr::Assign(left, right) => {
                self.generate_operand(left, Precedence::Assignment, false, position);
                self.write(" = ");
                self.generate_operand(
                    right,
                    Precedence::Assignment,
                    true,
                    position.without_statement_head(),
                );
            }
            Expr::UnaryOp(op, expr) => {
                self.write(op);
                self.generate_operand(
                    expr,
                    Precedence::Prefix,
                    true,
                    position.without_statement_head(),
                );
            }
            Expr::Index(array, index) => {
                self.generate_postfix_base(array, position);
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

    fn generate_braced_block(&mut self, block: &Block) {
        self.writeln("{");
        self.indent();
        self.generate_block(block);
        self.dedent();
        self.write("}");
    }

    fn generate_label(&mut self, label: Option<&str>) {
        if let Some(label) = label {
            self.write("'");
            self.write(label);
            self.write(": ");
        }
    }

    fn generate_jump_expr(&mut self, keyword: &str, label: Option<&str>, value: Option<&Expr>) {
        self.write(keyword);
        if let Some(label) = label {
            self.write(" '");
            self.write(label);
        }
        if let Some(value) = value {
            self.write(" ");
            self.generate_expr(value);
        }
    }

    fn generate_let_scrutinee(&mut self, value: &Expr, position: ExpressionPosition) {
        // A lazy boolean operator can start a new condition in a let chain.
        self.generate_operand(value, Precedence::Comparison, true, position);
    }

    fn generate_operand(
        &mut self,
        expr: &Expr,
        precedence: Precedence,
        allow_equal: bool,
        position: ExpressionPosition,
    ) {
        let actual = Precedence::of(expr);
        // Rust can end a statement at a leading block before seeing its operator.
        let leading_block = position == ExpressionPosition::StatementHead
            && matches!(
                expr,
                Expr::Block(_)
                    | Expr::LabeledBlock { .. }
                    | Expr::Unsafe(_)
                    | Expr::Loop(..)
                    | Expr::While { .. }
                    | Expr::For { .. }
                    | Expr::If { .. }
                    | Expr::IfLet { .. }
                    | Expr::Match { .. }
            );
        if actual < precedence || (actual == precedence && !allow_equal) || leading_block {
            self.generate_parenthesized(expr);
        } else {
            self.generate_expr_at(expr, position);
        }
    }

    fn generate_parenthesized(&mut self, expr: &Expr) {
        self.write("(");
        self.generate_expr(expr);
        self.write(")");
    }

    fn generate_postfix_base(&mut self, base: &Expr, position: ExpressionPosition) {
        self.generate_operand(base, Precedence::Postfix, true, position);
    }

    fn generate_callee(&mut self, callee: &Expr, position: ExpressionPosition) {
        // An ungrouped named field followed by parentheses is a method call.
        if matches!(
            callee,
            Expr::Field {
                member: Member::Named(_),
                ..
            }
        ) {
            self.generate_parenthesized(callee);
        } else {
            self.generate_postfix_base(callee, position);
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
        self.generate_operand(expr, Precedence::Cast, true, ExpressionPosition::Free);
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
        self.generate_attributes(&t.attrs, false);
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
        self.generate_data_attributes(&e.attrs);

        self.write(&format!("{}enum {} ", self.visibility(&e.vis), e.name));

        self.write(&self.generic_params_to_string(&e.generics.params));
        self.write_where_predicates(&e.generics.where_predicates);
        self.writeln(" {");

        self.indent();
        for variant in &e.variants {
            for attr in &variant.attrs {
                self.writeln(attr);
            }
            self.write(&variant.name);
            match &variant.fields {
                Fields::Unit => {}
                Fields::Tuple(fields) => self.generate_tuple_fields(fields),
                Fields::Named(fields) => self.generate_named_fields(fields),
            }
            if let Some(discriminant) = &variant.discriminant {
                self.write(" = ");
                self.generate_expr(discriminant);
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
        self.generate_attributes(&c.attrs, false);
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

    fn generate_data_attributes(&mut self, attrs: &[DataAttribute]) {
        for attr in attrs {
            match attr {
                DataAttribute::Other(attr) => self.generate_attribute(attr),
                DataAttribute::Derive {
                    names,
                    trailing_comma,
                } => {
                    let suffix = if *trailing_comma && !names.is_empty() {
                        ","
                    } else {
                        ""
                    };
                    self.writeln(&format!("#[derive({}{suffix})]", names.join(", ")));
                }
            }
        }
    }

    fn generate_attributes(&mut self, attrs: &[Attribute], inner: bool) {
        for attr in attrs.iter().filter(|attr| attr.is_inner() == inner) {
            self.generate_attribute(attr);
        }
    }

    fn generate_attribute(&mut self, attr: &Attribute) {
        let (name, args) = match attr {
            Attribute::Source { meta, inner } => {
                let marker = if *inner { "#!" } else { "#" };
                self.writeln(&format!("{marker}[{meta}]"));
                return;
            }
            Attribute::Structured { name, args } => (name, args),
        };
        self.write(&format!("#[{name}"));
        if !args.is_empty() {
            self.write("(");
            for (i, arg) in args.iter().enumerate() {
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
                    .map(|binding| {
                        format!("{} = {}", binding.name, self.type_to_string(&binding.ty))
                    })
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
                let (lifetime, separator) =
                    lifetime.as_deref().map_or(("", ""), |name| (name, " "));
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
        self.generate_data_attributes(&u.attrs);

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
    use crate::rustlight_ast::{
        Block, CallableTraitQualifier, CallableTraitType, ClosureParam, Expr, FunctionDef,
        GenericParam, Item, Literal, Param, PathType, RustModule, Statement, Type, TypeAlias,
        TypeParameter, Visibility,
    };
    use crate::{BindingMode, Pattern};

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
                qualifiers: Default::default(),
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
                attrs: Vec::new(),
                name: "Callback".to_string(),
                target: Type::Generic("Rc".to_string(), vec![Type::Named("T".to_string())]),
                generics: vec![GenericParam::Type(TypeParameter {
                    name: "T".to_string(),
                    bounds: vec!["Clone".to_string()],
                })]
                .into(),
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
                qualifiers: Default::default(),
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
                qualifiers: Default::default(),
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
                label: None,
                condition: Box::new(Expr::BinaryOp(
                    Box::new(Expr::Ident("i".to_string())),
                    "<".to_string(),
                    Box::new(Expr::Literal(Literal::Int(10))),
                )),
                body: Block {
                    stmts: vec![Statement::Expr(Expr::Break {
                        label: None,
                        value: None,
                    })],
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
                label: None,
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
                    stmts: vec![Statement::Expr(Expr::Continue(None))],
                    expr: None,
                },
            },
            Type::Unit,
        );

        assert!(printed.contains("for i in [1, 2, 3] {\n        continue;\n    }"));
    }
}
