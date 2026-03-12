use {
	crate::{Highlighter, Language, LanguageLoader, SealedSource, TreeCursor, tree_sitter::InputEdit},
	liney_tree_house::{self as tree_house, tree_sitter::Node},
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
	inner: tree_house::Syntax,
	opts: SyntaxOptions,
	viewport: Option<ViewportMetadata>,
}

impl Syntax {
	pub fn new(
		source: RopeSlice<'_>, language: Language, loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<Self, tree_house::Error> {
		let inner = tree_house::Syntax::new(source, language, opts.parse_timeout, loader)?;
		Ok(Self {
			inner,
			opts,
			viewport: None,
		})
	}

	pub fn new_viewport(
		sealed: Arc<SealedSource>, language: Language, loader: &impl LanguageLoader, opts: SyntaxOptions,
		base_offset: u32,
	) -> Result<Self, tree_house::Error> {
		let inner = tree_house::Syntax::new(sealed.slice(), language, opts.parse_timeout, loader)?;
		Ok(Self {
			inner,
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
		self.viewport = None;
		self.inner.update(source, opts.parse_timeout, edits, loader)
	}

	pub fn opts(&self) -> SyntaxOptions {
		self.opts
	}

	pub fn is_partial(&self) -> bool {
		self.viewport.is_some()
	}

	pub fn tree(&self) -> &tree_house::tree_sitter::Tree {
		self.inner.tree()
	}

	pub fn tree_for_byte_range(&self, start: u32, end: u32) -> &tree_house::tree_sitter::Tree {
		self.inner.tree_for_byte_range(start, end)
	}

	pub fn root_layer(&self) -> tree_house::Layer {
		self.inner.root()
	}

	pub fn root_language(&self) -> Language {
		self.layer(self.root_layer()).language
	}

	pub fn layer(&self, layer: tree_house::Layer) -> &tree_house::LayerData {
		self.inner.layer(layer)
	}

	pub fn layer_for_byte_range(&self, start: u32, end: u32) -> tree_house::Layer {
		self.inner.layer_for_byte_range(start, end)
	}

	pub fn layers_for_byte_range(&self, start: u32, end: u32) -> impl Iterator<Item = tree_house::Layer> + '_ {
		self.inner.layers_for_byte_range(start, end)
	}

	pub fn named_descendant_for_byte_range(&self, start: u32, end: u32) -> Option<Node<'_>> {
		self.inner.named_descendant_for_byte_range(start, end)
	}

	pub fn descendant_for_byte_range(&self, start: u32, end: u32) -> Option<Node<'_>> {
		self.inner.descendant_for_byte_range(start, end)
	}

	pub fn walk(&self) -> TreeCursor<'_> {
		self.inner.walk()
	}

	pub fn highlighter<'a, Loader>(
		&'a self, source: RopeSlice<'a>, loader: &'a Loader, range: impl RangeBounds<u32>,
	) -> Highlighter<'a, Loader>
	where
		Loader: LanguageLoader,
	{
		if let Some(meta) = &self.viewport {
			Highlighter::new_mapped(
				&self.inner,
				meta.sealed_source.slice(),
				loader,
				range,
				meta.base_offset,
				meta.base_offset + meta.real_len,
			)
		} else {
			Highlighter::new(&self.inner, source, loader, range)
		}
	}
}
