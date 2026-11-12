//! Preorder expression traversal over the RustLight AST.

use crate::{Block, BuilderMethod, Expr, Statement};

/// Visit a block's expressions in source order. Item bodies are separate roots.
pub fn visit_block(block: &mut Block, visit: &mut impl FnMut(&mut Expr) -> bool) {
    Expressions {
        visit,
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
        visit,
        local_functions: true,
    }
    .block(block);
}

/// Visit an expression before its children. Return false to skip its subtree.
pub fn visit_expr(expression: &mut Expr, visit: &mut impl FnMut(&mut Expr) -> bool) {
    Expressions {
        visit,
        local_functions: false,
    }
    .expression(expression);
}

struct Expressions<'a, F> {
    visit: &'a mut F,
    local_functions: bool,
}

impl<F: FnMut(&mut Expr) -> bool> Expressions<'_, F> {
    fn block(&mut self, block: &mut Block) {
        for statement in &mut block.stmts {
            match statement {
                Statement::Let(local) => {
                    if let Some(expr) = &mut local.init {
                        self.expression(expr);
                    }
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
                Statement::Item(_) | Statement::Macro(_) | Statement::Comment(_) => {}
            }
        }
        if let Some(expr) = &mut block.expr {
            self.expression(expr);
        }
    }
    fn expression(&mut self, expression: &mut Expr) {
        if !(self.visit)(expression) {
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
            | Expr::TypedClosure(_, _, inner, _) => self.expression(inner),
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
    }
}
