use {
	crate::{Highlighter, Language, LanguageLoader, SealedSource, TreeCursor, tree_sitter::InputEdit},
	liney_tree_house::{
		self as tree_house, ByteRangeText, ChangeSet, DocumentSession, EngineConfig, RopeText, TextEdit,
		tree_sitter::Node,
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

		if let Some(meta) = &self.viewport {
			let coverage = remap_viewport_range(meta.base_offset..meta.base_offset + meta.real_len, edits);
			let sealed = Arc::new(SealedSource::from_byte_range_with_newline_padding(
				source,
				coverage.clone(),
			));
			self.session = DocumentSession::new(
				self.root_language(),
				&RopeText::from_slice(sealed.slice()),
				loader,
				opts.into(),
			)?;
			self.viewport = Some(ViewportMetadata {
				base_offset: coverage.start,
				real_len: sealed.real_len_bytes,
				sealed_source: sealed,
			});
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
		crate::{SingleLanguageLoader, tree_sitter::Grammar},
		ropey::Rope,
	};

	#[test]
	fn partial_update_stays_partial() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader =
			SingleLanguageLoader::from_queries(Language::new(0), grammar, "", "", "").expect("loader should build");
		let mut rope = Rope::from_str("fn alpha() {\n    alpha();\n}\n");
		let start = 0u32;
		let end = rope.len_bytes() as u32 - 2;
		let sealed = Arc::new(SealedSource::from_byte_range_with_newline_padding(
			rope.slice(..),
			start..end,
		));
		let mut syntax = Syntax::new_viewport(sealed, loader.language(), &loader, SyntaxOptions::default(), start)
			.expect("viewport syntax should parse");

		let replacement = "beta";
		let old_end = 8u32;
		rope.remove(3..8);
		rope.insert(3, replacement);
		let edit = InputEdit {
			start_byte: 3,
			old_end_byte: old_end,
			new_end_byte: 3 + replacement.len() as u32,
			start_point: tree_house::tree_sitter::Point { row: 0, col: 3 },
			old_end_point: tree_house::tree_sitter::Point { row: 0, col: 8 },
			new_end_point: tree_house::tree_sitter::Point { row: 0, col: 7 },
		};

		syntax
			.update(rope.slice(..), &[edit], &loader, SyntaxOptions::default())
			.expect("viewport update should succeed");

		assert!(syntax.is_partial());
		assert_eq!(syntax.snapshot().byte_text(0..12), "fn beta() {\n");
	}
}
