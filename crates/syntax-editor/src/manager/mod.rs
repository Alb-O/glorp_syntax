use {
	crate::Syntax,
	ropey::Rope,
	std::{
		collections::{HashMap, VecDeque},
		ops::Range,
	},
};

const FULL_TREE_MEMORY_CAP: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
/// Stable document key used by [`SyntaxManager`].
pub struct DocumentId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Stable key identifying one viewport entry inside a document slot.
pub struct ViewportKey(pub u32);

#[derive(Debug, Clone)]
struct InstalledSyntax {
	syntax: Syntax,
	doc_version: u64,
	tree_id: u64,
}

#[derive(Debug, Clone)]
struct ViewportSyntax {
	syntax: Syntax,
	doc_version: u64,
	tree_id: u64,
	coverage: Range<u32>,
}

#[derive(Debug, Default, Clone)]
struct ViewportEntry {
	stage_a: Option<ViewportSyntax>,
	stage_b: Option<ViewportSyntax>,
}

impl ViewportEntry {
	fn stages(&self) -> impl Iterator<Item = (&ViewportSyntax, bool)> + '_ {
		self.stage_b
			.iter()
			.map(|tree| (tree, true))
			.chain(self.stage_a.iter().map(|tree| (tree, false)))
	}

	fn has_any(&self) -> bool {
		self.stage_a.is_some() || self.stage_b.is_some()
	}

	fn best_doc_version(&self) -> Option<u64> {
		self.stages().map(|(tree, _)| tree.doc_version).max()
	}
}

#[derive(Debug, Clone)]
struct FullTreeMemoryEntry {
	content: Rope,
	syntax: Syntax,
}

/// MRU cache of viewport-bounded parse results.
#[derive(Debug, Clone)]
struct ViewportCache {
	cap: usize,
	order: VecDeque<ViewportKey>,
	map: HashMap<ViewportKey, ViewportEntry>,
}

impl Default for ViewportCache {
	fn default() -> Self {
		Self::new(4)
	}
}

impl ViewportCache {
	pub fn new(cap: usize) -> Self {
		Self {
			cap,
			order: VecDeque::with_capacity(cap),
			map: HashMap::new(),
		}
	}

	pub fn iter_keys_mru(&self) -> impl Iterator<Item = ViewportKey> + '_ {
		self.order.iter().copied()
	}

	pub fn get_mut_or_insert(&mut self, key: ViewportKey) -> &mut ViewportEntry {
		if self.promote(key) {
			return self
				.map
				.get_mut(&key)
				.expect("viewport order and map must stay in sync");
		}

		if self.order.len() >= self.cap
			&& let Some(evicted) = self.order.pop_back()
		{
			self.map.remove(&evicted);
		}
		self.order.push_front(key);
		self.map.entry(key).or_default()
	}

	pub fn entries_mru(&self) -> impl Iterator<Item = &ViewportEntry> + '_ {
		self.iter_keys_mru().filter_map(|key| self.map.get(&key))
	}

	pub fn clear(&mut self) {
		self.order.clear();
		self.map.clear();
	}

	pub fn has_any(&self) -> bool {
		self.map.values().any(ViewportEntry::has_any)
	}

	pub fn best_doc_version(&self) -> Option<u64> {
		self.map.values().filter_map(ViewportEntry::best_doc_version).max()
	}
}

/// Per-document syntax state.
#[derive(Debug, Default, Clone)]
struct SyntaxSlot {
	full: Option<InstalledSyntax>,
	viewport_cache: ViewportCache,
	dirty: bool,
	updated: bool,
	change_id: u64,
	next_tree_id: u64,
	full_tree_memory: VecDeque<FullTreeMemoryEntry>,
}

impl SyntaxSlot {
	pub fn take_updated(&mut self) -> bool {
		let updated = self.updated;
		self.updated = false;
		updated
	}

	pub fn alloc_tree_id(&mut self) -> u64 {
		let id = self.next_tree_id;
		self.next_tree_id = self.next_tree_id.wrapping_add(1);
		id
	}

	pub fn has_any_tree(&self) -> bool {
		self.full.is_some() || self.viewport_cache.has_any()
	}

	pub fn best_doc_version(&self) -> Option<u64> {
		let full_ver = self.full.as_ref().map(|tree| tree.doc_version);
		[full_ver, self.viewport_cache.best_doc_version()]
			.into_iter()
			.flatten()
			.max()
	}

	pub fn drop_full(&mut self) {
		self.full = None;
		self.full_tree_memory.clear();
	}

