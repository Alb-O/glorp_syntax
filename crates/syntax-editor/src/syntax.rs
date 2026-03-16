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

impl ViewportMetadata {
	fn new(base_offset: u32, sealed_source: Arc<SealedSource>) -> Self {
		Self {
			base_offset,
			real_len: sealed_source.real_len_bytes,
			sealed_source,
		}
	}

	fn coverage(&self) -> Range<u32> {
		self.base_offset..self.base_offset + self.real_len
	}
}

#[derive(Debug, Clone)]
struct SyntaxCore {
	session: DocumentSession,
	snapshot: tree_house::DocumentSnapshot,
	opts: SyntaxOptions,
}

impl SyntaxCore {
	fn new(
		source: RopeSlice<'_>, language: Language, loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<Self, tree_house::Error> {
		let text = RopeText::from_slice(source);
		let session = DocumentSession::new(language, &text, loader, opts.into())?;
		Ok(Self::from_session(session, opts))
	}

	fn from_session(session: DocumentSession, opts: SyntaxOptions) -> Self {
		let snapshot = session.snapshot();
		Self {
			session,
			snapshot,
			opts,
		}
	}

	fn language(&self) -> Language {
		self.session.language()
	}

	fn set_opts(&mut self, opts: SyntaxOptions) {
		self.opts = opts;
		self.session.set_config(opts.into());
	}

	fn refresh_snapshot(&mut self) {
		self.snapshot = self.session.snapshot();
	}

	fn rebuild(
		&mut self, source: RopeSlice<'_>, loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<(), tree_house::Error> {
		self.session = DocumentSession::new(self.language(), &RopeText::from_slice(source), loader, opts.into())?;
		self.opts = opts;
		self.refresh_snapshot();
		Ok(())
	}

	fn opts(&self) -> SyntaxOptions {
		self.opts
	}

	fn snapshot(&self) -> &tree_house::DocumentSnapshot {
		&self.snapshot
	}

	fn root_end_byte(&self) -> u32 {
		self.snapshot.tree().root_node().end_byte()
	}

	fn highlight_spans<'a, Loader>(
		&'a self, loader: &'a Loader, range: impl RangeBounds<u32>,
	) -> HighlightSpans<'a, Loader>
	where
		Loader: LanguageLoader, {
		self.snapshot.highlight_spans(loader, range)
	}

	fn highlight_spans_mapped<'a, Loader>(
		&'a self, viewport: &'a ViewportMetadata, loader: &'a Loader, range: impl RangeBounds<u32>,
	) -> HighlightSpans<'a, Loader>
	where
		Loader: LanguageLoader, {
		// Viewport trees parse a sealed local slice, but callers still speak in document bytes.
		// The mapped iterator bridges that gap without exposing sealed offsets at the API boundary.
		HighlightSpans::new_mapped(
			self.snapshot.syntax(),
			viewport.sealed_source.slice(),
			loader,
			range,
			viewport.base_offset,
			viewport.base_offset + viewport.real_len,
		)
	}
}

/// Full-document syntax tree wrapper for semantic queries and updates.
#[derive(Debug, Clone)]
pub struct DocumentSyntax {
	core: SyntaxCore,
}

impl DocumentSyntax {
	/// Parses a full-document syntax tree.
	pub fn new(
		source: RopeSlice<'_>, language: Language, loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<Self, tree_house::Error> {
		Ok(Self {
			core: SyntaxCore::new(source, language, loader, opts)?,
		})
	}

	/// Converts this semantic tree into a render tree backed by the same parse state.
	pub fn into_render(self) -> RenderSyntax {
		RenderSyntax {
			core: self.core,
			viewport: None,
		}
	}

	/// Applies edits to a full-document syntax tree.
	///
	/// `source` must be the post-edit document text described by `edits`.
	pub fn update(
		&mut self, source: RopeSlice<'_>, edits: &[InputEdit], loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<(), tree_house::Error> {
		apply_document_edits(&mut self.core, source, edits, loader, opts)
	}

	/// Returns the current parse options.
	pub fn opts(&self) -> SyntaxOptions {
		self.core.opts()
	}

	/// Returns the root tree for the document.
	pub fn tree(&self) -> &tree_house::tree_sitter::Tree {
		self.core.snapshot().tree()
	}

	/// Returns the smallest parsed tree covering `start..end`.
	pub fn tree_for_byte_range(&self, start: u32, end: u32) -> &tree_house::tree_sitter::Tree {
		self.core.snapshot().tree_for_byte_range(start, end)
	}

	/// Returns the root syntax layer.
	pub fn root_layer(&self) -> tree_house::Layer {
		self.core.snapshot().root_layer()
	}

