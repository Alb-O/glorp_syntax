use {
	crate::{HighlightSpans, Language, LanguageLoader, SealedSource, TreeCursor, tree_sitter::InputEdit},
	glorp_syntax_tree::{
		self as tree_house, ChangeSet, DocumentSession, EngineConfig, RopeText, TextEdit, tree_sitter::Node,
	},
	ropey::RopeSlice,
	std::{
		ops::{Range, RangeBounds},
		sync::Arc,
		time::Duration,
	},
};

/// Default parse timeout for syntax tree construction and updates.
const DEFAULT_PARSE_TIMEOUT: Duration = Duration::from_millis(500);

/// Parse options used when building or updating a syntax tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxOptions {
	/// Maximum time a parse/update may spend inside tree-sitter before timing out.
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
	/// Document byte offset at which the sealed window begins.
	pub base_offset: u32,
	/// Number of real document bytes covered by the sealed window before padding.
	pub real_len: u32,
	/// Current sealed source window, including any parser padding added during sealing.
	pub sealed_source: Arc<SealedSource>,
}

/// Full-document syntax tree wrapper.
#[derive(Debug, Clone)]
pub struct Syntax {
	session: DocumentSession,
	snapshot: tree_house::DocumentSnapshot,
	opts: SyntaxOptions,
}

impl Syntax {
	fn from_session(session: DocumentSession, opts: SyntaxOptions) -> Self {
		let snapshot = session.snapshot();
		Self {
			session,
			snapshot,
			opts,
		}
	}

	/// Parses a full-document syntax tree.
	pub fn new(
		source: RopeSlice<'_>, language: Language, loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<Self, tree_house::Error> {
		let text = RopeText::from_slice(source);
		let session = DocumentSession::new(language, &text, loader, opts.into())?;
		Ok(Self::from_session(session, opts))
	}

	/// Applies edits to a full-document syntax tree.
	///
	/// `source` must be the post-edit document text described by `edits`.
	pub fn update(
		&mut self, source: RopeSlice<'_>, edits: &[InputEdit], loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<(), tree_house::Error> {
		self.opts = opts;
		self.session.set_config(opts.into());
		if edits.is_empty() {
			return Ok(());
		}

		let change_set = ChangeSet::new(edits.iter().map(|edit| {
			let replacement = source
				.byte_slice(edit.start_byte as usize..edit.new_end_byte as usize)
				.to_string();
			TextEdit::new(edit.start_byte..edit.old_end_byte, replacement)
		}));
		self.session.apply_edits(&change_set, loader)?;
		self.snapshot = self.session.snapshot();
		Ok(())
	}

	/// Returns the current parse options.
	pub fn opts(&self) -> SyntaxOptions {
		self.opts
	}

	/// Returns the root tree for the document.
	pub fn tree(&self) -> &tree_house::tree_sitter::Tree {
		self.snapshot.tree()
	}

	/// Returns the smallest parsed tree covering `start..end`.
	pub fn tree_for_byte_range(&self, start: u32, end: u32) -> &tree_house::tree_sitter::Tree {
		self.snapshot.tree_for_byte_range(start, end)
	}

	/// Returns the root syntax layer.
	pub fn root_layer(&self) -> tree_house::Layer {
		self.snapshot.root_layer()
	}

	/// Returns the root language token for this syntax tree.
	pub fn root_language(&self) -> Language {
		self.layer(self.root_layer()).language
	}

	/// Returns metadata for one syntax layer.
	pub fn layer(&self, layer: tree_house::Layer) -> &tree_house::LayerData {
		self.snapshot.layer(layer)
	}

	/// Returns the smallest layer covering `start..end`.
	pub fn layer_for_byte_range(&self, start: u32, end: u32) -> tree_house::Layer {
		self.snapshot.layer_for_byte_range(start, end)
	}

	/// Iterates layers that fully include `start..end`, from outermost to innermost.
	pub fn layers_for_byte_range(&self, start: u32, end: u32) -> impl Iterator<Item = tree_house::Layer> + '_ {
		self.snapshot.layers_for_byte_range(start, end)
	}

	/// Returns the smallest named node covering `start..end`.
	pub fn named_descendant_for_byte_range(&self, start: u32, end: u32) -> Option<Node<'_>> {
		self.snapshot.named_node_at(start, end)
	}

	/// Returns the smallest node covering `start..end`.
	pub fn descendant_for_byte_range(&self, start: u32, end: u32) -> Option<Node<'_>> {
		self.snapshot.node_at(start, end)
	}

	/// Walks the layered syntax tree.
	pub fn walk(&self) -> TreeCursor<'_> {
		self.snapshot.walk()
	}

	/// Returns the current immutable snapshot.
	pub fn snapshot(&self) -> &tree_house::DocumentSnapshot {
		&self.snapshot
	}

	/// Streams highlight spans in document byte coordinates.
	pub fn highlight_spans<'a, Loader>(
		&'a self, loader: &'a Loader, range: impl RangeBounds<u32>,
	) -> HighlightSpans<'a, Loader>
	where
		Loader: LanguageLoader, {
		self.snapshot.highlight_spans(loader, range)
	}

	pub(crate) fn root_end_byte(&self) -> u32 {
		self.snapshot.tree().root_node().end_byte()
	}
}

