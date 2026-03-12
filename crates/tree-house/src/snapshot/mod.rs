use {
	crate::{
		Language, LanguageLoader, Layer, Syntax, TREE_SITTER_MATCH_LIMIT, TreeCursor,
		change::{Revision, SnapshotId},
		highlighter::{Highlight, HighlightEvent},
		locals::{Definition, Locals},
		query_iter::{QueryIter, QueryLoader},
		text::DocumentText,
	},
	ropey::{Rope, RopeSlice},
	std::{
		ops::{Bound, RangeBounds},
		sync::Arc,
	},
	tree_sitter::{Capture, InactiveQueryCursor, Node, Query, RopeInput, Tree},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
	pub start: u32,
	pub end: u32,
	pub highlight: Highlight,
}

impl HighlightSpan {
	pub fn range(&self) -> std::ops::Range<u32> {
		self.start..self.end
	}

	pub fn len(&self) -> u32 {
		self.end - self.start
	}

	pub fn is_empty(&self) -> bool {
		self.start >= self.end
	}
}

pub struct HighlightSpans<'a, Loader>
where
	Loader: LanguageLoader,
{
	inner: crate::highlighter::Highlighter<'a, 'a, Loader>,
	end_byte: u32,
	current_start: u32,
	current_highlight: Option<Highlight>,
}

impl<'a, Loader> HighlightSpans<'a, Loader>
where
	Loader: LanguageLoader,
{
	pub fn new(snapshot: &'a DocumentSnapshot, loader: &'a Loader, range: impl RangeBounds<u32>) -> Self {
		let start = match range.start_bound() {
			Bound::Included(&n) => n,
			Bound::Excluded(&n) => n + 1,
			Bound::Unbounded => 0,
		};
		let end = match range.end_bound() {
			Bound::Included(&n) => n + 1,
			Bound::Excluded(&n) => n,
			Bound::Unbounded => snapshot.len_bytes(),
		};

		let inner = crate::highlighter::Highlighter::new(snapshot.syntax(), snapshot.rope_slice(), loader, start..end);
		Self {
			current_start: inner.next_event_offset(),
			inner,
			end_byte: end,
			current_highlight: None,
		}
	}

	pub fn next_event_offset(&self) -> u32 {
		self.inner.next_event_offset()
	}

	pub fn is_done(&self) -> bool {
		self.next_event_offset() >= self.end_byte
	}

	pub fn collect_spans(self) -> Vec<HighlightSpan> {
		self.collect()
	}

	fn close_span(&self, event_start: u32) -> Option<HighlightSpan> {
		self.current_highlight.and_then(|highlight| {
			(self.current_start < event_start).then_some(HighlightSpan {
				start: self.current_start,
				end: event_start,
				highlight,
			})
		})
	}
}

impl<'a, Loader> Iterator for HighlightSpans<'a, Loader>
where
	Loader: LanguageLoader,
{
	type Item = HighlightSpan;

	fn next(&mut self) -> Option<Self::Item> {
		while self.inner.next_event_offset() < self.end_byte {
			let event_start = self.inner.next_event_offset();
			if event_start == u32::MAX {
				break;
			}
			let (event, mut highlights) = self.inner.advance();
			let new_highlight = highlights.next_back();

			let span = self.close_span(event_start);
			self.current_start = event_start;

			match event {
				HighlightEvent::Push => {
					if new_highlight.is_some() {
						self.current_highlight = new_highlight;
					}
				}
				HighlightEvent::Refresh => {
					self.current_highlight = new_highlight;
				}
			}

			if span.is_some() {
				return span;
			}
		}

		if let Some(highlight) = self.current_highlight.take() {
			let end = self.inner.next_event_offset().min(self.end_byte);
			if self.current_start < end {
				return Some(HighlightSpan {
					start: self.current_start,
					end,
					highlight,
				});
			}
		}

		None
	}
}

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
	pub(crate) fn new(id: SnapshotId, revision: Revision, generation: u64, text: Rope, syntax: Syntax) -> Self {
		Self {
			id,
			revision,
			generation,
			text: Arc::new(text),
			syntax: Arc::new(syntax),
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
		let end = range.end.min(self.len_bytes());
		let start = range.start.min(end);
		self.text.byte_slice(start as usize..end as usize).to_string()
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

	pub fn highlights<'a, Loader>(
		&'a self, loader: &'a Loader, range: impl RangeBounds<u32>,
	) -> HighlightSpans<'a, Loader>
	where
		Loader: LanguageLoader,
	{
		HighlightSpans::new(self, loader, range)
	}

	pub fn locals_at(&self, byte: u32) -> LocalScope<'_> {
		let layer = self.layer_for_byte_range(byte, byte);
		let locals = self.layer(layer).locals();
		let cursor = locals.scope_cursor(byte);
		let scope = cursor.current_scope();
		LocalScope { locals, scope }
	}

	pub fn query_iter<'a, Loader, State>(
		&'a self, loader: Loader, range: impl RangeBounds<u32>,
	) -> QueryIter<'a, 'a, Loader, State>
	where
		Loader: QueryLoader<'a>,
		State: Default,
	{
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