	/// Returns the root language token for this syntax tree.
	pub fn root_language(&self) -> Language {
		self.layer(self.root_layer()).language
	}

	/// Returns metadata for one syntax layer.
	pub fn layer(&self, layer: tree_house::Layer) -> &tree_house::LayerData {
		self.core.snapshot().layer(layer)
	}

	/// Returns the smallest layer covering `start..end`.
	pub fn layer_for_byte_range(&self, start: u32, end: u32) -> tree_house::Layer {
		self.core.snapshot().layer_for_byte_range(start, end)
	}

	/// Iterates layers that fully include `start..end`, from outermost to innermost.
	pub fn layers_for_byte_range(&self, start: u32, end: u32) -> impl Iterator<Item = tree_house::Layer> + '_ {
		self.core.snapshot().layers_for_byte_range(start, end)
	}

	/// Returns the smallest named node covering `start..end`.
	pub fn named_descendant_for_byte_range(&self, start: u32, end: u32) -> Option<Node<'_>> {
		self.core.snapshot().named_node_at(start, end)
	}

	/// Returns the smallest node covering `start..end`.
	pub fn descendant_for_byte_range(&self, start: u32, end: u32) -> Option<Node<'_>> {
		self.core.snapshot().node_at(start, end)
	}

	/// Walks the layered syntax tree.
	pub fn walk(&self) -> TreeCursor<'_> {
		self.core.snapshot().walk()
	}

	/// Returns the current immutable snapshot.
	pub fn snapshot(&self) -> &tree_house::DocumentSnapshot {
		self.core.snapshot()
	}
}

/// Render-oriented syntax tree used by viewport selection and highlight streaming.
#[derive(Debug, Clone)]
pub struct RenderSyntax {
	core: SyntaxCore,
	viewport: Option<ViewportMetadata>,
}

impl RenderSyntax {
	/// Parses a full-document render tree.
	pub fn new_full(
		source: RopeSlice<'_>, language: Language, loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<Self, tree_house::Error> {
		Ok(Self {
			core: SyntaxCore::new(source, language, loader, opts)?,
			viewport: None,
		})
	}

	/// Parses a viewport-local render tree from a sealed source window.
	pub fn new_viewport(
		sealed: Arc<SealedSource>, language: Language, loader: &impl LanguageLoader, opts: SyntaxOptions,
		base_offset: u32,
	) -> Result<Self, tree_house::Error> {
		Ok(Self {
			core: SyntaxCore::new(sealed.slice(), language, loader, opts)?,
			viewport: Some(ViewportMetadata::new(base_offset, sealed)),
		})
	}

	/// Applies edits to this render tree.
	///
	/// Full trees are updated incrementally. Viewport trees are re-sealed and rebuilt against
	/// the post-edit full document text.
	pub fn update(
		&mut self, source: RopeSlice<'_>, edits: &[InputEdit], loader: &impl LanguageLoader, opts: SyntaxOptions,
	) -> Result<(), tree_house::Error> {
		match &mut self.viewport {
			None => apply_document_edits(&mut self.core, source, edits, loader, opts),
			Some(viewport) => {
				self.core.set_opts(opts);
				if edits.is_empty() {
					return Ok(());
				}

				// Viewport trees are deliberately render-only: after edits we remap the old document
				// coverage, reseal that window against the new source, and rebuild from scratch.
				let coverage = remap_viewport_range(viewport.coverage(), edits);
				let base_offset = coverage.start;
				let sealed = Arc::new(SealedSource::from_byte_range_with_newline_padding(source, coverage));
				self.core.rebuild(sealed.slice(), loader, opts)?;
				*viewport = ViewportMetadata::new(base_offset, sealed);
				Ok(())
			}
		}
	}

	/// Returns the current parse options.
	pub fn opts(&self) -> SyntaxOptions {
		self.core.opts()
	}

	/// Returns whether this render tree covers the full document.
	pub fn is_full(&self) -> bool {
		self.viewport.is_none()
	}

	/// Returns whether this render tree is backed by a viewport window.
	pub fn is_viewport(&self) -> bool {
		self.viewport.is_some()
	}

	/// Returns the viewport coverage in document byte coordinates when this is a viewport tree.
	pub fn coverage(&self) -> Option<Range<u32>> {
		self.viewport_metadata().map(ViewportMetadata::coverage)
	}

	/// Returns document-mapping metadata when this render tree is viewport-backed.
	pub fn viewport_metadata(&self) -> Option<&ViewportMetadata> {
		self.viewport.as_ref()
	}

	/// Returns the parsed root tree end byte for the underlying render tree.
	pub fn root_end_byte(&self) -> u32 {
		self.core.root_end_byte()
	}

