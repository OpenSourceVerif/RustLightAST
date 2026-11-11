//! Mutable expression traversal over the RustLight AST.
//!
//! The block and expression walkers follow executable code. To traverse a
//! const generic argument, pass its expression directly to [`visit_expr`].

use crate::{Block, BuilderMethod, Expr, LetStmt, MacroStmt, Statement};

/// Visit a block's expressions in source order. Item bodies are separate roots.
pub fn visit_block(block: &mut Block, visit: &mut impl FnMut(&mut Expr) -> bool) {
    Expressions {
        callback: Preorder(visit),
        local_functions: false,
    }
    .block(block);
}

/// Visit expressions in a block and in its local function declarations.
pub fn visit_block_with_local_functions(
    block: &mut Block,
    visit: &mut impl FnMut(&mut Expr) -> bool,
) {
    Expressions {
        callback: Preorder(visit),
        local_functions: true,
    }
    .block(block);
}

/// Visit an expression before its children. Return false to skip its subtree.
pub fn visit_expr(expression: &mut Expr, visit: &mut impl FnMut(&mut Expr) -> bool) {
    Expressions {
        callback: Preorder(visit),
        local_functions: false,
    }
    .expression(expression);
}

/// Visit a block's expressions after their children. Item bodies are separate roots.
pub fn visit_block_postorder(block: &mut Block, visit: &mut impl FnMut(&mut Expr)) {
    visit_block_postorder_with_locals(block, visit, &mut |_| {});
}

/// Visit expressions after their children and bindings after their initializer.
pub fn visit_block_postorder_with_locals(
    block: &mut Block,
    visit: &mut impl FnMut(&mut Expr),
    visit_local: &mut impl FnMut(&mut LetStmt),
) {
    Expressions {
        callback: Postorder(visit, visit_local),
        local_functions: false,
    }
    .block(block);
}

/// Visit expressions and blocks after their children. Item bodies are separate roots.
pub fn visit_block_postorder_with_blocks(
    block: &mut Block,
    visit: &mut impl FnMut(&mut Expr),
    leave_block: &mut impl FnMut(&mut Block),
) {
    Expressions {
        callback: PostorderBlocks(visit, leave_block),
        local_functions: false,
    }
    .block(block);
}

