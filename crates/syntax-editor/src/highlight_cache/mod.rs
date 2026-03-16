use {
	crate::{DocumentId, Highlight, HighlightSpan, LanguageLoader, RenderSyntaxSelection},
	ropey::Rope,
	std::collections::{HashMap, VecDeque},
};

/// Number of lines per cached highlight tile.
pub const TILE_SIZE: usize = 128;

const MAX_TILES: usize = 16;

/// Cache key for a highlight tile.
///
/// Highlight tiles are scoped not just to a document and syntax version, but
/// also to the selected tree. This prevents viewport trees for different
/// regions from aliasing inside one document cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HighlightKey {
	/// Syntax-manager version used to invalidate tiles after tree changes.
	pub syntax_version: u64,
	/// Selected syntax tree identifier used to distinguish concurrent viewport trees.
	pub tree_id: u64,
	/// Theme/version token used to invalidate tiles after style changes.
	pub theme_epoch: u64,
	/// Zero-based tile index in `TILE_SIZE` line units.
	pub tile_idx: usize,
}

/// A cached highlight tile.
#[derive(Debug, Clone)]
pub struct HighlightTile<S> {
	/// Cache identity for this tile.
	pub key: HighlightKey,
	/// Highlight spans and resolved styles stored for the tile.
	pub spans: Vec<(HighlightSpan, S)>,
}

/// Query parameters for loading highlighted spans for a document window.
pub struct HighlightSpanQuery<'a, Loader, Resolve, S>
where
	Loader: LanguageLoader,
	Resolve: Fn(Highlight) -> S,
	S: Copy, {
	/// Document whose highlight tiles are being queried.
	pub doc_id: DocumentId,
	/// Syntax-manager version associated with the selected tree.
	pub syntax_version: u64,
	/// Full document text used for line-to-byte conversion.
	pub rope: &'a Rope,
	/// Render selection to highlight against.
	///
	/// Its `tree_id` is folded into the tile cache key automatically.
	pub selection: RenderSyntaxSelection<'a>,
	/// Language/query loader used by the selected syntax tree.
	pub loader: &'a Loader,
	/// Maps opaque highlight ids to caller-owned style data.
	pub style_resolver: Resolve,
	/// Inclusive start line for the requested window.
	pub start_line: usize,
	/// Exclusive end line for the requested window.
	pub end_line: usize,
}

/// LRU cache for syntax highlight tiles.
///
/// Cached tiles are partitioned by document, selected tree, tile index, and
/// theme epoch.
#[derive(Debug)]
pub struct HighlightTiles<S> {
	tiles: Vec<HighlightTile<S>>,
	mru_order: VecDeque<usize>,
	max_tiles: usize,
	index: HashMap<DocumentId, HashMap<(u64, usize), usize>>,
	theme_epoch: u64,
}

impl<S> Default for HighlightTiles<S> {
	fn default() -> Self {
		Self::new()
	}
}

impl<S> HighlightTiles<S> {
	/// Creates a tile cache with the default capacity.
	pub fn new() -> Self {
		Self::with_capacity(MAX_TILES)
	}

	/// Creates a tile cache with space for at most `max_tiles` cached tiles.
	pub fn with_capacity(max_tiles: usize) -> Self {
		assert!(max_tiles > 0, "highlight tile cache capacity must be non-zero");
		Self {
			tiles: Vec::with_capacity(max_tiles),
			mru_order: VecDeque::with_capacity(max_tiles),
			max_tiles,
			index: HashMap::new(),
			theme_epoch: 0,
		}
	}

	/// Returns the current theme epoch.
	pub fn theme_epoch(&self) -> u64 {
		self.theme_epoch
	}

	/// Sets the theme epoch, clearing the cache when it changes.
	pub fn set_theme_epoch(&mut self, epoch: u64) {
		if epoch != self.theme_epoch {
			self.theme_epoch = epoch;
			self.clear();
		}
	}

	/// Removes all cached tiles.
	pub fn clear(&mut self) {
		self.tiles.clear();
		self.mru_order.clear();
		self.index.clear();
	}

	/// Invalidates every cached tile associated with `doc_id`.
	///
	/// This drops tiles for every cached tree selection under the document.
	pub fn invalidate_document(&mut self, doc_id: DocumentId) {
		let Some(indices) = self.index.remove(&doc_id) else {
			return;
		};
		let mut removed = vec![false; self.tiles.len()];
		for idx in indices.into_values() {
			removed[idx] = true;
		}
		self.compact_tiles(&removed);
	}

	/// Returns highlighted spans for the requested line window.
	///
	/// Tile boundaries are an internal cache detail; the returned spans are clipped to the
	/// requested line range.
	pub fn get_spans<Loader, Resolve>(
		&mut self, q: &HighlightSpanQuery<'_, Loader, Resolve, S>,
	) -> Vec<(HighlightSpan, S)>
	where
		Loader: LanguageLoader,
		Resolve: Fn(Highlight) -> S,
		S: Copy, {
		if q.start_line >= q.end_line {
			return Vec::new();
		}

		let start_byte = line_to_byte_or_eof(q.rope, q.start_line);
		let end_byte = line_to_byte_or_eof(q.rope, q.end_line);

		let start_tile = q.start_line / TILE_SIZE;
		let end_tile = (q.end_line.saturating_sub(1)) / TILE_SIZE;
		let mut spans = Vec::new();

		for tile_idx in start_tile..=end_tile {
			let key = HighlightKey {
				syntax_version: q.syntax_version,
				tree_id: q.selection.tree_id(),
				theme_epoch: self.theme_epoch,
				tile_idx,
			};
			let tile_index = self.get_or_build_tile_index(q, key);
			for (span, style) in &self.tiles[tile_index].spans {
				let start = span.start.max(start_byte);
				let end = span.end.min(end_byte);
				if start < end {
					spans.push((
						HighlightSpan {
							start,
							end,
							highlight: span.highlight,
						},
						*style,
					));
				}
			}
		}

		spans
	}