	pub fn drop_viewports(&mut self) {
		self.viewport_cache.clear();
	}

	pub fn drop_all_trees(&mut self) {
		self.drop_full();
		self.drop_viewports();
	}

	pub fn remember_full_tree_for_content(&mut self, content: &Rope) {
		let Some(full) = self.full.as_ref() else {
			return;
		};
		if self
			.full_tree_memory
			.front()
			.is_some_and(|entry| entry.content == *content)
		{
			return;
		}
		if let Some(pos) = self.full_tree_memory.iter().position(|entry| entry.content == *content) {
			self.full_tree_memory.remove(pos);
		}
		if self.full_tree_memory.len() >= FULL_TREE_MEMORY_CAP {
			self.full_tree_memory.pop_back();
		}
		self.full_tree_memory.push_front(FullTreeMemoryEntry {
			content: content.clone(),
			syntax: full.syntax.clone(),
		});
	}

	pub fn restore_full_tree_for_content(&mut self, content: &Rope, doc_version: u64) -> bool {
		let Some(pos) = self.full_tree_memory.iter().position(|entry| entry.content == *content) else {
			return false;
		};
		let remembered = self
			.full_tree_memory
			.remove(pos)
			.expect("full tree memory position must be valid");
		let tree_id = self.alloc_tree_id();
		self.full = Some(InstalledSyntax {
			syntax: remembered.syntax.clone(),
			doc_version,
			tree_id,
		});
		self.full_tree_memory.push_front(remembered);
		self.updated = true;
		self.change_id = self.change_id.wrapping_add(1);
		true
	}
}

impl ViewportCache {
	fn promote(&mut self, key: ViewportKey) -> bool {
		if self.order.front() == Some(&key) {
			return true;
		}
		let Some(pos) = self.order.iter().position(|entry| *entry == key) else {
			return false;
		};
		self.order.remove(pos);
		self.order.push_front(key);
		true
	}
}

/// Best syntax tree selected for a render viewport.
pub struct SyntaxSelection<'a> {
	/// Syntax tree chosen for the requested viewport.
	pub syntax: &'a Syntax,
	/// Monotonic tree identifier assigned by [`SyntaxManager`].
	pub tree_id: u64,
	/// Document version associated with the selected tree.
	pub tree_doc_version: u64,
	/// Covered byte range for viewport-local trees. `None` for full-document trees.
	pub coverage: Option<Range<u32>>,
}

#[derive(Debug, Default, Clone)]
/// Per-document syntax registry with full-tree and viewport-tree selection.
pub struct SyntaxManager {
	entries: HashMap<DocumentId, SyntaxSlot>,
}

impl SyntaxManager {
	/// Creates an empty syntax manager.
	pub fn new() -> Self {
		Self::default()
	}

	fn document(&self, doc_id: DocumentId) -> Option<&SyntaxSlot> {
		self.entries.get(&doc_id)
	}

	fn document_mut(&mut self, doc_id: DocumentId) -> &mut SyntaxSlot {
		self.entries.entry(doc_id).or_default()
	}

	/// Removes all syntax state for `doc_id`.
	pub fn remove_document(&mut self, doc_id: DocumentId) -> bool {
		self.entries.remove(&doc_id).is_some()
	}

	/// Returns whether any full or viewport tree is installed for `doc_id`.
	pub fn has_syntax(&self, doc_id: DocumentId) -> bool {
		self.document(doc_id).is_some_and(SyntaxSlot::has_any_tree)
	}

	/// Returns whether the document has been marked dirty since the last full install.
	pub fn is_dirty(&self, doc_id: DocumentId) -> bool {
		self.document(doc_id).is_some_and(|slot| slot.dirty)
	}

	/// Marks the document as needing a fresh parse.
	///
	/// Does nothing when `doc_id` is unknown.
	pub fn mark_dirty(&mut self, doc_id: DocumentId) {
		if let Some(slot) = self.entries.get_mut(&doc_id) {
			slot.dirty = true;
		}
	}

	/// Returns and clears the per-document "updated" flag.
	///
	/// Returns `false` when `doc_id` is unknown.
	pub fn take_updated(&mut self, doc_id: DocumentId) -> bool {
		self.entries.get_mut(&doc_id).is_some_and(SyntaxSlot::take_updated)
	}

	/// Drops the installed full-document tree for `doc_id`.
	///
	/// Does nothing when `doc_id` is unknown.
	pub fn drop_full(&mut self, doc_id: DocumentId) {
		if let Some(slot) = self.entries.get_mut(&doc_id) {
			slot.drop_full();
		}
	}