/// A macro invocation inside an expression or a statement.
pub enum MacroNodeMut<'a> {
    Expression(&'a mut String),
    Statement(&'a mut MacroStmt),
}

/// Visit a block's macro invocations in source order. The payload is opaque.
pub fn visit_block_macros(block: &mut Block, visit: &mut impl FnMut(MacroNodeMut<'_>)) {
    Expressions {
        callback: Macros(visit),
        local_functions: false,
    }
    .block(block);
}

trait Callback {
    fn enter(&mut self, _: &mut Expr) -> bool {
        true
    }

    fn leave(&mut self, _: &mut Expr) {}

    fn leave_block(&mut self, _: &mut Block) {}

    fn leave_local(&mut self, _: &mut LetStmt) {}

    fn leave_macro_statement(&mut self, _: &mut MacroStmt) {}
}

struct Preorder<F>(F);

impl<F: FnMut(&mut Expr) -> bool> Callback for Preorder<F> {
    fn enter(&mut self, expression: &mut Expr) -> bool {
        (self.0)(expression)
    }
}

struct Postorder<F, L>(F, L);

impl<F: FnMut(&mut Expr), L: FnMut(&mut LetStmt)> Callback for Postorder<F, L> {
    fn leave(&mut self, expression: &mut Expr) {
        (self.0)(expression);
    }

    fn leave_local(&mut self, local: &mut LetStmt) {
        (self.1)(local)
    }
}

struct PostorderBlocks<E, B>(E, B);

impl<E: FnMut(&mut Expr), B: FnMut(&mut Block)> Callback for PostorderBlocks<E, B> {
    fn leave(&mut self, expression: &mut Expr) {
        (self.0)(expression)
    }

    fn leave_block(&mut self, block: &mut Block) {
        (self.1)(block)
    }
}

struct Macros<F>(F);

impl<F: FnMut(MacroNodeMut<'_>)> Callback for Macros<F> {
    fn leave(&mut self, expression: &mut Expr) {
        if let Expr::Macro(source) = expression {
            (self.0)(MacroNodeMut::Expression(source));
        }
    }

    fn leave_macro_statement(&mut self, statement: &mut MacroStmt) {
        (self.0)(MacroNodeMut::Statement(statement));
    }
}

struct Expressions<C> {
    callback: C,
    local_functions: bool,
}

impl<C: Callback> Expressions<C> {
    fn block(&mut self, block: &mut Block) {
        for statement in &mut block.stmts {
            match statement {
                Statement::Let(local) => {
                    if let Some(expr) = &mut local.init {
                        self.expression(expr);
                    }
                    self.callback.leave_local(local);
                }
                Statement::LetElse {
                    value, else_block, ..
                } => {
                    self.expression(value);
                    self.block(else_block);
                }
                Statement::Expr(expr) => self.expression(expr),
                Statement::Item(item) if self.local_functions => {
                    if let crate::Item::Function(function) = item.as_mut() {
                        self.block(&mut function.body);
                    }
                }
                Statement::Macro(statement) => self.callback.leave_macro_statement(statement),
                Statement::Item(_) | Statement::Comment(_) => {}
            }
        }
        if let Some(expr) = &mut block.expr {
            self.expression(expr);
        }
        self.callback.leave_block(block);
    }
    fn expression(&mut self, expression: &mut Expr) {
        if !self.callback.enter(expression) {
            return;
        }
        match expression {
            Expr::Call(receiver, args) | Expr::MethodCall(receiver, _, args) => {
                self.expression(receiver);
                for arg in args {
                    self.expression(arg);
                }
            }
            Expr::Field { base, .. } | Expr::Try(base) => self.expression(base),
            Expr::Repeat { value, len } => {
                self.expression(value);
                self.expression(len);
            }
            Expr::Range { start, end } => {
                for bound in start.as_deref_mut().into_iter().chain(end.expression_mut()) {
                    self.expression(bound);
                }
            }
            Expr::Record { fields, rest, .. } => {
                for field in fields {
                    self.expression(&mut field.value);
                }
                if let Some(base) = rest {
                    self.expression(base);
                }
            }
            Expr::Tuple(items) | Expr::Array(items) => {
                for item in items {
                    self.expression(item);
                }
            }
            Expr::Block(block)
            | Expr::LabeledBlock { body: block, .. }
            | Expr::Async { body: block, .. } => self.block(block),
            Expr::Loop(block, _) | Expr::Unsafe(block) => self.block(block),
            Expr::Let { value, .. }
            | Expr::Return(Some(value))
            | Expr::Break {
                value: Some(value), ..
            } => {
                self.expression(value);
            }
            Expr::While {
                condition, body, ..
            } => {
                self.expression(condition);
                self.block(body);
            }
            Expr::For { iter, body, .. } => {
                self.expression(iter);
                self.block(body);
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expression(condition);
                self.block(then_branch);
                if let Some(block) = else_branch {
                    self.block(block);
                }
            }
            Expr::IfLet {
                value,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(value);
                self.block(then_branch);
                if let Some(block) = else_branch {
                    self.block(block);
                }
            }
            Expr::Match { expr, arms } => {
                self.expression(expr);
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        self.expression(guard);
                    }
                    self.block(&mut arm.body);
                }
            }
            Expr::Reference(inner, _, _)
            | Expr::UnaryOp(_, inner)
            | Expr::Parenthesized(inner)
            | Expr::Cast(inner, _)
            | Expr::Await(inner)
            | Expr::Closure(_, inner, _)
            | Expr::TypedClosure(_, _, inner, _)
            | Expr::AsyncClosure { body: inner, .. } => self.expression(inner),
            Expr::BinaryOp(left, _, right)
            | Expr::Index(left, right)
            | Expr::Assign(left, right) => {
                self.expression(left);
                self.expression(right);
            }
            Expr::BuilderChain(methods) => {
                for method in methods {
                    if let BuilderMethod::Spawn { closure, .. } = method {
                        self.expression(closure);
                    }
                }
            }
            Expr::Ident(_)
            | Expr::Macro(_)
            | Expr::Path(_, _)
            | Expr::Literal(_)
            | Expr::Return(None)
            | Expr::Break { value: None, .. }
            | Expr::Continue(_) => {}
        }
        self.callback.leave(expression);
    }
}
