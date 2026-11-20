//! Occurrence identities for nodes in a RustLight model.

use std::cell::RefCell;

use crate::{
    visit::{self, Visit},
    Block, ClosureParam, Expr, FunctionDef, ImplBlock, Item, LetStmt, MacroInvocation, Param,
    Pattern, RustModule, Statement, TraitDef, Type,
};

use super::SourceSnapshotId;

mod conversion;
pub(super) use conversion::*;

/// An occurrence in the lowered model's traversal, including its descendants.
#[derive(Clone, Debug)]
pub struct ModelNodeOrigin {
    pub snapshot: SourceSnapshotId,
    pub source_address: usize,
    pub subtree_end: usize,
}

#[derive(Clone, Debug)]
pub struct ExpressionOrigin {
    pub node: ModelNodeOrigin,
}

#[derive(Clone, Debug)]
pub struct TypeOrigin {
    pub node: ModelNodeOrigin,
}

#[derive(Clone, Debug)]
pub struct MacroOrigin {
    pub node: ModelNodeOrigin,
    pub has_attributes: bool,
}

#[derive(Clone, Debug)]
pub struct FunctionOrigin {
    pub node: ModelNodeOrigin,
    pub source_address: usize,
}

#[allow(unused_variables)]
pub trait ModelOriginVisitor<'model> {
    fn unmatched(&mut self) {}
    fn item(&mut self, model: &'model Item, origin: ModelNodeOrigin) {}
    fn leave_item(&mut self, model: &'model Item) {}
    fn function(&mut self, model: &'model FunctionDef, origin: FunctionOrigin) {}
    fn leave_function(&mut self, model: &'model FunctionDef) {}
    fn signature(&mut self, model: &'model FunctionDef, origin: ModelNodeOrigin) {}
    fn initializer(&mut self, model: &'model Expr, source_declaration_address: usize) {}
    fn impl_block(&mut self, model: &'model ImplBlock, origin: ModelNodeOrigin) {}
    fn trait_definition(&mut self, model: &'model TraitDef, origin: ModelNodeOrigin) {}
    fn parameter(&mut self, model: &'model Param, origin: ModelNodeOrigin) {}
    fn block(&mut self, model: &'model Block, origin: ModelNodeOrigin) {}
    fn leave_block(&mut self, model: &'model Block) {}
    fn block_expression(&mut self, model: &'model Block, origin: ModelNodeOrigin) {}
    fn statement(&mut self, model: &'model Statement, origin: ModelNodeOrigin) {}
    fn binding(&mut self, model: &'model LetStmt, origin: ModelNodeOrigin) {}
    fn closure_parameter(&mut self, model: &'model ClosureParam, origin: ModelNodeOrigin) {}
    fn condition(&mut self, model: &'model Pattern, origin: ModelNodeOrigin) {}
    fn expression(&mut self, model: &'model Expr, origin: ExpressionOrigin) {}
    fn leave_expression(&mut self, model: &'model Expr) {}
    fn expression_alias(&mut self, model: &'model Expr, origin: ExpressionOrigin) {}
    fn transparent_expression(&mut self, origin: ExpressionOrigin) {}
    fn pattern(&mut self, model: &'model Pattern, origin: ModelNodeOrigin) {}
    fn pattern_alias(&mut self, model: &'model Pattern, origin: ModelNodeOrigin) {}
    fn ty(&mut self, model: &'model Type, origin: TypeOrigin) {}
    fn macro_invocation(&mut self, model: &'model MacroInvocation, origin: MacroOrigin) {}
    fn leave_macro_invocation(&mut self, model: &'model MacroInvocation) {}
    #[doc(hidden)]
    fn visit_macro_operands(&self) -> bool {
        true
    }
}

pub(in crate::rustlight_parser) fn visit_file<'model>(
    origins: &SourceOrigins,
    snapshot: SourceSnapshotId,
    model: &'model RustModule,
    visitor: &mut impl ModelOriginVisitor<'model>,
) {
    let mut walker = Origins {
        reader: ReadOrigins::new(origins, snapshot),
        visitor,
    };
    walker.visit_module(model);
    walker.finish_scope();
}

/// Install the exact structural copy before exposing any references to it.
pub(in crate::rustlight_parser) fn clone_file<'model>(
    origins: &SourceOrigins,
    snapshot: SourceSnapshotId,
    original: &RustModule,
    storage: &'model mut Option<RustModule>,
    visitor: &mut impl ModelOriginVisitor<'model>,
) -> &'model RustModule {
    let copy = storage.insert(original.clone());
    visit_file(origins, snapshot, copy, visitor);
    copy
}

struct Origins<'a, V> {
    reader: ReadOrigins<'a>,
    visitor: &'a mut V,
}

impl<'model, V: ModelOriginVisitor<'model>> Origins<'_, V> {
    fn finish_scope(&mut self) {
        if self.reader.has_unvisited() {
            self.visitor.unmatched();
        }
    }

    fn in_scope(&mut self, node: &ReadNode<'_>, visit: impl FnOnce(&mut Self)) {
        let scope = self.reader.enter(node);
        visit(self);
        self.finish_scope();
        self.reader.leave(scope);
    }

    fn declaration(&mut self, model: &'model Item, origin: &ReadNode<'_>) {
        if let Item::Macro(mac) = model {
            self.visit_macro_invocation(&mac.invocation);
            return;
        }
        if let Item::Retained(retained) = model {
            if let Some(macro_syntax) = retained.macro_syntax() {
                self.visit_macro_invocation(macro_syntax.invocation());
            }
            if let Some(body) = retained
                .modeled_body_item()
                .or_else(|| retained.conditional_item())
            {
                self.visitor.item(body, origin.node());
                self.declaration(body, origin);
                self.visitor.leave_item(body);
                return;
            }
            if let Some(crate::RetainedDeclaration::Function {
                signature: Some(function),
                ..
            }) = retained.declaration()
            {
                if let Some(signature) = self.reader.take(Kind::Signature) {
                    self.visitor.signature(function, signature.node());
                    self.in_scope(&signature, |this| this.signature(function));
                }
            }
            return;
        }

        if let Item::Impl(implementation) = model {
            if let Some(node) = self.reader.take(Kind::Impl) {
                self.visitor.impl_block(implementation, node.node());
                self.in_scope(&node, |this| visit::walk_item(this, model));
            } else {
                self.visitor.unmatched();
            }
        } else if let Item::Trait(definition) = model {
            if let Some(node) = self.reader.take(Kind::Trait) {
                self.visitor.trait_definition(definition, node.node());
                self.in_scope(&node, |this| visit::walk_item(this, model));
            } else {
                self.visitor.unmatched();
            }
        } else {
            if let Item::Const(definition) = model {
                self.visitor
                    .initializer(&definition.value, origin.node().source_address);
            } else if let Item::Static(definition) = model {
                if let crate::Initializer::Expr(initializer) = &definition.value {
                    self.visitor
                        .initializer(initializer, origin.node().source_address);
                }
            }
            visit::walk_item(self, model);
        }
    }

    fn parameters(&mut self, parameters: impl IntoIterator<Item = &'model Param>) {
        for parameter in parameters {
            let Some(node) = self.reader.take(Kind::Parameter) else {
                self.visitor.unmatched();
                continue;
            };
            self.visitor.parameter(parameter, node.node());
            self.in_scope(&node, |this| {
                if let Some(pattern) = &parameter.pattern {
                    this.visit_pattern(pattern);
                }
                this.visit_type(&parameter.ty);
            });
        }
    }

    fn signature(&mut self, function: &'model FunctionDef) {
        self.visit_generics(&function.generics);
        self.parameters(&function.params);
        self.visit_type(&function.return_type);
    }

    fn block_contents(&mut self, model: &'model Block, node: &ReadNode<'_>) {
        if node.kind() == Kind::Block {
            self.visitor.block(model, node.node());
        }
        self.in_scope(node, |this| visit::walk_block(this, model));
        if node.kind() == Kind::Block {
            self.visitor.leave_block(model);
        }
    }

    fn pattern_origin(&mut self, model: &'model Pattern, node: &ReadNode<'_>) {
        match node.kind() {
            Kind::PatternAlias => {
                self.visitor.pattern_alias(model, node.node());
                self.in_scope(node, |this| this.visit_pattern(model));
            }
            Kind::Condition => {
                self.visitor.transparent_expression(node.expression());
                self.visitor.condition(model, node.node());
                self.in_scope(node, |this| this.visit_pattern(model));
            }
            Kind::Pattern => {
                self.visitor.pattern(model, node.node());
                self.in_scope(node, |this| visit::walk_pattern(this, model));
            }
            _ => unreachable!(),
        }
    }


    fn closure_parameters(&mut self, parameters: &'model [ClosureParam]) {
        for parameter in parameters {
            let Some(node) = self.reader.take(Kind::ClosureParameter) else {
                self.visitor.unmatched();
                continue;
            };
            self.in_scope(&node, |this| {
                if let Some(pattern) = this.reader.peek(Kind::Pattern) {
                    this.visitor.closure_parameter(parameter, pattern.node());
                }
                this.visit_pattern(&parameter.pattern);
                if let Some(ty) = &parameter.ty {
                    this.visit_type(ty);
                }
            });
        }
    }
}

impl<'model, V: ModelOriginVisitor<'model>> Visit<'model> for Origins<'_, V> {
    fn visit_module(&mut self, model: &'model RustModule) {
        // Inner attributes precede declarations as raw items in the model.
        let skip = self.reader.inner_attributes();
        for item in &model.items[skip..] {
            self.visit_item(item);
        }
    }

    fn visit_item(&mut self, model: &'model Item) {
        let Some(node) = self.reader.take(Kind::Item) else {
            self.visitor.unmatched();
            return;
        };
        self.visitor.item(model, node.node());
        self.in_scope(&node, |this| this.declaration(model, &node));
        self.visitor.leave_item(model);
    }

    fn visit_function(&mut self, model: &'model FunctionDef) {
        let Some(node) = self.reader.take(Kind::Function) else {
            self.visitor.unmatched();
            return;
        };
        self.visitor.function(model, node.function());
        self.in_scope(&node, |this| {
            this.signature(model);
            this.visit_block(&model.body);
        });
        self.visitor.leave_function(model);
    }

    fn visit_function_declaration(&mut self, model: &'model crate::FunctionDeclaration) {
        self.visit_generics(&model.generics);
        self.parameters(model.params.iter().map(|parameter| &parameter.param));
        if let Some(pattern) = model
            .variadic
            .as_ref()
            .and_then(|variadic| variadic.pattern.as_ref())
        {
            self.visit_pattern(pattern);
        }
        self.visit_type(&model.return_type);
    }

    fn visit_block(&mut self, model: &'model Block) {
        let Some(node) = self.reader.take(Kind::Block) else {
            self.visitor.unmatched();
            return;
        };
        if node.kind() == Kind::BlockExpression {
            self.visitor.transparent_expression(node.expression());
            self.visitor.block_expression(model, node.node());
            self.in_scope(&node, |this| this.visit_block(model));
        } else {
            self.block_contents(model, &node);
        }
    }

    fn visit_statement(&mut self, model: &'model Statement) {
        let Some(node) = self.reader.take(Kind::Statement) else {
            self.visitor.unmatched();
            return;
        };
        self.visitor.statement(model, node.node());
        self.in_scope(&node, |this| {
            if let Statement::Let(binding) = model {
                if let Some(node) = this.reader.take(Kind::Binding) {
                    this.in_scope(&node, |this| {
                        if let Some(pattern) = this.reader.peek(Kind::Pattern) {
                            this.visitor.binding(binding, pattern.node());
                        }
                        visit::walk_statement(this, model);
                    });
                } else {
                    this.visitor.unmatched();
                }
            } else {
                visit::walk_statement(this, model);
            }
        });
    }

    fn visit_expr(&mut self, model: &'model Expr) {
        let Some(node) = self.reader.take(Kind::Expression) else {
            self.visitor.unmatched();
            return;
        };
        self.visitor.expression(model, node.expression());
        self.in_scope(&node, |this| match model {
            Expr::Closure(params, body, _) => {
                this.closure_parameters(params);
                this.visit_expr(body);
            }
            Expr::TypedClosure(params, ty, body, _) => {
                this.closure_parameters(params);
                this.visit_type(ty);
                this.visit_expr(body);
            }
            Expr::AsyncClosure {
                params,
                return_type,
                body,
                ..
            } => {
                this.closure_parameters(params);
                if let Some(ty) = return_type {
                    this.visit_type(ty);
                }
                this.visit_expr(body);
            }
            Expr::Block(block) | Expr::LabeledBlock { body: block, .. } => {
                this.visitor.block_expression(block, node.node());
                this.visit_block(block);
            }
            _ => visit::walk_expr(this, model),
        });
        self.visitor.leave_expression(model);
    }

    fn visit_pattern(&mut self, model: &'model Pattern) {
        if let Some(node) = self.reader.take(Kind::Pattern) {
            self.pattern_origin(model, &node);
        } else {
            self.visitor.unmatched();
        }
    }

    fn visit_type(&mut self, model: &'model Type) {
        let Some(node) = self.reader.take(Kind::Type) else {
            self.visitor.unmatched();
            return;
        };
        if node.kind() == Kind::Type {
            self.visitor.ty(model, node.ty());
            self.in_scope(&node, |this| visit::walk_type(this, model));
        }
    }

    fn visit_macro_invocation(&mut self, model: &'model MacroInvocation) {
        let Some(node) = self.reader.take(Kind::Macro) else {
            self.visitor.unmatched();
            return;
        };
        let origin = node.macro_invocation();
        self.visitor.macro_invocation(model, origin);
        if self.visitor.visit_macro_operands() {
            self.in_scope(&node, |this| walk_macro_operands(this, model));
        }
        self.visitor.leave_macro_invocation(model);
    }
}

fn walk_macro_operands<'model>(visitor: &mut impl Visit<'model>, invocation: &'model MacroInvocation) {
    if let Some(arguments) = invocation.expression_arguments() {
        for argument in arguments {
            visitor.visit_expr(argument);
        }
    }
    if let Some(([value], Some(count))) = invocation.vector_input() {
        visitor.visit_expr(value);
        visitor.visit_expr(count);
    }
    if let Some(input) = invocation.match_input() {
        visitor.visit_expr(&input.value);
        visitor.visit_pattern(&input.pattern);
        if let Some(guard) = &input.guard {
            visitor.visit_expr(guard);
        }
    }
    if let Some(statics) = invocation.thread_local_statics() {
        for declaration in statics {
            if let Some(ty) = &declaration.ty {
                visitor.visit_type(ty);
            }
            if let Some(initializer) = &declaration.initializer {
                visitor.visit_expr(initializer);
            }
        }
    }
}