	/// Drops all installed viewport trees for `doc_id`.
	///
	/// Does nothing when `doc_id` is unknown.
	pub fn drop_viewports(&mut self, doc_id: DocumentId) {
		if let Some(slot) = self.entries.get_mut(&doc_id) {
			slot.drop_viewports();
		}
	}

	/// Drops all installed trees for `doc_id`.
	///
	/// Does nothing when `doc_id` is unknown.
	pub fn drop_all_trees(&mut self, doc_id: DocumentId) {
		if let Some(slot) = self.entries.get_mut(&doc_id) {
			slot.drop_all_trees();
		}
	}

	/// Caches the current full-document tree for later content-based restoration.
	///
	/// Does nothing when `doc_id` is unknown or has no full tree.
	pub fn remember_full_tree_for_content(&mut self, doc_id: DocumentId, content: &Rope) {
		if let Some(slot) = self.entries.get_mut(&doc_id) {
			slot.remember_full_tree_for_content(content);
		}
	}

	/// Restores a cached full-document tree for `content` if one is available.
	///
	/// Returns `false` when `doc_id` is unknown or the content was not cached.
	pub fn restore_full_tree_for_content(&mut self, doc_id: DocumentId, content: &Rope, doc_version: u64) -> bool {
		self.entries
			.get_mut(&doc_id)
			.is_some_and(|slot| slot.restore_full_tree_for_content(content, doc_version))
	}

	/// Returns the manager-local change counter for `doc_id`.
	pub fn syntax_version(&self, doc_id: DocumentId) -> u64 {
		self.document(doc_id).map_or(0, |slot| slot.change_id)
	}

	/// Returns the installed full-document syntax tree for the document, if present.
	pub fn full_syntax_for_doc(&self, doc_id: DocumentId) -> Option<&Syntax> {
		self.document(doc_id)
			.and_then(|slot| slot.full.as_ref().map(|full| &full.syntax))
	}

