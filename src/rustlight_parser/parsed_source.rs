//! An immutable source revision and its parser-owned provenance.

use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

use crate::{Attribute, Item, RetainedItem, RustModule};

use super::{SourceParseError, model_origins::SourceOrigins};

/// Identity of one parsed physical source, independent of its allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct SourceSnapshotId(u64);

impl SourceSnapshotId {
    fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// A validated physical source file. Queries borrow the same parse and don't
/// need to reinterpret printed Rust to obtain coordinates or declarations.
pub struct ParsedSourceFile {
    identity: SourceSnapshotId,
    source: Arc<str>,
    analysis_model: Arc<RustModule>,
    origins: SourceOrigins,
    file_attributes: Vec<Attribute>,
    shebang: Option<String>,
    ascii_identifiers: bool,
}

impl ParsedSourceFile {
    pub fn parse(source: &str) -> Result<Self, SourceParseError> {
        let file = syn::parse_file(source)?;
        let file_attributes = super::convert_attributes(&file.attrs);
        let shebang = file.shebang.clone();
        let ascii_identifiers = source.is_ascii() || ascii_identifiers(&file);
        let (analysis_model, origins) = super::model_origins::capture(|| {
            super::convert_file(&file, String::new())
        });
        let analysis_model = analysis_model?;
        Ok(Self {
            identity: SourceSnapshotId::fresh(),
            source: Arc::from(source),
            analysis_model: Arc::new(analysis_model),
            origins,
            file_attributes,
            shebang,
            ascii_identifiers,
        })
    }

    /// Index an edited model directly, keeping the input metadata.
    pub fn with_model(&self, analysis_model: Arc<RustModule>) -> Self {
        let origins = super::model_origins::index_model(&analysis_model);
        Self {
            identity: SourceSnapshotId::fresh(),
            source: Arc::clone(&self.source),
            analysis_model,
            origins,
            file_attributes: self.file_attributes.clone(),
            shebang: self.shebang.clone(),
            ascii_identifiers: self.ascii_identifiers,
        }
    }

    /// Visit occurrences in the immutable model produced by this parse.
    pub fn visit_original_origins<'model>(
        &'model self,
        visitor: &mut impl super::model_origins::ModelOriginVisitor<'model>,
    ) {
        super::model_origins::visit_file(&self.origins, self.identity, &self.analysis_model, visitor);
    }

    pub fn identity(&self) -> SourceSnapshotId {
        self.identity
    }

    pub fn file_has_conditional_attributes(&self) -> bool {
        self.file_attributes.iter().any(conditional_attribute)
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// The original declarations and bodies, including syntax that is retained
    /// verbatim when writing this file.
    pub fn analysis_model(&self) -> &RustModule {
        &self.analysis_model
    }

    pub fn shared_model(&self) -> Arc<RustModule> {
        Arc::clone(&self.analysis_model)
    }

    pub fn clone_with_origins<'model>(
        &self,
        storage: &'model mut Option<RustModule>,
        visitor: &mut impl super::model_origins::ModelOriginVisitor<'model>,
    ) {
        super::model_origins::clone_file(
            &self.origins, self.identity, &self.analysis_model, storage, visitor,
        );
    }

    pub fn file_attributes(&self) -> &[Attribute] {
        &self.file_attributes
    }

    pub fn shebang(&self) -> Option<&str> {
        self.shebang.as_deref()
    }

    /// Whether every identifier in the parsed Rust grammar is ASCII.
    /// Callers that also constrain macro token trees must check those separately.
    pub fn has_ascii_identifiers(&self) -> bool {
        self.ascii_identifiers
    }

    pub fn has_conditional_attributes(&self) -> bool {
        self.file_has_conditional_attributes()
            || super::source_file::model_has_conditional_attributes(self.analysis_model())
    }

    pub fn has_module_path_attribute(&self) -> bool {
        super::source_file::model_has_module_path_attribute(self.analysis_model())
    }

    pub fn declaration_view(&self, name: String) -> RustModule {
        super::declaration_view::model_declaration_view(self.analysis_model(), name)
    }

    /// Keep the declaration available to analyses when an item is read-only.
    pub fn retain_declaration(&self, item: &Item) -> Option<RetainedItem> {
        if matches!(item, Item::Raw(_) | Item::Retained(_) | Item::Macro(_)) {
            return None;
        }
        let mut retained = super::source_file::retained_model_item(item);
        retained.set_readonly_model(item.clone());
        Some(retained)
    }


}

fn conditional_attribute(attribute: &Attribute) -> bool {
    matches!(attribute.unqualified_name(), Some("cfg" | "cfg_attr"))
}


fn ascii_identifiers(file: &syn::File) -> bool {
    use syn::visit::Visit;

    struct Identifiers(bool);
    impl<'ast> Visit<'ast> for Identifiers {
        fn visit_ident(&mut self, identifier: &'ast syn::Ident) {
            self.0 &= identifier.to_string().is_ascii();
        }
    }
    let mut identifiers = Identifiers(true);
    identifiers.visit_file(file);
    identifiers.0
}
