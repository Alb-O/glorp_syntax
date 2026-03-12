use {
	crate::LanguageLoader,
	liney_tree_house::{self as tree_house, highlighter::HighlightEvent},
	ropey::RopeSlice,
	std::ops::{Bound, RangeBounds},
};

pub use tree_house::highlighter::Highlight;

/// A byte span with an associated tree-sitter highlight.
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

/// Iterator wrapper that emits contiguous highlight spans.
pub struct Highlighter<'a, Loader>
where
	Loader: LanguageLoader,
{
	inner: tree_house::highlighter::Highlighter<'a, 'a, Loader>,
	end_byte: u32,
	current_start: u32,
	current_highlight: Option<Highlight>,
	base: u32,
	end_doc: u32,
}

impl<'a, Loader> Highlighter<'a, Loader>
where
	Loader: LanguageLoader,
{
	fn doc_offset(base: u32, local: u32) -> u32 {
		if local == u32::MAX {
			u32::MAX
		} else {
			base.saturating_add(local)
		}
	}

	pub fn new(
		syntax: &'a tree_house::Syntax, source: RopeSlice<'a>, loader: &'a Loader, range: impl RangeBounds<u32>,
	) -> Self {
		let start = match range.start_bound() {
			Bound::Included(&n) => n,
			Bound::Excluded(&n) => n + 1,
			Bound::Unbounded => 0,
		};
		let end = match range.end_bound() {
			Bound::Included(&n) => n + 1,
			Bound::Excluded(&n) => n,
			Bound::Unbounded => source.len_bytes() as u32,
		};

		let inner = tree_house::highlighter::Highlighter::new(syntax, source, loader, start..end);
		Self {
			current_start: Self::doc_offset(0, inner.next_event_offset()),
			inner,
			end_byte: end,
			current_highlight: None,
			base: 0,
			end_doc: end,
		}
	}

	pub fn new_mapped(
		syntax: &'a tree_house::Syntax, source: RopeSlice<'a>, loader: &'a Loader, doc_range: impl RangeBounds<u32>,
		base: u32, end_doc: u32,
	) -> Self {
		let start_doc = match doc_range.start_bound() {
			Bound::Included(&n) => n,
			Bound::Excluded(&n) => n + 1,
			Bound::Unbounded => 0,
		};
		let end_doc_req = match doc_range.end_bound() {
			Bound::Included(&n) => n + 1,
			Bound::Excluded(&n) => n,
			Bound::Unbounded => u32::MAX,
		};

		let start_local = start_doc.saturating_sub(base);
		let end_local = end_doc_req.saturating_sub(base).min(end_doc.saturating_sub(base));

		let inner = tree_house::highlighter::Highlighter::new(syntax, source, loader, start_local..end_local);
		Self {
			current_start: Self::doc_offset(base, inner.next_event_offset()),
			inner,
			end_byte: end_local,
			current_highlight: None,
			base,
			end_doc: base.saturating_add(end_local),
		}
	}

	pub fn next_event_offset(&self) -> u32 {
		Self::doc_offset(self.base, self.inner.next_event_offset())
	}

	pub fn is_done(&self) -> bool {
		self.next_event_offset() >= self.end_doc
	}

	pub fn collect_spans(self) -> Vec<HighlightSpan> {
		self.collect()
	}

	fn close_span(&self, event_start: u32) -> Option<HighlightSpan> {
		self.current_highlight.and_then(|highlight| {
			if event_start == u32::MAX {
				return None;
			}
			(self.current_start < event_start).then_some(HighlightSpan {
				start: self.current_start,
				end: event_start,
				highlight,
			})
		})
	}
}

impl<'a, Loader> Iterator for Highlighter<'a, Loader>
where
	Loader: LanguageLoader,
{
	type Item = HighlightSpan;

	fn next(&mut self) -> Option<Self::Item> {
		while self.inner.next_event_offset() < self.end_byte {
			let event_start_local = self.inner.next_event_offset();
			if event_start_local == u32::MAX {
				break;
			}
			let event_start_doc = Self::doc_offset(self.base, event_start_local);
			let (event, mut highlights) = self.inner.advance();
			let new_highlight = highlights.next_back();

			let span = self.close_span(event_start_doc);
			self.current_start = event_start_doc;

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
			let local_offset = self.inner.next_event_offset();
			let end_doc = if local_offset == u32::MAX {
				self.end_doc
			} else {
				Self::doc_offset(self.base, local_offset).min(self.end_doc)
			};

			if self.current_start < end_doc {
				return Some(HighlightSpan {
					start: self.current_start,
					end: end_doc,
					highlight,
				});
			}
		}

		None
	}
}