	/// Returns the best currently installed syntax tree for the document.
	///
	/// Unlike [`Self::full_syntax_for_doc`], this may return a viewport-local tree
	/// and therefore `coverage: Some(..)`.
	pub fn best_syntax_for_doc(&self, doc_id: DocumentId) -> Option<SyntaxSelection<'_>> {
		let slot = self.document(doc_id)?;
		if let Some(full) = slot.full.as_ref() {
			return Some(SyntaxSelection {
				syntax: &full.syntax,
				tree_id: full.tree_id,
				tree_doc_version: full.doc_version,
				coverage: None,
			});
		}
		slot.viewport_cache.entries_mru().find_map(|entry| {
			// The first MRU stage is the best available partial fallback for doc-wide consumers.
			entry.stages().next().map(|(tree, _)| SyntaxSelection {
				syntax: &tree.syntax,
				tree_id: tree.tree_id,
				tree_doc_version: tree.doc_version,
				coverage: Some(tree.coverage.clone()),
			})
		})
	}

	/// Selects the best installed syntax tree for a render viewport.
	///
	/// This API is render-oriented: if no installed tree overlaps `viewport`, it falls back
	/// to the freshest available tree so drawing can continue while a better viewport parse
	/// is still in flight. Callers must check the returned `coverage` before using the tree
	/// for non-rendering features.
	pub fn syntax_for_viewport(
		&self, doc_id: DocumentId, doc_version: u64, viewport: Range<u32>,
	) -> Option<SyntaxSelection<'_>> {
		let slot = self.document(doc_id)?;
		let mut best_overlapping: Option<ScoredSelection<'_>> = None;
		let mut best_any: Option<ScoredSelection<'_>> = None;

		if let Some(full) = slot.full.as_ref() {
			consider_candidate(
				&mut best_overlapping,
				&mut best_any,
				CandidateSelection {
					syntax: &full.syntax,
					tree_id: full.tree_id,
					tree_doc_version: full.doc_version,
					coverage: None,
				},
				false,
				doc_version,
				&viewport,
			);
		}

		for entry in slot.viewport_cache.entries_mru() {
			for (tree, enriched) in entry.stages() {
				consider_candidate(
					&mut best_overlapping,
					&mut best_any,
					CandidateSelection {
						syntax: &tree.syntax,
						tree_id: tree.tree_id,
						tree_doc_version: tree.doc_version,
						coverage: Some(&tree.coverage),
					},
					enriched,
					doc_version,
					&viewport,
				);
			}
		}

		best_overlapping
			// Prefer trees that actually cover the requested viewport, but fall back to the
			// freshest installed tree so rendering can continue while viewport parses catch up.
			.or(best_any)
			.map(|(selection, _)| selection.into_selection())
	}

	/// Installs a full-document syntax tree.
	pub fn install_full(&mut self, doc_id: DocumentId, syntax: Syntax, doc_version: u64) -> u64 {
		let slot = self.document_mut(doc_id);
		if slot
			.full
			.as_ref()
			.is_some_and(|current| current.doc_version > doc_version)
		{
			return slot.change_id;
		}
		let tree_id = slot.alloc_tree_id();
		slot.full = Some(InstalledSyntax {
			syntax,
			doc_version,
			tree_id,
		});
		slot.dirty = false;
		slot.updated = true;
		slot.change_id = slot.change_id.wrapping_add(1);
		slot.change_id
	}

	/// Installs the fast viewport parse for a viewport key.
	pub fn install_viewport_stage_a(
		&mut self, doc_id: DocumentId, key: ViewportKey, syntax: Syntax, coverage: Range<u32>, doc_version: u64,
	) -> u64 {
		self.install_viewport(doc_id, key, syntax, coverage, doc_version, false)
	}

	/// Installs the enriched viewport parse for a viewport key.
	pub fn install_viewport_stage_b(
		&mut self, doc_id: DocumentId, key: ViewportKey, syntax: Syntax, coverage: Range<u32>, doc_version: u64,
	) -> u64 {
		self.install_viewport(doc_id, key, syntax, coverage, doc_version, true)
	}

	fn install_viewport(
		&mut self, doc_id: DocumentId, key: ViewportKey, syntax: Syntax, coverage: Range<u32>, doc_version: u64,
		enriched: bool,
	) -> u64 {
		let slot = self.document_mut(doc_id);
		if slot.best_doc_version().is_some_and(|current| current > doc_version) {
			return slot.change_id;
		}
		let tree_id = slot.alloc_tree_id();
		let entry = slot.viewport_cache.get_mut_or_insert(key);
		let target = if enriched {
			&mut entry.stage_b
		} else {
			&mut entry.stage_a
		};
		*target = Some(ViewportSyntax {
			syntax,
			doc_version,
			tree_id,
			coverage,
		});
		slot.updated = true;
		slot.change_id = slot.change_id.wrapping_add(1);
		slot.change_id
	}
}

fn overlaps(coverage: Option<&Range<u32>>, viewport: &Range<u32>) -> bool {
	match coverage {
		None => true,
		Some(coverage) => viewport.start < coverage.end && viewport.end > coverage.start,
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateScore {
	matches_doc_version: bool,
	is_full: bool,
	enriched: bool,
	tree_doc_version: u64,
}

impl CandidateScore {
	fn new(tree_doc_version: u64, is_full: bool, enriched: bool, doc_version: u64) -> Self {
		Self {
			matches_doc_version: tree_doc_version == doc_version,
			is_full,
			enriched,
			tree_doc_version,
		}
	}
}

struct CandidateSelection<'a> {
	syntax: &'a Syntax,
	tree_id: u64,
	tree_doc_version: u64,
	coverage: Option<&'a Range<u32>>,
}

impl<'a> CandidateSelection<'a> {
	fn into_selection(self) -> SyntaxSelection<'a> {
		SyntaxSelection {
			syntax: self.syntax,
			tree_id: self.tree_id,
			tree_doc_version: self.tree_doc_version,
			coverage: self.coverage.cloned(),
		}
	}
}

type ScoredSelection<'a> = (CandidateSelection<'a>, CandidateScore);

fn consider_candidate<'a>(
	best_overlapping: &mut Option<ScoredSelection<'a>>, best_any: &mut Option<ScoredSelection<'a>>,
	selection: CandidateSelection<'a>, enriched: bool, doc_version: u64, viewport: &Range<u32>,
) {
	let score = CandidateScore::new(
		selection.tree_doc_version,
		selection.coverage.is_none(),
		enriched,
		doc_version,
	);
	if overlaps(selection.coverage, viewport) {
		if best_overlapping.as_ref().is_none_or(|(_, prev)| score > *prev) {
			*best_overlapping = Some((selection, score));
		}
	} else if best_any.as_ref().is_none_or(|(_, prev)| score > *prev) {
		*best_any = Some((selection, score));
	}
}

#[cfg(test)]
mod tests;