	fn get_or_build_tile_index<Loader, Resolve>(
		&mut self, q: &HighlightSpanQuery<'_, Loader, Resolve, S>, key: HighlightKey,
	) -> usize
	where
		Loader: LanguageLoader,
		Resolve: Fn(Highlight) -> S,
		S: Copy, {
		// `syntax_version` and `theme_epoch` still live in `HighlightKey`, but
		// tree identity has to be part of the lookup key to avoid cross-viewport aliasing.
		let lookup = (key.tree_id, key.tile_idx);
		if let Some(&idx) = self.index.get(&q.doc_id).and_then(|doc| doc.get(&lookup))
			&& self.tiles[idx].key == key
		{
			self.touch(idx);
			return idx;
		}

		let tile_start_line = key.tile_idx * TILE_SIZE;
		let tile_end_line = ((key.tile_idx + 1) * TILE_SIZE).min(q.rope.len_lines());
		let spans = build_tile_spans(
			q.rope,
			&q.selection,
			q.loader,
			&q.style_resolver,
			tile_start_line,
			tile_end_line,
		);
		self.insert_tile(q.doc_id, HighlightTile { key, spans })
	}

	fn touch(&mut self, idx: usize) {
		if self.mru_order.front() == Some(&idx) {
			return;
		}
		if let Some(pos) = self.mru_order.iter().position(|entry| *entry == idx) {
			self.mru_order.remove(pos);
			self.mru_order.push_front(idx);
		}
	}

	fn insert_tile(&mut self, doc_id: DocumentId, tile: HighlightTile<S>) -> usize {
		let lookup = (tile.key.tree_id, tile.key.tile_idx);
		if let Some(existing_idx) = self.take_tile_index(doc_id, lookup) {
			self.tiles[existing_idx] = tile;
			self.touch(existing_idx);
			self.index.entry(doc_id).or_default().insert(lookup, existing_idx);
			return existing_idx;
		}

		if self.tiles.len() == self.max_tiles
			&& let Some(evicted_idx) = self.mru_order.pop_back()
		{
			self.index.retain(|_, doc_tiles| {
				doc_tiles.retain(|_, idx| *idx != evicted_idx);
				!doc_tiles.is_empty()
			});
			self.tiles[evicted_idx] = tile;
			self.mru_order.push_front(evicted_idx);
			self.index.entry(doc_id).or_default().insert(lookup, evicted_idx);
			return evicted_idx;
		}

		let idx = self.tiles.len();
		self.tiles.push(tile);
		self.mru_order.push_front(idx);
		self.index.entry(doc_id).or_default().insert(lookup, idx);
		idx
	}

	fn take_tile_index(&mut self, doc_id: DocumentId, lookup: (u64, usize)) -> Option<usize> {
		let idx = self
			.index
			.get_mut(&doc_id)
			.and_then(|doc_tiles| doc_tiles.remove(&lookup));
		if self.index.get(&doc_id).is_some_and(HashMap::is_empty) {
			self.index.remove(&doc_id);
		}
		idx
	}

	fn compact_tiles(&mut self, removed: &[bool]) {
		if !removed.iter().any(|removed| *removed) {
			return;
		}

		let mut remap = vec![None; self.tiles.len()];
		let kept = self.tiles.len() - removed.iter().filter(|removed| **removed).count();
		let mut next = 0usize;
		let mut tiles = Vec::with_capacity(kept);
		for (idx, tile) in self.tiles.drain(..).enumerate() {
			if removed[idx] {
				continue;
			}
			remap[idx] = Some(next);
			tiles.push(tile);
			next += 1;
		}
		self.tiles = tiles;

		self.mru_order.retain(|idx| !removed[*idx]);
		for idx in &mut self.mru_order {
			*idx = remap[*idx].expect("kept MRU entry must have a remapped tile index");
		}

		self.index.retain(|_, doc_tiles| {
			doc_tiles.retain(|_, idx| {
				if removed[*idx] {
					return false;
				}
				*idx = remap[*idx].expect("kept tile index must be remapped");
				true
			});
			!doc_tiles.is_empty()
		});
	}
}

fn line_to_byte_or_eof(rope: &Rope, line: usize) -> u32 {
	if line < rope.len_lines() {
		rope.line_to_byte(line) as u32
	} else {
		rope.len_bytes() as u32
	}
}

fn build_tile_spans<Loader, Resolve, S>(
	rope: &Rope, selection: &RenderSyntaxSelection<'_>, loader: &Loader, style_resolver: &Resolve, start_line: usize,
	end_line: usize,
) -> Vec<(HighlightSpan, S)>
where
	Loader: LanguageLoader,
	Resolve: Fn(Highlight) -> S,
	S: Copy, {
	let rope_len_bytes = rope.len_bytes() as u32;
	let tile_start_byte = line_to_byte_or_eof(rope, start_line);
	let tile_end_byte = line_to_byte_or_eof(rope, end_line);

	let syntax = selection.syntax();
	if syntax.root_end_byte() > rope_len_bytes {
		return Vec::new();
	}
	let spans = syntax.highlight_spans(loader, tile_start_byte..tile_end_byte);

	spans
		.filter_map(|mut span| {
			span.start = span.start.max(tile_start_byte).min(tile_end_byte);
			span.end = span.end.max(tile_start_byte).min(tile_end_byte);
			(span.start < span.end).then(|| (span, style_resolver(span.highlight)))
		})
		.collect()
}

#[cfg(test)]
mod tests;
