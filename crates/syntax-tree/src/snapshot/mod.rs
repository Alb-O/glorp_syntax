use {
	crate::{
		Language, LanguageLoader, Layer, Syntax, TREE_SITTER_MATCH_LIMIT, TreeCursor,
		change::{Revision, SnapshotId},
		highlighter::HighlightSpans,
		locals::{Definition, Locals},
		query_iter::{QueryIter, QueryLoader},
		text::{ByteRangeText, DocumentText},
	},
	ropey::{Rope, RopeSlice},
	std::{ops::RangeBounds, sync::Arc},
	tree_sitter::{Capture, InactiveQueryCursor, Node, Query, RopeInput, Tree},
};

#[derive(Debug, Clone, Copy)]
pub struct LocalScope<'a> {
	locals: &'a Locals,
	scope: crate::locals::Scope,
}

impl<'a> LocalScope<'a> {
	pub fn lookup(&self, name: &str) -> Option<&'a Definition> {
		self.locals.lookup_reference(self.scope, name)
	}
}

#[derive(Debug, Clone)]
pub struct DocumentSnapshot {
	id: SnapshotId,
	revision: Revision,
	generation: u64,
	text: Arc<Rope>,
	syntax: Arc<Syntax>,
}

impl DocumentSnapshot {
	pub(crate) fn new(
		id: SnapshotId, revision: Revision, generation: u64, text: Arc<Rope>, syntax: Arc<Syntax>,
	) -> Self {
		Self {
			id,
			revision,
			generation,
			text,
			syntax,
		}
	}

	pub fn id(&self) -> SnapshotId {
		self.id
	}

	pub fn revision(&self) -> Revision {
		self.revision
	}

	pub fn generation(&self) -> u64 {
		self.generation
	}

	pub fn syntax(&self) -> &Syntax {
		&self.syntax
	}

	pub fn text(&self) -> DocumentText<'_> {
		DocumentText::new(self.rope_slice())
	}

	pub fn len_bytes(&self) -> u32 {
		self.text.len_bytes() as u32
	}

	pub fn byte_text(&self, range: std::ops::Range<u32>) -> String {
		self.text().byte_text(range)
	}

	pub fn tree(&self) -> &Tree {
		self.syntax.tree()
	}

	pub fn root_node(&self) -> Node<'_> {
		self.tree().root_node()
	}

	pub fn tree_for_byte_range(&self, start: u32, end: u32) -> &Tree {
		self.syntax.tree_for_byte_range(start, end)
	}

	pub fn root_layer(&self) -> Layer {
		self.syntax.root()
	}

	pub fn root_language(&self) -> Language {
		self.layer(self.root_layer()).language
	}

	pub fn layer(&self, layer: Layer) -> &crate::LayerData {
		self.syntax.layer(layer)
	}

	pub fn layer_for_byte_range(&self, start: u32, end: u32) -> Layer {
		self.syntax.layer_for_byte_range(start, end)
	}

	pub fn layers_for_byte_range(&self, start: u32, end: u32) -> impl Iterator<Item = Layer> + '_ {
		self.syntax.layers_for_byte_range(start, end)
	}

	pub fn named_node_at(&self, start: u32, end: u32) -> Option<Node<'_>> {
		self.syntax.named_descendant_for_byte_range(start, end)
	}

	pub fn node_at(&self, start: u32, end: u32) -> Option<Node<'_>> {
		self.syntax.descendant_for_byte_range(start, end)
	}

	pub fn walk(&self) -> TreeCursor<'_> {
		self.syntax.walk()
	}

	pub fn highlight_spans<'a, Loader>(
		&'a self, loader: &'a Loader, range: impl RangeBounds<u32>,
	) -> HighlightSpans<'a, Loader>
	where
		Loader: LanguageLoader, {
		HighlightSpans::new(self.syntax(), self.rope_slice(), loader, range)
	}

	pub fn locals_at(&self, byte: u32) -> LocalScope<'_> {
		let layer = self.layer_for_byte_range(byte, byte);
		let locals = self.layer(layer).locals();
		LocalScope {
			locals,
			scope: locals.scope_cursor(byte).current_scope(),
		}
	}

	pub fn query_iter<'a, Loader, State>(
		&'a self, loader: Loader, range: impl RangeBounds<u32>,
	) -> QueryIter<'a, 'a, Loader, State>
	where
		Loader: QueryLoader<'a>,
		State: Default, {
		QueryIter::new(self.syntax(), self.rope_slice(), loader, range)
	}

	pub fn layer_count(&self) -> usize {
		self.syntax.layer_count()
	}

	pub fn matched_capture_nodes<'a>(
		&'a self, query: &'a Query, capture: Capture, node: Node<'a>,
	) -> Vec<Vec<Node<'a>>> {
		let mut cursor = InactiveQueryCursor::new(0..u32::MAX, TREE_SITTER_MATCH_LIMIT).execute_query(
			query,
			&node,
			RopeInput::new(self.rope_slice()),
		);
		let mut matched = Vec::new();
		while let Some(mat) = cursor.next_match() {
			let nodes: Vec<_> = mat.nodes_for_capture(capture).cloned().collect();
			if !nodes.is_empty() {
				matched.push(nodes);
			}
		}
		matched
	}
	pub(crate) fn rope_slice(&self) -> RopeSlice<'_> {
		self.text.slice(..)
	}
}
