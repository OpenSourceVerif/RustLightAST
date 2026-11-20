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
            Expr::AsyncClosure { .. }
            | Expr::Closure(..)
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
    measurement: Option<ModuleMeasurement>,
}

#[derive(Default)]
struct ModuleMeasurement {
    bytes: usize,
    ends_with_newline: bool,
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
            measurement: None,
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
                Some(Self::namespace_segments_with_arguments(path, arguments, true))
            }
            PathType::Qualified(_) => None,
        }
    }

    fn namespace_segments_with_arguments<'a>(
        path: &'a [String],
        arguments: &[PathTypeArguments],
        turbofish: bool,
    ) -> Cow<'a, [String]> {
        if arguments.is_empty() {
            return Cow::Borrowed(path);
        }
        let printer = Self::new();
        let mut segments = path.to_vec();
        for list in arguments {
            let segment = &mut segments[list.segment];
            if turbofish {
                segment.push_str("::");
            }
            printer.append_generic_arguments(segment, &list.arguments);
        }
        Cow::Owned(segments)
    }

    /// Render a record constructor path with its explicit generic arguments.
    pub fn generate_record_path_code(&self, path: &RecordPath) -> String {
        match path {
            RecordPath::Segments(segments) => segments.join("::"),
            RecordPath::WithTypeArguments { segments, arguments } => {
                let mut source = segments.join("::");
                source.push_str("::");
                self.append_generic_arguments(&mut source, arguments);
                source
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
        self.generate_module(module);
        self.buffer.clone()
    }

    /// Measure the module's printed size without allocating the output text.
    pub fn measure_module_code(module: &RustModule) -> usize {
        let mut generator = Self {
            measurement: Some(ModuleMeasurement::default()),
            ..Self::new()
        };
        generator.generate_module(module);
        generator.measurement.unwrap().bytes
    }

    fn generate_module(&mut self, module: &RustModule) {
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
            Item::Retained(item) => {
                if let Some(model) = item.modeled_body_item().filter(|_| item.source().is_empty()) {
                    self.generate_item(model);
                } else {
                    self.generate_raw(item.source());
                }
            }
            Item::Macro(item) => self.generate_macro_statement(item),
            Item::Struct(s) => self.generate_struct(s),
            Item::Enum(e) => self.generate_enum(e),
            Item::Union(u) => self.generate_union(u), // New
            Item::Function(f) => self.generate_function(f),
            Item::Impl(i) => self.generate_impl(i),
            Item::Trait(t) => self.generate_trait(t),
            Item::Const(c) => self.generate_const(c),
            Item::Static(s) => self.generate_static(s),
            Item::Foreign(block) => self.generate_foreign_block(block),
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
            self.write(&self.ordered_type_bounds_to_string(&trait_def.supertraits));
        }
        self.write_where_predicates(&trait_def.generics.where_predicates);
        self.writeln(" {");
        self.indent();
        self.generate_attributes(&trait_def.attrs, true);
        for member in &trait_def.members {
            match member {
                TraitMember::DefaultMethod(function) => self.generate_function(function),
                TraitMember::RequiredMethod(function) => self.generate_function_declaration(function),
                TraitMember::AssocConst(associated) => self.generate_associated_const(associated),
                TraitMember::AssocType(associated) => self.generate_associated_type(associated),
                TraitMember::Macro(mac) => self.generate_member_macro(mac),
                TraitMember::RetainedMethod { source, .. } |
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

        let head = self.function_head(
            &f.vis, &f.qualifiers, f.asyncness, false, &f.name, &f.generics,
        );
        let params_and_return = self.function_params_and_return(
            &self.params_to_string(&f.params), &f.return_type, f.qualifiers.explicit_return_type,
        );
        let signature = format!("{}{}", head, params_and_return);
        let where_clauses = f.generics.where_predicates.iter()
            .map(|predicate| self.where_predicate_to_string(predicate))
            .collect::<Vec<_>>();
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

    fn function_head(
        &self, vis: &Visibility, qualifiers: &FunctionQualifiers, asyncness: bool, safe: bool,
        name: &str, generics: &Generics,
    ) -> String {
        let safety = if safe { "safe " } else if qualifiers.unsafety { "unsafe " } else { "" };
        format!(
            "{}{}{}{safety}{}fn {name}{}",
            self.visibility(vis),
            if qualifiers.constness { "const " } else { "" },
            if asyncness { "async " } else { "" },
            Self::abi_prefix(qualifiers.abi.as_ref()),
            self.generic_params_to_string(&generics.params)
        )
    }

    fn function_params_and_return(&self, params: &str, ty: &Type, explicit_return: bool) -> String {
        let mut signature = format!("({params})");
        if explicit_return || *ty != Type::Unit {
            signature.push_str(" -> ");
            signature.push_str(&self.type_to_string(ty));
        }
        signature
    }

    fn generate_function_declaration(&mut self, function: &FunctionDeclaration) {
        for doc in &function.docs {
            self.writeln(doc);
        }
        self.generate_attributes(&function.attrs, false);
        self.write(&self.function_head(
            &function.vis, &function.qualifiers, function.asyncness, function.safe,
            &function.name, &function.generics,
        ));
        let mut params = function.params.iter().enumerate().map(|(index, parameter)| {
            let mut generator = Self::new();
            generator.generate_attributes(&parameter.attrs, false);
            let param = &parameter.param;
            if index == 0 && function.shorthand_receiver {
                generator.write(&self.shorthand_receiver_to_string(param));
            } else {
                generator.write(&self.param_to_string(param));
            }
            generator.buffer
        }).collect::<Vec<_>>();
        if let Some(variadic) = &function.variadic {
            let mut generator = Self::new();
            generator.generate_attributes(&variadic.attrs, false);
            if let Some(pattern) = &variadic.pattern {
                generator.generate_pattern(pattern);
                generator.write(": ");
            }
            generator.write("...");
            params.push(generator.buffer);
        }
        self.write(&self.function_params_and_return(
            &params.join(", "), &function.return_type, function.qualifiers.explicit_return_type,
        ));
        self.write_where_predicates(&function.generics.where_predicates);
        self.writeln(";");
    }

    fn shorthand_receiver_to_string(&self, param: &Param) -> String {
        if let Type::Reference(_, lifetime, mutable) = &param.ty {
            let lifetime = lifetime.as_ref().map_or(String::new(), |lifetime| format!("{lifetime} "));
            let modifier = if *mutable { "mut " } else { "" };
            return format!("&{lifetime}{modifier}self");
        }
        if param.mutable { "mut self".to_owned() } else { "self".to_owned() }
    }

    fn generate_associated_const(&mut self, associated: &AssociatedConstDeclaration) {
        self.generate_attributes(&associated.attrs, false);
        self.write(&format!(
            "const {}{}: {}", associated.name, self.generic_params_to_string(&associated.generics.params),
            self.type_to_string(&associated.ty),
        ));
        self.write_where_predicates(&associated.generics.where_predicates);
        if let Some(value) = &associated.default {
            self.write(" = ");
            self.generate_initializer(value);
        }
        self.writeln(";");
    }

    fn generate_associated_type(&mut self, associated: &AssociatedTypeDeclaration) {
        self.generate_attributes(&associated.attrs, false);
        self.write(&format!("type {}{}", associated.name, self.generic_params_to_string(&associated.generics.params)));
        if !associated.bounds.is_empty() {
            self.write(": ");
            self.write(&self.ordered_type_bounds_to_string(&associated.bounds));
        }
        if let Some(ty) = &associated.default {
            self.write(&format!(" = {}", self.type_to_string(ty)));
        }
        self.write_where_predicates(&associated.generics.where_predicates);
        self.writeln(";");
    }

    fn generate_initializer(&mut self, initializer: &Initializer) {
        match initializer {
            Initializer::Expr(value) => self.generate_expr(value),
            Initializer::Raw(source) => self.write(source),
        }
    }

    fn generate_static(&mut self, item: &StaticDef) {
        for doc in &item.docs {
            self.writeln(doc);
        }
        self.generate_attributes(&item.attrs, false);
        self.write_static_declaration(&item.vis, ForeignSafety::Implicit, item.mutable, &item.name, &item.ty);
        self.write(" = ");
        self.generate_initializer(&item.value);
        self.writeln(";");
        self.writeln("");
    }

    fn write_static_declaration(
        &mut self, visibility: &Visibility, safety: ForeignSafety, mutable: bool, name: &str, ty: &Type,
    ) {
        let safety = match safety {
            ForeignSafety::Implicit => "",
            ForeignSafety::Safe => "safe ",
            ForeignSafety::Unsafe => "unsafe ",
        };
        let mutable = if mutable { "mut " } else { "" };
        self.write(&format!("{}{safety}static {mutable}{name}: {}", self.visibility(visibility), self.type_to_string(ty)));
    }

    fn generate_foreign_block(&mut self, block: &ForeignBlock) {
        self.generate_attributes(&block.attrs, false);
        let unsafety = if block.unsafety { "unsafe " } else { "" };
        self.writeln(&format!("{unsafety}{}{{", Self::abi_prefix(Some(&block.abi))));
        self.indent();
        self.generate_attributes(&block.attrs, true);
        for member in &block.members {
            match member {
                ForeignMember::Function(function) => self.generate_function_declaration(function),
                ForeignMember::Static(item) => {
                    self.generate_attributes(&item.attrs, false);
                    self.write_static_declaration(&item.vis, item.safety, item.mutable, &item.name, &item.ty);
                    self.writeln(";");
                }
                ForeignMember::Type(item) => {
                    self.generate_attributes(&item.attrs, false);
                    self.write(&format!("{}type {}{}", self.visibility(&item.vis), item.name,
                        self.generic_params_to_string(&item.generics.params)));
                    self.write_where_predicates(&item.generics.where_predicates);
                    self.writeln(";");
                }
                ForeignMember::Macro(mac) => self.generate_member_macro(mac),
                ForeignMember::Raw(source) => self.generate_raw(source),
            }
        }
        self.dedent();
        self.writeln("}");
        self.writeln("");
    }

    fn generate_member_macro(&mut self, mac: &MemberMacro) {
        self.generate_attributes(&mac.attrs, false);
        self.generate_macro_invocation(&mac.invocation);
        self.writeln(if mac.trailing_semicolon { ";" } else { "" });
    }

    fn generic_param_to_string(&self, generic: &GenericParam) -> String {
        match generic {
            GenericParam::Type(parameter) => self.type_parameter_to_string(parameter),
            GenericParam::Const { name, ty, default } => {
                let mut parameter = format!("const {name}: {}", self.type_to_string(ty));
                if let Some(value) = default {
                    parameter.push_str(" = ");
                    parameter.push_str(&Self::new().generate_expression_code(value));
                }
                parameter
            }
            GenericParam::Lifetime(parameter) => bounded_name(&parameter.name, &parameter.bounds),
        }
    }

    fn type_parameter_to_string(&self, parameter: &TypeParameter) -> String {
        let mut source = parameter.name.clone();
        if !parameter.bounds.is_empty() {
            source.push_str(": ");
            source.push_str(&self.type_bounds_to_string(&parameter.bounds));
        }
        if let Some(default) = &parameter.default {
            source.push_str(" = ");
            source.push_str(&self.type_to_string(default));
        }
        source
    }

    fn params_to_string(&self, params: &[Param]) -> String {
        params
            .iter()
            .map(|param| self.param_to_string(param))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn param_to_string(&self, param: &Param) -> String {
        if let Some(pattern) = &param.pattern {
            let pattern = RustCodeGenerator::new().generate_pattern_code(pattern);
            format!("{pattern}: {}", self.type_to_string(&param.ty))
        } else if param.name.is_empty() {
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

    fn where_predicate_to_string(&self, predicate: &WherePredicate) -> String {
        match predicate {
            WherePredicate::Type { subject, bounds } => format!(
                "{}: {}",
                self.type_to_string(subject),
                self.type_bounds_to_string(bounds)
            ),
            WherePredicate::Lifetime { subject, bounds } => {
                format!("{subject}: {}", ordered_bounds_to_string(bounds))
            }
        }
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
                self.write("let ");
                self.generate_pattern(&ls.pattern);
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
            Statement::Macro(statement) => self.generate_macro_statement(statement),
            Statement::Item(item) => self.generate_item(item),
            Statement::Comment(comment) => {
                self.writeln(&format!("// {}", comment));
            }
        }
    }

    fn generate_closure(
        &mut self,
        params: &[ClosureParam],
        return_type: Option<&Type>,
        body: &Expr,
        move_capture: bool,
        position: ExpressionPosition,
    ) {
        if move_capture {
            self.write("move ");
        }
        self.write("|");
        for (index, param) in params.iter().enumerate() {
            if index > 0 {
                self.write(", ");
            }
            self.generate_closure_param(param);
        }
        self.write("| ");
        if let Some(return_type) = return_type {
            self.write("-> ");
            self.write(&self.type_to_string(return_type));
            self.write(" ");
            if matches!(body, Expr::Block(_)) {
                self.generate_expr(body);
            } else {
                self.write("{ ");
                self.generate_expr(body);
                self.write(" }");
            }
        } else {
            self.generate_expr_at(body, position.without_statement_head());
        }
    }

    fn generate_range(
        &mut self,
        start: Option<&Expr>,
        end: &RangeEnd,
        position: ExpressionPosition,
    ) {
        if let Some(start) = start {
            self.generate_operand(start, Precedence::Range, false, position);
        }
        self.write(match end {
            RangeEnd::Exclusive(_) => "..",
            RangeEnd::Inclusive(_) => "..=",
        });
        if let Some(end) = end.expression() {
            self.generate_operand(end, Precedence::Range, false, position.without_statement_head());
        }
    }

    fn generate_expr(&mut self, expr: &Expr) {
        self.generate_expr_at(expr, ExpressionPosition::Free);
    }

    fn generate_expr_at(&mut self, expr: &Expr, position: ExpressionPosition) {
        match expr {
            Expr::Ident(id) => self.write(id),
            Expr::Macro(expression) => {
                for attr in &expression.attrs {
                    self.write(attr);
                    self.write(" ");
                }
                self.generate_macro_invocation(&expression.invocation);
            }
            Expr::Path(path, PathType::Namespace) => self.generate_path(path),
            Expr::Path(path, kind @ PathType::NamespaceWithTypeArguments(_)) => {
                let segments = Self::namespace_path_segments(path, kind)
                    .expect("modeled type arguments are on a namespace path");
                self.generate_path(&segments);
            }
            Expr::Path(path, PathType::Qualified(qualification)) => {
                self.write(&self.qualified_path_to_string(path, qualification, true));
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
            Expr::Range { start, end } => self.generate_range(start.as_deref(), end, position),
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
                if method.name.is_empty() {
                    self.generate_callee(receiver, position);
                } else {
                    self.generate_postfix_base(receiver, position);
                    self.write(".");
                    self.write(&method.name);
                    if let Some(arguments) = &method.generic_arguments {
                        let mut turbofish = "::".to_owned();
                        self.append_generic_arguments(&mut turbofish, arguments);
                        self.write(&turbofish);
                    }
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
                self.generate_closure(params, None, body, *is_move, position);
            }
            Expr::TypedClosure(params, return_type, body, is_move) => {
                self.generate_closure(params, Some(return_type), body, *is_move, position);
            }
            Expr::AsyncClosure {
                params,
                return_type,
                body,
                move_capture,
            } => {
                self.write("async ");
                self.generate_closure(params, return_type.as_ref(), body, *move_capture, position);
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
                self.generate_pattern(pattern);
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

    fn generate_macro_statement(&mut self, statement: &crate::MacroStmt) {
        for attr in &statement.attrs {
            self.writeln(attr);
        }
        if let Some(name) = &statement.name {
            let mac = &statement.invocation;
            let (open, close) = mac.delimiter.brackets();
            self.generate_path(&mac.path);
            self.write(&format!("! {name} {open}"));
            self.generate_macro_input(&mac.tokens);
            self.write(close.encode_utf8(&mut [0; 4]));
        } else {
            self.generate_macro_invocation(&statement.invocation);
        }
        self.writeln(if statement.semicolon { ";" } else { "" });
    }

    fn generate_macro_invocation(&mut self, mac: &crate::MacroInvocation) {
        let (open, close) = mac.delimiter.brackets();
        self.generate_path(&mac.path);
        self.write("!");
        self.write(open.encode_utf8(&mut [0; 4]));
        self.generate_macro_input(&mac.tokens);
        self.write(close.encode_utf8(&mut [0; 4]));
    }

    fn generate_macro_input(&mut self, input: &crate::MacroInput) {
        use crate::ModeledMacroInput;
        let Some(arguments) = input.modeled_arguments() else {
            self.write(input.written_source().expect("written macro input"));
            return;
        };
        match &arguments.input {
            ModeledMacroInput::Expressions { arguments, .. } => {
                for (index, expression) in arguments.iter().enumerate() {
                    if index != 0 {
                        self.write(", ");
                    }
                    self.generate_expr(expression);
                }
            }
            ModeledMacroInput::Repeat(value, count) => {
                self.generate_expr(value);
                self.write("; ");
                self.generate_expr(count);
            }
            ModeledMacroInput::Match(input) => {
                self.generate_expr(&input.value);
                self.write(", ");
                self.generate_pattern(&input.pattern);
                if let Some(guard) = &input.guard {
                    self.write(" if ");
                    self.generate_expr(guard);
                }
            }
            ModeledMacroInput::Statics(items) => {
                for (index, item) in items.iter().enumerate() {
                    if index != 0 {
                        self.write("\n");
                    }
                    for attribute in &item.attrs {
                        self.generate_attribute(attribute);
                    }
                    let visibility = self.visibility(&item.visibility);
                    let ty = self.type_to_string(item.ty.as_ref().expect("modeled macro static type"));
                    self.write(&format!("{visibility}static {}: {ty} = ", item.name));
                    self.generate_expr(item.initializer.as_ref().expect("modeled macro static initializer"));
                    self.write(";");
                }
            }
        }
        if arguments.trailing_comma {
            self.write(",");
        }
    }

    fn generate_literal(&mut self, lit: &Literal) {
        match lit {
            Literal::Raw(source)
            | Literal::Byte { spelling: source, .. }
            | Literal::StringToken { spelling: source, .. }
            | Literal::CharToken { spelling: source, .. } => self.write(source),
            Literal::Integer { digits, suffix } | Literal::FloatToken { digits, suffix } => {
                self.write(digits);
                if let Some(suffix) = suffix {
                    self.write(suffix);
                }
            }
            Literal::Int(i) => self.write(&i.to_string()),
            Literal::Float(f) => self.write(&f.to_string()),
            Literal::Str(s) => self.write(&format!("\"{}\"", s)),
            Literal::Bool(b) => self.write(&b.to_string()),
            Literal::Char(c) => self.write(&format!("{c:?}")),
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
        self.generate_pattern(&param.pattern);
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
        self.write_use_tree(&import.tree);
        self.writeln(";");
    }

    fn write_use_tree(&mut self, tree: &UseTree) {
        match tree {
            UseTree::Path { segment, tree } => {
                self.write(segment);
                self.write("::");
                self.write_use_tree(tree);
            }
            UseTree::Name(name) => self.write(name),
            UseTree::Rename { name, alias } => {
                self.write(name);
                self.write(" as ");
                self.write(alias);
            }
            UseTree::Glob => self.write("*"),
            UseTree::Group(items) => {
                self.write("{");
                for (index, item) in items.iter().enumerate() {
                    if index != 0 {
                        self.write(", ");
                    }
                    self.write_use_tree(item);
                }
                self.write("}");
            }
        }
    }

    fn generate_data_attributes(&mut self, attrs: &[DataAttribute]) {
        for attr in attrs {
            match attr {
                DataAttribute::Other(attr) => self.generate_attribute(attr),
                DataAttribute::Derive { paths, trailing_comma } =>
                    self.writeln(&Self::generate_derive_attribute_code(paths, *trailing_comma)),
            }
        }
    }

    /// Print a derive attribute for publication at its source range.
    pub fn generate_derive_attribute_code(paths: &[Vec<String>], trailing_comma: bool) -> String {
        let names = paths.iter().map(|path| path.join("::")).collect::<Vec<_>>();
        let suffix = if trailing_comma && !paths.is_empty() { "," } else { "" };
        format!("#[derive({}{suffix})]", names.join(", "))
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

    fn abi_prefix(abi: Option<&FunctionAbi>) -> String {
        match abi {
            Some(FunctionAbi::Default) => "extern ".to_string(),
            Some(FunctionAbi::Named(name)) => format!("extern {name:?} "),
            None => String::new(),
        }
    }

    fn bare_fn_param_to_string(&self, attrs: &[Attribute], name: Option<&str>, ty: &str) -> String {
        let mut generator = Self::new();
        generator.generate_attributes(attrs, false);
        if let Some(name) = name {
            generator.write(name);
            generator.write(": ");
        }
        generator.write(ty);
        generator.buffer
    }

    fn bare_fn_to_string(&self, function: &BareFnType) -> String {
        let mut source = String::new();
        if !function.higher_ranked.is_empty() {
            let binder = function
                .higher_ranked
                .iter()
                .map(|parameter| bounded_name(&parameter.name, &parameter.bounds))
                .collect::<Vec<_>>()
                .join(", ");
            source.push_str(&format!("for<{binder}> "));
        }
        if function.unsafety {
            source.push_str("unsafe ");
        }
        source.push_str(&Self::abi_prefix(function.abi.as_ref()));
        let mut args = function
            .inputs
            .iter()
            .map(|param| {
                self.bare_fn_param_to_string(
                    &param.attrs,
                    param.name.as_deref(),
                    &self.type_to_string(&param.ty),
                )
            })
            .collect::<Vec<_>>();
        if let Some(variadic) = &function.variadic {
            args.push(self.bare_fn_param_to_string(&variadic.attrs, variadic.name.as_deref(), "..."));
        }
        source.push_str(&format!("fn({})", args.join(", ")));
        if let Some(output) = &function.output {
            source.push_str(" -> ");
            source.push_str(&self.type_to_string(output));
        }
        source
    }

    fn pointee_to_string(&self, ty: &Type) -> String {
        let source = self.type_to_string(ty);
        if matches!(ty, Type::CallableTrait(callable)
            if !callable.leading_bounds.is_empty() || !callable.trailing_bounds.is_empty())
            || matches!(ty, Type::TraitObject { bounds, .. } | Type::ImplTraitBounds(bounds)
                if bounds.len() > 1)
        {
            format!("({source})")
        } else {
            source
        }
    }

    fn append_generic_arguments(&self, source: &mut String, arguments: &[GenericArgument]) {
        source.push('<');
        for (index, argument) in arguments.iter().enumerate() {
            if index > 0 {
                source.push_str(", ");
            }
            self.append_generic_argument(source, argument);
        }
        source.push('>');
    }

    fn append_generic_argument(&self, source: &mut String, argument: &GenericArgument) {
        match argument {
            GenericArgument::Type(ty) => source.push_str(&self.type_to_string(ty)),
            GenericArgument::Lifetime(lifetime) => source.push_str(lifetime),
            GenericArgument::Const(expression) => {
                let expression_source = Self::new().generate_expression_code(expression);
                // Braces keep a rewritten identifier or expression in const syntax.
                let needs_braces =
                    !matches!(expression.as_ref(), Expr::Literal(_) | Expr::Block(_));
                if needs_braces {
                    source.push_str("{ ");
                }
                source.push_str(&expression_source);
                if needs_braces {
                    source.push_str(" }");
                }
            }
        }
    }

    /// Print a generic type bound at the source boundary.
    pub fn generate_type_bound_code(&self, bound: &TypeBound) -> String {
        let trait_bound = match bound {
            TypeBound::Lifetime(source) | TypeBound::Opaque(source) => return source.clone(),
            TypeBound::Trait(bound) => bound,
        };
        let mut source = String::new();
        if trait_bound.parenthesized {
            source.push('(');
        }
        if trait_bound.optional {
            source.push('?');
        }
        if !trait_bound.higher_ranked.is_empty() {
            let binders = trait_bound
                .higher_ranked
                .iter()
                .map(|parameter| bounded_name(&parameter.name, &parameter.bounds))
                .collect::<Vec<_>>()
                .join(", ");
            source.push_str(&format!("for<{binders}> "));
        }
        if trait_bound.absolute {
            source.push_str("::");
        }
        for (index, segment) in trait_bound.path.iter().enumerate() {
            if index > 0 {
                source.push_str("::");
            }
            source.push_str(&segment.name);
            match &segment.arguments {
                TraitBoundArguments::None => {}
                TraitBoundArguments::Angle(arguments) => {
                    self.append_bound_arguments(&mut source, arguments)
                }
                TraitBoundArguments::Parenthesized { inputs, output } => {
                    source.push('(');
                    source.push_str(
                        &inputs
                            .iter()
                            .map(|ty| self.type_to_string(ty))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    source.push(')');
                    if let Some(output) = output {
                        source.push_str(" -> ");
                        source.push_str(&self.type_to_string(output));
                    }
                }
            }
        }
        if trait_bound.parenthesized {
            source.push(')');
        }
        source
    }

    fn append_bound_arguments(&self, source: &mut String, arguments: &[TraitBoundArgument]) {
        source.push('<');
        for (index, argument) in arguments.iter().enumerate() {
            if index > 0 {
                source.push_str(", ");
            }
            match argument {
                TraitBoundArgument::Generic(argument) => {
                    self.append_generic_argument(source, argument)
                }
                TraitBoundArgument::AssociatedType {
                    name,
                    arguments,
                    value,
                } => {
                    source.push_str(name);
                    if let Some(arguments) = arguments {
                        self.append_bound_arguments(source, arguments);
                    }
                    source.push_str(" = ");
                    source.push_str(&self.type_to_string(value));
                }
            }
        }
        source.push('>');
    }

    fn type_bounds_to_string(&self, bounds: &[TypeBound]) -> String {
        bounds
            .iter()
            .filter(|bound| bound.lifetime() != Some("'static"))
            .chain(
                bounds
                    .iter()
                    .filter(|bound| bound.lifetime() == Some("'static")),
            )
            .map(|bound| self.generate_type_bound_code(bound))
            .collect::<Vec<_>>()
            .join(" + ")
    }

    fn type_to_string(&self, ty: &Type) -> String {
        match ty {
            Type::Path(path) => path.join("::"),
            Type::Named(name) => name.clone(),
            Type::Generic(name, arguments) => {
                let mut source = name.clone();
                self.append_generic_arguments(&mut source, arguments);
                source
            }
            Type::PathWithArguments(path, arguments) => {
                Self::namespace_segments_with_arguments(path, arguments, false).join("::")
            }
            Type::AssociatedPath(path, qualification) => {
                self.qualified_path_to_string(path, qualification, false)
            }
            Type::TraitObject { dyn_token, bounds } => {
                let prefix = if *dyn_token { "dyn " } else { "" };
                format!("{prefix}{}", self.ordered_type_bounds_to_string(bounds))
            }
            Type::ImplTraitBounds(bounds) => format!("impl {}", self.ordered_type_bounds_to_string(bounds)),
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
                if callable.parenthesized {
                    source.push('(');
                }
                source.push_str(&format!("{}({args})", callable.trait_path.join("::")));
                if callable.explicit_return_type || *callable.return_type != Type::Unit {
                    source.push_str(" -> ");
                    source.push_str(&self.type_to_string(&callable.return_type));
                }
                if callable.parenthesized {
                    source.push(')');
                }
                for bound in &callable.trailing_bounds {
                    source.push_str(" + ");
                    source.push_str(&callable_bound_to_string(bound));
                }
                source
            }
            Type::BareFn(function) => self.bare_fn_to_string(function),
            Type::RawPointer(inner, kind) => {
                let qualifier = match kind {
                    PointerKind::Const => "const",
                    PointerKind::Mut => "mut",
                };
                format!("*{qualifier} {}", self.pointee_to_string(inner))
            }
            Type::Reference(inner, lifetime, mutable) => {
                let (lifetime, separator) =
                    lifetime.as_deref().map_or(("", ""), |name| (name, " "));
                let mutable = if *mutable { "mut " } else { "" };
                format!("&{lifetime}{separator}{mutable}{}", self.pointee_to_string(inner))
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
            Type::Array(inner, size) => format!(
                "[{}; {}]",
                self.type_to_string(inner),
                Self::new().generate_expression_code(size),
            ),
            Type::Parenthesized(inner) => format!("({})", self.type_to_string(inner)),
            Type::Macro(invocation) => {
                let mut generator = Self::new();
                generator.generate_macro_invocation(invocation);
                generator.buffer
            }
            Type::Unit => "()".to_string(),
            Type::Never => "!".to_string(),
            Type::Infer => "_".to_string(),
        }
    }

    fn ordered_type_bounds_to_string(&self, bounds: &[TypeBound]) -> String {
        bounds.iter().map(|bound| self.generate_type_bound_code(bound))
            .collect::<Vec<_>>().join(" + ")
    }

    fn qualified_path_to_string(
        &self,
        path: &[String],
        qualification: &PathQualification,
        turbofish: bool,
    ) -> String {
        let mut source = String::from("<");
        match &qualification.self_type {
            QualifiedSelfType::Type(ty) => source.push_str(&self.type_to_string(ty)),
            QualifiedSelfType::Verbatim(written) => source.push_str(written),
        }
        if let Some(trait_path) = &qualification.trait_path {
            source.push_str(" as ");
            source.push_str(&Self::namespace_segments_with_arguments(
                trait_path, &qualification.trait_arguments, false,
            ).join("::"));
        }
        source.push_str(">::");
        source.push_str(&Self::namespace_segments_with_arguments(
            path, &qualification.associated_arguments, turbofish,
        ).join("::"));
        source
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
        if let Some(measurement) = &mut self.measurement {
            measurement.bytes += 1;
            measurement.ends_with_newline = true;
        } else {
            self.buffer.push('\n');
        }
    }

    fn write(&mut self, s: &str) {
        if let Some(measurement) = &mut self.measurement {
            if measurement.bytes == 0 || measurement.ends_with_newline {
                measurement.bytes += 4 * self.indent_level;
            }
            measurement.bytes += s.len();
            if !s.is_empty() || self.indent_level > 0 {
                measurement.ends_with_newline = s.ends_with('\n');
            }
            return;
        }
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
    match &use_stmt.tree {
        UseTree::Path { segment, .. } => Some(segment),
        UseTree::Name(name) | UseTree::Rename { name, .. } => Some(name),
        UseTree::Group(_) | UseTree::Glob => None,
    }
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
