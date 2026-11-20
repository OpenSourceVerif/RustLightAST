use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(in crate::rustlight_parser) enum Kind {
    Item,
    Function,
    Signature,
    Impl,
    Trait,
    Parameter,
    Block,
    SyntheticBlock,
    Statement,
    Binding,
    ClosureParameter,
    Condition,
    Expression,
    BlockExpression,
    Pattern,
    PatternAlias,
    Type,
    SyntheticType,
    Macro,
    InnerAttributes,
}

#[derive(Default)]
pub(in crate::rustlight_parser) struct SourceOrigins {
    nodes: Vec<Occurrence>,
    details: Vec<Details>,
}

/// A conversion scope owns the segment produced by its child conversions.
/// Syntax discarded by conversion never enters an enclosing scope.
struct Occurrence {
    kind: Kind,
    end: u32,
    details: Option<std::num::NonZeroU32>,
}

enum Details {
    Macro { has_attributes: bool },
    InnerAttributes(usize),
}

// All scoped conversion happens synchronously on the invoking thread. An
// isolated fragment converter can use the same entry points without a tape.
thread_local! {
    static CAPTURE: RefCell<Option<SourceOrigins>> = const { RefCell::new(None) };
}

// A nested or temporarily suspended capture restores its enclosing capture
// even if a converter panics.
struct Capture(Option<SourceOrigins>);

impl Drop for Capture {
    fn drop(&mut self) {
        CAPTURE.with_borrow_mut(|slot| *slot = self.0.take());
    }
}

pub(in crate::rustlight_parser) fn capture<T>(
    convert: impl FnOnce() -> T,
) -> (T, SourceOrigins) {
    let capture = Capture(CAPTURE.with_borrow_mut(|slot| slot.replace(SourceOrigins::default())));
    let model = convert();
    let origins = CAPTURE.with_borrow_mut(|slot| slot.take()).unwrap();
    drop(capture);
    (model, origins)
}

pub(in crate::rustlight_parser) fn without_origins<T>(convert: impl FnOnce() -> T) -> T {
    let _capture = Capture(CAPTURE.with_borrow_mut(Option::take));
    convert()
}

#[derive(Clone, Copy)]
struct Mark {
    start: usize,
    details: usize,
}

fn mark() -> Option<Mark> {
    CAPTURE.with_borrow(|slot| {
        let origins = slot.as_ref()?;
        Some(Mark {
            start: origins.nodes.len(),
            details: origins.details.len(),
        })
    })
}

fn rollback(mark: Mark) {
    CAPTURE.with_borrow_mut(|slot| {
        let origins = slot.as_mut().unwrap();
        origins.nodes.truncate(mark.start);
        origins.details.truncate(mark.details);
    });
}

pub(in crate::rustlight_parser) fn attempt<T, E>(
    convert: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let mark = mark();
    let value = convert();
    if let (Some(mark), Err(_)) = (mark, &value) {
        rollback(mark);
    }
    value
}

fn occurrence_index(index: usize) -> u32 {
    u32::try_from(index).expect("source origin capacity exceeded")
}

fn append(origins: &mut SourceOrigins, kind: Kind, details: Option<Details>) -> usize {
    let details = details.map(|details| {
        let index = occurrence_index(origins.details.len() + 1);
        origins.details.push(details);
        std::num::NonZeroU32::new(index).unwrap()
    });
    let index = origins.nodes.len();
    origins.nodes.push(Occurrence {
        kind,
        end: 0,
        details,
    });
    index
}

fn start(kind: Kind, details: Option<Details>) -> usize {
    CAPTURE.with_borrow_mut(|slot| append(slot.as_mut().unwrap(), kind, details))
}

pub(in crate::rustlight_parser) fn inner_attributes(count: usize) {
    if count == 0 {
        return;
    }
    CAPTURE.with_borrow_mut(|slot| {
        if let Some(origins) = slot {
            let index = append(
                origins,
                Kind::InnerAttributes,
                Some(Details::InnerAttributes(count)),
            );
            origins.nodes[index].end = occurrence_index(index + 1);
        }
    });
}

fn finish(index: usize) {
    CAPTURE.with_borrow_mut(|slot| {
        let origins = slot.as_mut().unwrap();
        origins.nodes[index].end = occurrence_index(origins.nodes.len());
    });
}

fn record<T>(
    kind: Kind,
    details: Option<Details>,
    convert: impl FnOnce() -> T,
) -> T {
    let enabled = CAPTURE.with_borrow(Option::is_some);
    if enabled {
        let index = start(kind, details);
        let value = convert();
        finish(index);
        value
    } else {
        convert()
    }
}

pub(in crate::rustlight_parser) fn node<T>(kind: Kind, convert: impl FnOnce() -> T) -> T {
    record(kind, None, convert)
}

pub(in crate::rustlight_parser) fn try_node<T, E>(
    kind: Kind,
    convert: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    attempt(|| node(kind, convert))
}