/// Viewport-local syntax tree wrapper with document-mapped highlighting support.
#[derive(Debug, Clone)]
pub struct ViewportSyntax {
	session: DocumentSession,
	snapshot: tree_house::DocumentSnapshot,
	opts: SyntaxOptions,
	viewport: ViewportMetadata,
}

impl ViewportSyntax {
	fn from_session(session: DocumentSession, opts: SyntaxOptions, viewport: ViewportMetadata) -> Self {
		let snapshot = session.snapshot();
		Self {
			session,
			snapshot,
			opts,
			viewport,
		}
	}

	/// Parses a viewport-local syntax tree from a sealed source window.
	///
	/// This type is render-oriented. Use [`Syntax`] for document-wide semantic queries.
	pub fn new(
		sealed: Arc<SealedSource>, language: Language, loader: &impl LanguageLoader, opts: SyntaxOptions,
		base_offset: u32,
	) -> Result<Self, tree_house::Error> {
		let text = RopeText::from_slice(sealed.slice());
		let session = DocumentSession::new(language, &text, loader, opts.into())?;
		Ok(Self::from_session(
			session,
			opts,
			ViewportMetadata {
				base_offset,
				real_len: sealed.real_len_bytes,
				sealed_source: sealed,
			},
		))
	}

	/// Rebuilds the viewport tree after edits against the full document source.
	///
	/// The resulting metadata may shift if edits move or resize the covered window.
	pub fn update(
		&mut self, source: RopeSlice<'_>, edits: &[InputEdit], loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<(), tree_house::Error> {
		self.opts = opts;
		if edits.is_empty() {
			self.session.set_config(opts.into());
			return Ok(());
		}

		let coverage = remap_viewport_range(
			self.viewport.base_offset..self.viewport.base_offset + self.viewport.real_len,
			edits,
		);
		let base_offset = coverage.start;
		// Re-seal against the remapped document coverage so future highlight output stays in
		// full-document coordinates even after leading/trailing edits shift the window.
		let sealed = Arc::new(SealedSource::from_byte_range_with_newline_padding(source, coverage));
		self.session = DocumentSession::new(
			self.session.language(),
			&RopeText::from_slice(sealed.slice()),
			loader,
			opts.into(),
		)?;
		self.viewport = ViewportMetadata {
			base_offset,
			real_len: sealed.real_len_bytes,
			sealed_source: sealed,
		};
		self.snapshot = self.session.snapshot();
		Ok(())
	}

	/// Returns the current parse options.
	pub fn opts(&self) -> SyntaxOptions {
		self.opts
	}

	/// Returns the current document-mapping metadata for this viewport tree.
	pub fn metadata(&self) -> &ViewportMetadata {
		&self.viewport
	}

	/// Streams highlight spans remapped into full-document byte coordinates.
	pub fn highlight_spans<'a, Loader>(
		&'a self, loader: &'a Loader, range: impl RangeBounds<u32>,
	) -> HighlightSpans<'a, Loader>
	where
		Loader: LanguageLoader, {
		HighlightSpans::new_mapped(
			self.snapshot.syntax(),
			self.viewport.sealed_source.slice(),
			loader,
			range,
			self.viewport.base_offset,
			self.viewport.base_offset + self.viewport.real_len,
		)
	}

	pub(crate) fn root_end_byte(&self) -> u32 {
		self.snapshot.tree().root_node().end_byte()
	}
}

fn remap_viewport_range(mut range: Range<u32>, edits: &[InputEdit]) -> Range<u32> {
	for edit in edits {
		range.start = remap_offset(range.start, edit, false);
		range.end = remap_offset(range.end, edit, true).max(range.start);
	}
	range
}

fn remap_offset(offset: u32, edit: &InputEdit, map_to_new_end: bool) -> u32 {
	if offset < edit.start_byte {
		return offset;
	}
	if offset > edit.old_end_byte {
		return offset.saturating_add_signed(edit.new_end_byte as i32 - edit.old_end_byte as i32);
	}
	if map_to_new_end {
		edit.new_end_byte
	} else {
		edit.start_byte
	}
}

impl From<SyntaxOptions> for EngineConfig {
	fn from(value: SyntaxOptions) -> Self {
		Self {
			parse_timeout: value.parse_timeout,
		}
	}
}

#[cfg(test)]
mod tests {
	use {
		super::*,
		crate::{Highlight, SingleLanguageLoader, tree_sitter::Grammar},
		ropey::Rope,
	};

	#[test]
	fn full_update_refreshes_parse_timeout_without_edits() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
		let rope = Rope::from_str("fn alpha() {}\n");
		let mut syntax = Syntax::new(rope.slice(..), loader.language(), &loader, SyntaxOptions::default())
			.expect("syntax should parse");
		let timeout = Duration::from_millis(5);