	/// Streams highlight spans in document byte coordinates.
	pub fn highlight_spans<'a, Loader>(
		&'a self, loader: &'a Loader, range: impl RangeBounds<u32>,
	) -> HighlightSpans<'a, Loader>
	where
		Loader: LanguageLoader, {
		match &self.viewport {
			None => self.core.highlight_spans(loader, range),
			Some(viewport) => self.core.highlight_spans_mapped(viewport, loader, range),
		}
	}
}

fn apply_document_edits(
	core: &mut SyntaxCore, source: RopeSlice<'_>, edits: &[InputEdit], loader: &impl LanguageLoader,
	opts: SyntaxOptions,
) -> Result<(), tree_house::Error> {
	core.set_opts(opts);
	if edits.is_empty() {
		return Ok(());
	}

	let change_set = ChangeSet::new(edits.iter().map(|edit| {
		let replacement = source
			.byte_slice(edit.start_byte as usize..edit.new_end_byte as usize)
			.to_string();
		TextEdit::new(edit.start_byte..edit.old_end_byte, replacement)
	}));
	core.session.apply_edits(&change_set, loader)?;
	core.refresh_snapshot();
	Ok(())
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
	fn document_update_refreshes_parse_timeout_without_edits() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
		let rope = Rope::from_str("fn alpha() {}\n");
		let mut syntax = DocumentSyntax::new(rope.slice(..), loader.language(), &loader, SyntaxOptions::default())
			.expect("syntax should parse");
		let timeout = Duration::from_millis(5);

		syntax
			.update(rope.slice(..), &[], &loader, SyntaxOptions { parse_timeout: timeout })
			.expect("no-op update should succeed");

		assert_eq!(syntax.opts().parse_timeout, timeout);
		assert_eq!(syntax.core.session.config().parse_timeout, timeout);
	}

	#[test]
	fn document_update_refreshes_parse_timeout_before_editing() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
		let mut rope = Rope::from_str("fn alpha() {}\n");
		let mut syntax = DocumentSyntax::new(rope.slice(..), loader.language(), &loader, SyntaxOptions::default())
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
		assert_eq!(syntax.core.session.config().parse_timeout, timeout);
		assert_eq!(syntax.snapshot().byte_text(0..14), "fn beta() {}\n");
	}

	#[test]
	fn full_render_highlight_spans_match_document_render_spans() {
		const SOURCE: &str = r#"fn middle(value: i32) -> i32 {
    let label = "mid";
    value + 1
}
"#;
		const HIGHLIGHT_QUERY: &str = r"
(identifier) @identifier
(primitive_type) @type.builtin
(string_literal) @string
(integer_literal) @number
";

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
		let document = DocumentSyntax::new(rope.slice(..), loader.language(), &loader, SyntaxOptions::default())
			.expect("document syntax should parse");
		let full = RenderSyntax::new_full(rope.slice(..), loader.language(), &loader, SyntaxOptions::default())
			.expect("full render syntax should parse");

		let full_spans: Vec<_> = full.highlight_spans(&loader, ..).collect();
		let document_render_spans: Vec<_> = document.into_render().highlight_spans(&loader, ..).collect();

		assert_eq!(full_spans, document_render_spans);
	}

	#[test]
	fn viewport_update_tracks_shifted_base_offset() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
		let source = "const before = 1;\nfn alpha() {}\n";
		let mut rope = Rope::from_str(source);
		let viewport_start = source.find("fn alpha").expect("viewport start should exist") as u32;
		let viewport_end = rope.len_bytes() as u32;
		let sealed = Arc::new(SealedSource::from_byte_range_with_newline_padding(
			rope.slice(..),
			viewport_start..viewport_end,
		));
		let mut syntax = RenderSyntax::new_viewport(
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

		let viewport = syntax
			.viewport_metadata()
			.expect("viewport render syntax should retain viewport metadata");
		assert_eq!(viewport.base_offset, viewport_start + 3);
		assert!(viewport.real_len >= viewport_end - viewport_start);
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
		const HIGHLIGHT_QUERY: &str = r"
(identifier) @identifier
(primitive_type) @type.builtin
(string_literal) @string
(integer_literal) @number
";

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
		let full = RenderSyntax::new_full(rope.slice(..), loader.language(), &loader, SyntaxOptions::default())
			.expect("full render syntax should parse");
		let viewport_start = SOURCE.find("fn middle").expect("viewport start should exist") as u32;
		let viewport_end = SOURCE.find("\n\nconst AFTER").expect("viewport end should exist") as u32;
		let sealed = Arc::new(SealedSource::from_byte_range_with_newline_padding(
			rope.slice(..),
			viewport_start..viewport_end,
		));
		let viewport = RenderSyntax::new_viewport(
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