pub(in crate::rustlight_parser) fn macro_invocation<T>(
    has_attributes: bool,
    convert: impl FnOnce() -> T,
) -> T {
    record(Kind::Macro, Some(Details::Macro { has_attributes }), convert)
}

fn channel(kind: Kind) -> Kind {
    match kind {
        Kind::SyntheticBlock | Kind::BlockExpression => Kind::Block,
        Kind::PatternAlias | Kind::Condition => Kind::Pattern,
        Kind::SyntheticType => Kind::Type,
        _ => kind,
    }
}

// Converters and the model walker agree on the order of siblings in each
// category. The categories allow independent fields to be read in a different
// cross-category order, for example a function's signature and body.
const CHANNEL_COUNT: usize = Kind::InnerAttributes as usize + 1;

pub(super) struct ReadOrigins<'a> {
    origins: &'a SourceOrigins,
    snapshot: SourceSnapshotId,
    scope: Scope,
}

pub(super) struct Scope {
    end: usize,
    next: [usize; CHANNEL_COUNT],
    remaining: usize,
}

impl Scope {
    fn new(origins: &SourceOrigins, start: usize, end: usize) -> Self {
        let mut index = start;
        let mut remaining = 0;
        let mut next = [end; CHANNEL_COUNT];
        while index < end {
            let occurrence = &origins.nodes[index];
            let channel = channel(occurrence.kind) as usize;
            if next[channel] == end {
                next[channel] = index;
            }
            remaining += 1;
            index = occurrence.end as usize;
        }
        Self {
            end,
            next,
            remaining,
        }
    }
}

pub(super) struct ReadNode<'a> {
    index: usize,
    occurrence: &'a Occurrence,
    details: Option<&'a Details>,
    snapshot: SourceSnapshotId,
}

impl<'a> ReadOrigins<'a> {
    pub(super) fn new(origins: &'a SourceOrigins, snapshot: SourceSnapshotId) -> Self {
        Self {
            origins,
            snapshot,
            scope: Scope::new(origins, 0, origins.nodes.len()),
        }
    }

    pub(super) fn peek(&self, kind: Kind) -> Option<ReadNode<'a>> {
        let route = channel(kind);
        let index = self.scope.next[route as usize];
        if index == self.scope.end {
            return None;
        }
        let occurrence = &self.origins.nodes[index];
        Some(ReadNode {
            index,
            occurrence,
            details: occurrence
                .details
                .map(|details| &self.origins.details[details.get() as usize - 1]),
            snapshot: self.snapshot,
        })
    }

    pub(super) fn take(&mut self, kind: Kind) -> Option<ReadNode<'a>> {
        let node = self.peek(kind)?;
        let route = channel(kind);
        let mut next = node.occurrence.end as usize;
        while next < self.scope.end {
            let candidate = &self.origins.nodes[next];
            if channel(candidate.kind) == route {
                break;
            }
            next = candidate.end as usize;
        }
        self.scope.next[route as usize] = next;
        self.scope.remaining -= 1;
        Some(node)
    }

    pub(super) fn inner_attributes(&mut self) -> usize {
        match self
            .take(Kind::InnerAttributes)
            .and_then(|node| node.details)
        {
            Some(Details::InnerAttributes(count)) => *count,
            None => 0,
            _ => unreachable!(),
        }
    }

    pub(super) fn has_unvisited(&self) -> bool {
        self.scope.remaining != 0
    }

    pub(super) fn enter(&mut self, node: &ReadNode<'_>) -> Scope {
        let scope = Scope::new(self.origins, node.index + 1, node.occurrence.end as usize);
        std::mem::replace(&mut self.scope, scope)
    }

    pub(super) fn leave(&mut self, scope: Scope) {
        self.scope = scope;
    }
}

impl ReadNode<'_> {
    pub(super) fn kind(&self) -> Kind {
        self.occurrence.kind
    }

    pub(super) fn node(&self) -> ModelNodeOrigin {
        ModelNodeOrigin {
            snapshot: self.snapshot,
            source_address: self.index,
            subtree_end: self.occurrence.end as usize,
        }
    }

    pub(super) fn expression(&self) -> ExpressionOrigin {
        ExpressionOrigin { node: self.node() }
    }

    pub(super) fn ty(&self) -> TypeOrigin {
        TypeOrigin { node: self.node() }
    }

    pub(super) fn function(&self) -> FunctionOrigin {
        FunctionOrigin { node: self.node(), source_address: self.index }
    }

    pub(super) fn macro_invocation(&self) -> MacroOrigin {
        let Some(Details::Macro { has_attributes }) = self.details else {
            unreachable!("only macro conversion produces invocation metadata")
        };
        MacroOrigin { node: self.node(), has_attributes: *has_attributes }
    }
}

pub(in crate::rustlight_parser) fn index_model(model: &RustModule) -> SourceOrigins {
    let mut index = IndexModel { origins: SourceOrigins::default(), macro_attributes: false };
    index.visit_module(model);
    index.origins
}

struct IndexModel {
    origins: SourceOrigins,
    macro_attributes: bool,
}