		syntax
			.update(rope.slice(..), &[], &loader, SyntaxOptions { parse_timeout: timeout })
			.expect("no-op update should succeed");

		assert_eq!(syntax.opts().parse_timeout, timeout);
		assert_eq!(syntax.session.config().parse_timeout, timeout);
	}

	#[test]
	fn full_update_refreshes_parse_timeout_before_editing() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
		let mut rope = Rope::from_str("fn alpha() {}\n");
		let mut syntax = Syntax::new(rope.slice(..), loader.language(), &loader, SyntaxOptions::default())
			.expect("syntax should parse");
		let replacement = "beta";
		let timeout = Duration::from_millis(7);
		rope.remove(3..8);
		rope.insert(3, replacement);
		let edit = InputEdit {
			start_byte: 3,
			old_end_byte: 8,
			new_end_byte: 3 + replacement.len() as u32,
			start_point: tree_house::tree_sitter::Point { row: 0, col: 3 },
			old_end_point: tree_house::tree_sitter::Point { row: 0, col: 8 },
			new_end_point: tree_house::tree_sitter::Point { row: 0, col: 7 },
		};

		syntax
			.update(
				rope.slice(..),
				&[edit],
				&loader,
				SyntaxOptions { parse_timeout: timeout },
			)
			.expect("update should succeed");

		assert_eq!(syntax.opts().parse_timeout, timeout);
		assert_eq!(syntax.session.config().parse_timeout, timeout);
		assert_eq!(syntax.snapshot().byte_text(0..14), "fn beta() {}\n");
	}

	#[test]
	fn viewport_update_tracks_shifted_base_offset() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
		let mut rope = Rope::from_str("const before = 1;\nfn alpha() {}\n");
		let viewport_start = rope.to_string().find("fn alpha").expect("viewport start should exist") as u32;
		let viewport_end = rope.len_bytes() as u32;
		let sealed = Arc::new(SealedSource::from_byte_range_with_newline_padding(
			rope.slice(..),
			viewport_start..viewport_end,
		));
		let mut syntax = ViewportSyntax::new(
			sealed,
			loader.language(),
			&loader,
			SyntaxOptions::default(),
			viewport_start,
		)
		.expect("viewport syntax should parse");

		rope.insert(0, "//\n");
		let edit = InputEdit {
			start_byte: 0,
			old_end_byte: 0,
			new_end_byte: 3,
			start_point: tree_house::tree_sitter::Point { row: 0, col: 0 },
			old_end_point: tree_house::tree_sitter::Point { row: 0, col: 0 },
			new_end_point: tree_house::tree_sitter::Point { row: 1, col: 0 },
		};

		syntax
			.update(rope.slice(..), &[edit], &loader, SyntaxOptions::default())
			.expect("viewport update should succeed");

		assert_eq!(syntax.metadata().base_offset, viewport_start + 3);
		assert!(syntax.metadata().real_len >= viewport_end - viewport_start);
	}

	#[test]
	fn viewport_highlight_spans_match_full_document_offsets() {
		const SOURCE: &str = r#"const BEFORE: u32 = 1;

fn middle(value: i32) -> i32 {
    let label = "mid";
    value + 1
}

const AFTER: u32 = 2;
"#;
		const HIGHLIGHT_QUERY: &str = r#"
(identifier) @identifier
(primitive_type) @type.builtin
(string_literal) @string
(integer_literal) @number
"#;

		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::with_highlights(grammar, HIGHLIGHT_QUERY, "", "", |name| {
			Some(match name {
				"identifier" => Highlight::new(1),
				"type.builtin" => Highlight::new(2),
				"string" => Highlight::new(3),
				"number" => Highlight::new(4),
				_ => return None,
			})
		})
		.expect("loader should build");
		let rope = Rope::from_str(SOURCE);
		let full = Syntax::new(rope.slice(..), loader.language(), &loader, SyntaxOptions::default())
			.expect("full syntax should parse");
		let viewport_start = SOURCE.find("fn middle").expect("viewport start should exist") as u32;
		let viewport_end = SOURCE.find("\n\nconst AFTER").expect("viewport end should exist") as u32;
		let sealed = Arc::new(SealedSource::from_byte_range_with_newline_padding(
			rope.slice(..),
			viewport_start..viewport_end,
		));
		let viewport = ViewportSyntax::new(
			sealed,
			loader.language(),
			&loader,
			SyntaxOptions::default(),
			viewport_start,
		)
		.expect("viewport syntax should parse");

		let full_spans: Vec<_> = full.highlight_spans(&loader, viewport_start..viewport_end).collect();
		let viewport_spans: Vec<_> = viewport
			.highlight_spans(&loader, viewport_start..viewport_end)
			.collect();

		assert_eq!(viewport_spans, full_spans);
		assert!(
			viewport_spans
				.iter()
				.all(|span| span.start >= viewport_start && span.end <= viewport_end)
		);
	}
}
