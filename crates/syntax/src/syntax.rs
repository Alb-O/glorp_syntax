use {
	crate::{Highlighter, Language, LanguageLoader, SealedSource, TreeCursor, tree_sitter::InputEdit},
	liney_tree_house::{
		self as tree_house, ByteRangeText, ChangeSet, DocumentSession, EngineConfig, RopeText, TextEdit,
		tree_sitter::Node,
	},
	ropey::RopeSlice,
	std::{ops::RangeBounds, sync::Arc, time::Duration},
};

/// Default parse timeout for syntax tree construction and updates.
const DEFAULT_PARSE_TIMEOUT: Duration = Duration::from_millis(500);

/// Parse options used when building or updating a syntax tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxOptions {
	pub parse_timeout: Duration,
}

impl Default for SyntaxOptions {
	fn default() -> Self {
		Self {
			parse_timeout: DEFAULT_PARSE_TIMEOUT,
		}
	}
}

/// Metadata for syntax trees parsed from a viewport-local sealed window.
#[derive(Debug, Clone)]
pub struct ViewportMetadata {
	pub base_offset: u32,
	pub real_len: u32,
	pub sealed_source: Arc<SealedSource>,
}

/// Syntax tree wrapper with viewport-aware highlighting support.
#[derive(Debug, Clone)]
pub struct Syntax {
	session: DocumentSession,
	snapshot: tree_house::DocumentSnapshot,
	opts: SyntaxOptions,
	viewport: Option<ViewportMetadata>,
}

impl Syntax {
	pub fn new(
		source: RopeSlice<'_>, language: Language, loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<Self, tree_house::Error> {
		let text = RopeText::from_slice(source);
		let session = DocumentSession::new(language, &text, loader, opts.into())?;
		let snapshot = session.snapshot();
		Ok(Self {
			session,
			snapshot,
			opts,
			viewport: None,
		})
	}

	pub fn new_viewport(
		sealed: Arc<SealedSource>, language: Language, loader: &impl LanguageLoader, opts: SyntaxOptions,
		base_offset: u32,
	) -> Result<Self, tree_house::Error> {
		let text = RopeText::from_slice(sealed.slice());
		let session = DocumentSession::new(language, &text, loader, opts.into())?;
		let snapshot = session.snapshot();
		Ok(Self {
			session,
			snapshot,
			opts,
			viewport: Some(ViewportMetadata {
				base_offset,
				real_len: sealed.real_len_bytes,
				sealed_source: sealed,
			}),
		})
	}

	pub fn update(
		&mut self, source: RopeSlice<'_>, edits: &[InputEdit], loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<(), tree_house::Error> {
		if edits.is_empty() {
			return Ok(());
		}

		self.opts = opts;
		let text = RopeText::from_slice(source);

		if self.viewport.is_some() {
			self.viewport = None;
			self.session = DocumentSession::new(self.root_language(), &text, loader, opts.into())?;
		} else {
			let change_set = ChangeSet::new(edits.iter().map(|edit| {
				let replacement = text.byte_text(edit.start_byte..edit.new_end_byte);
				TextEdit::new(edit.start_byte..edit.old_end_byte, replacement)
			}));
			self.session.apply_edits(&change_set, loader)?;
		}
		self.snapshot = self.session.snapshot();
		Ok(())
	}

	pub fn opts(&self) -> SyntaxOptions {
		self.opts
	}

	pub fn is_partial(&self) -> bool {
		self.viewport.is_some()
	}

	pub fn tree(&self) -> &tree_house::tree_sitter::Tree {
		self.snapshot.tree()
	}

	pub fn tree_for_byte_range(&self, start: u32, end: u32) -> &tree_house::tree_sitter::Tree {
		self.snapshot.tree_for_byte_range(start, end)
	}

	pub fn root_layer(&self) -> tree_house::Layer {
		self.snapshot.root_layer()
	}

	pub fn root_language(&self) -> Language {
		self.layer(self.root_layer()).language
	}

	pub fn layer(&self, layer: tree_house::Layer) -> &tree_house::LayerData {
		self.snapshot.layer(layer)
	}

	pub fn layer_for_byte_range(&self, start: u32, end: u32) -> tree_house::Layer {
		self.snapshot.layer_for_byte_range(start, end)
	}

	pub fn layers_for_byte_range(&self, start: u32, end: u32) -> impl Iterator<Item = tree_house::Layer> + '_ {
		self.snapshot.layers_for_byte_range(start, end)
	}

	pub fn named_descendant_for_byte_range(&self, start: u32, end: u32) -> Option<Node<'_>> {
		self.snapshot.named_node_at(start, end)
	}

	pub fn descendant_for_byte_range(&self, start: u32, end: u32) -> Option<Node<'_>> {
		self.snapshot.node_at(start, end)
	}

	pub fn walk(&self) -> TreeCursor<'_> {
		self.snapshot.walk()
	}

	pub fn snapshot(&self) -> &tree_house::DocumentSnapshot {
		&self.snapshot
	}

	pub fn highlighter<'a, Loader>(
		&'a self, source: RopeSlice<'a>, loader: &'a Loader, range: impl RangeBounds<u32>,
	) -> Highlighter<'a, Loader>
	where
		Loader: LanguageLoader,
	{
		if let Some(meta) = &self.viewport {
			Highlighter::new_mapped(
				self.snapshot(),
				meta.sealed_source.slice(),
				loader,
				range,
				meta.base_offset,
				meta.base_offset + meta.real_len,
			)
		} else {
			Highlighter::new(self.snapshot(), source, loader, range)
		}
	}
}

impl From<SyntaxOptions> for EngineConfig {
	fn from(value: SyntaxOptions) -> Self {
		Self {
			parse_timeout: value.parse_timeout,
		}
	}
}