impl IndexModel {
    fn scope(
        &mut self,
        kind: Kind,
        details: Option<Details>,
        children: impl FnOnce(&mut Self),
    ) {
        let index = append(&mut self.origins, kind, details);
        children(self);
        self.origins.nodes[index].end = occurrence_index(self.origins.nodes.len());
    }

    fn within(&mut self, kind: Kind, children: impl FnOnce(&mut Self)) {
        self.scope(kind, None, children);
    }

    fn declaration(&mut self, item: &Item) {
        match item {
            Item::Impl(_) => self.within(Kind::Impl, |this| visit::walk_item(this, item)),
            Item::Trait(_) => self.within(Kind::Trait, |this| visit::walk_item(this, item)),
            Item::Macro(mac) => {
                self.macro_attributes = !mac.attrs.is_empty();
                self.visit_macro_invocation(&mac.invocation);
            }
            Item::Retained(item) => {
                if let Some(mac) = item.macro_syntax() {
                    self.macro_attributes = !item.attrs().is_empty();
                    self.visit_macro_invocation(mac.invocation());
                }
                if let Some(body) = item.modeled_body_item().or_else(|| item.conditional_item()) {
                    self.declaration(body);
                } else if let Some(crate::RetainedDeclaration::Function {
                    signature: Some(function), ..
                }) = item.declaration() {
                    self.within(Kind::Signature, |this| this.signature(function));
                }
            }
            _ => visit::walk_item(self, item),
        }
    }

    fn parameters<'model>(&mut self, parameters: impl IntoIterator<Item = &'model Param>) {
        for parameter in parameters {
            self.within(Kind::Parameter, |this| {
                if let Some(pattern) = &parameter.pattern {
                    this.visit_pattern(pattern);
                }
                this.visit_type(&parameter.ty);
            });
        }
    }

    fn signature(&mut self, function: &FunctionDef) {
        self.visit_generics(&function.generics);
        self.parameters(&function.params);
        self.visit_type(&function.return_type);
    }

    fn closure(&mut self, params: &[ClosureParam], output: Option<&Type>, body: &Expr) {
        for param in params {
            self.within(Kind::ClosureParameter, |this| {
                this.visit_pattern(&param.pattern);
                if let Some(ty) = &param.ty {
                    this.visit_type(ty);
                }
            });
        }
        if let Some(ty) = output {
            self.visit_type(ty);
        }
        self.visit_expr(body);
    }
}

impl<'model> Visit<'model> for IndexModel {
    fn visit_item(&mut self, item: &'model Item) {
        self.within(Kind::Item, |this| this.declaration(item));
    }

    fn visit_function(&mut self, function: &'model FunctionDef) {
        self.within(Kind::Function, |this| {
            this.signature(function);
            this.visit_block(&function.body);
        });
    }

    fn visit_function_declaration(&mut self, declaration: &'model crate::FunctionDeclaration) {
        self.visit_generics(&declaration.generics);
        self.parameters(declaration.params.iter().map(|param| &param.param));
        if let Some(pattern) = declaration.variadic.as_ref().and_then(|var| var.pattern.as_ref()) {
            self.visit_pattern(pattern);
        }
        self.visit_type(&declaration.return_type);
    }

    fn visit_block(&mut self, block: &'model Block) {
        self.within(Kind::Block, |this| visit::walk_block(this, block));
    }

    fn visit_statement(&mut self, statement: &'model Statement) {
        self.within(Kind::Statement, |this| match statement {
            Statement::Let(_) => this.within(Kind::Binding, |this| {
                visit::walk_statement(this, statement);
            }),
            Statement::Macro(mac) => {
                this.macro_attributes = !mac.attrs.is_empty();
                this.visit_macro_invocation(&mac.invocation);
            }
            _ => visit::walk_statement(this, statement),
        });
    }

    fn visit_expr(&mut self, expression: &'model Expr) {
        self.within(Kind::Expression, |this| match expression {
            Expr::Closure(params, body, _) => this.closure(params, None, body),
            Expr::TypedClosure(params, ty, body, _) => this.closure(params, Some(ty), body),
            Expr::AsyncClosure { params, return_type, body, .. } => {
                this.closure(params, return_type.as_ref(), body);
            }
            Expr::Macro(mac) => {
                this.macro_attributes = !mac.attrs.is_empty();
                this.visit_macro_invocation(&mac.invocation);
            }
            _ => visit::walk_expr(this, expression),
        });
    }

    fn visit_pattern(&mut self, pattern: &'model Pattern) {
        self.within(Kind::Pattern, |this| visit::walk_pattern(this, pattern));
    }

    fn visit_type(&mut self, ty: &'model Type) {
        self.within(Kind::Type, |this| visit::walk_type(this, ty));
    }

    fn visit_macro_invocation(&mut self, invocation: &'model MacroInvocation) {
        let has_attributes = std::mem::take(&mut self.macro_attributes);
        self.scope(Kind::Macro, Some(Details::Macro { has_attributes }), |this| {
            walk_macro_operands(this, invocation);
        });
    }
}
