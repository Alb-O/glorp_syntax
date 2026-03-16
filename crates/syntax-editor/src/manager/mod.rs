use {
	crate::RenderSyntax,
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
struct InstalledRenderSyntax {
	syntax: RenderSyntax,
	doc_version: u64,
	tree_id: u64,
}

#[derive(Debug, Default, Clone)]
struct ViewportEntry {
	stage_a: Option<InstalledRenderSyntax>,
	stage_b: Option<InstalledRenderSyntax>,
}

impl ViewportEntry {
	fn stages(&self) -> impl Iterator<Item = (&InstalledRenderSyntax, bool)> + '_ {
		self.stage_b
			.iter()
			.map(|tree| (tree, true))
			.chain(self.stage_a.iter().map(|tree| (tree, false)))
	}

	fn best_doc_version(&self) -> Option<u64> {
		self.stages().map(|(tree, _)| tree.doc_version).max()
	}
}

#[derive(Debug, Clone)]
struct FullTreeMemoryEntry {
	content: Rope,
	compatibility_key: u64,
	syntax: RenderSyntax,
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
		self.iter_keys_mru()
			.map(|key| self.map.get(&key).expect("viewport order and map must stay in sync"))
	}

	pub fn clear(&mut self) {
		self.order.clear();
		self.map.clear();
	}

	pub fn has_any(&self) -> bool {
		!self.map.is_empty()
	}

	pub fn best_doc_version(&self) -> Option<u64> {
		self.map.values().filter_map(ViewportEntry::best_doc_version).max()
	}

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

/// Best render tree selected for a viewport query.
#[derive(Debug, Clone, Copy)]
pub struct RenderSyntaxSelection<'a> {
	syntax: &'a RenderSyntax,
	tree_id: u64,
	tree_doc_version: u64,
}

impl<'a> RenderSyntaxSelection<'a> {
	fn new(syntax: &'a RenderSyntax, tree_id: u64, tree_doc_version: u64) -> Self {
		Self {
			syntax,
			tree_id,
			tree_doc_version,
		}
	}

	/// Returns the selected render tree.
	pub fn syntax(&self) -> &'a RenderSyntax {
		self.syntax
	}

	/// Returns the selected tree's monotonic manager-local identifier.
	pub fn tree_id(&self) -> u64 {
		self.tree_id
	}

	/// Returns the document version associated with the selected tree.
	pub fn tree_doc_version(&self) -> u64 {
		self.tree_doc_version
	}

	/// Returns the selected viewport coverage in document byte coordinates.
	///
	/// Full-document selections return `None`.
	pub fn coverage(&self) -> Option<Range<u32>> {
		self.syntax.coverage()
	}
}

/// Per-document syntax registry with full-tree and viewport-tree selection.
#[derive(Debug, Default, Clone)]
pub struct SyntaxManager {
	entries: HashMap<DocumentId, SyntaxSlot>,
}

/// Per-document syntax state.
#[derive(Debug, Default, Clone)]
struct SyntaxSlot {
	full: Option<InstalledRenderSyntax>,
	viewport_cache: ViewportCache,
	dirty: bool,
	updated: bool,
	change_id: u64,
	next_tree_id: u64,
	full_tree_memory: VecDeque<FullTreeMemoryEntry>,
}

impl SyntaxSlot {
	fn full_tree_memory_pos(&self, content: &Rope, compatibility_key: u64) -> Option<usize> {
		self.full_tree_memory
			.iter()
			.position(|entry| entry.content == *content && entry.compatibility_key == compatibility_key)
	}

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
		full_ver.max(self.viewport_cache.best_doc_version())
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

	pub fn remember_full_tree_for_content(&mut self, content: &Rope, compatibility_key: u64) {
		let Some(full) = self.full.as_ref() else {
			return;
		};
		match self.full_tree_memory_pos(content, compatibility_key) {
			Some(0) => return,
			Some(pos) => {
				self.full_tree_memory.remove(pos);
			}
			None => {}
		}
		if self.full_tree_memory.len() >= FULL_TREE_MEMORY_CAP {
			self.full_tree_memory.pop_back();
		}
		self.full_tree_memory.push_front(FullTreeMemoryEntry {
			content: content.clone(),
			compatibility_key,
			syntax: full.syntax.clone(),
		});
	}

	pub fn restore_full_tree_for_content(&mut self, content: &Rope, compatibility_key: u64, doc_version: u64) -> bool {
		let Some(pos) = self.full_tree_memory_pos(content, compatibility_key) else {
			return false;
		};
		let remembered = self
			.full_tree_memory
			.remove(pos)
			.expect("full tree memory position must be valid");
		let tree_id = self.alloc_tree_id();
		self.full = Some(InstalledRenderSyntax {
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
	///
	/// `compatibility_key` must change whenever language, loader, or parse options change.
	/// Typical callers use an editor-owned token derived from grammar mode and
	/// runtime/query configuration.
	pub fn remember_full_tree_for_content(&mut self, doc_id: DocumentId, content: &Rope, compatibility_key: u64) {
		if let Some(slot) = self.entries.get_mut(&doc_id) {
			slot.remember_full_tree_for_content(content, compatibility_key);
		}
	}

	/// Restores a cached full-document tree for `content` if one is available.
	///
	/// Returns `false` when `doc_id` is unknown or the content was not cached.
	///
	/// `compatibility_key` must match the key used when the tree was remembered.
	/// A content match alone is not sufficient.
	pub fn restore_full_tree_for_content(
		&mut self, doc_id: DocumentId, content: &Rope, compatibility_key: u64, doc_version: u64,
	) -> bool {
		self.entries
			.get_mut(&doc_id)
			.is_some_and(|slot| slot.restore_full_tree_for_content(content, compatibility_key, doc_version))
	}

	/// Returns the manager-local change counter for `doc_id`.
	pub fn syntax_version(&self, doc_id: DocumentId) -> u64 {
		self.document(doc_id).map_or(0, |slot| slot.change_id)
	}

	/// Selects the best installed render tree for a viewport.
	pub fn syntax_for_viewport(
		&self, doc_id: DocumentId, doc_version: u64, viewport: Range<u32>,
	) -> Option<RenderSyntaxSelection<'_>> {
		let slot = self.document(doc_id)?;
		let mut best_overlapping: Option<(&InstalledRenderSyntax, CandidateScore)> = None;
		let mut best_any: Option<(&InstalledRenderSyntax, CandidateScore)> = None;

		if let Some(full) = slot.full.as_ref() {
			consider_candidate(
				&mut best_overlapping,
				&mut best_any,
				full,
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
					tree,
					enriched,
					doc_version,
					&viewport,
				);
			}
		}

		// Rendering prefers coverage over freshness: if anything overlaps the requested viewport,
		// pick the best overlapping tree; only fall back to the freshest non-overlapping tree when
		// that keeps the UI drawing while a better viewport parse is still in flight.
		best_overlapping
			.or(best_any)
			.map(|(tree, _)| RenderSyntaxSelection::new(&tree.syntax, tree.tree_id, tree.doc_version))
	}

	/// Installs a full-document render tree.
	///
	/// Returns the updated manager-local syntax version. Older installs are ignored.
	pub fn install_full(&mut self, doc_id: DocumentId, syntax: RenderSyntax, doc_version: u64) -> u64 {
		assert!(syntax.is_full(), "install_full requires a full render tree");

		let slot = self.document_mut(doc_id);
		if slot
			.full
			.as_ref()
			.is_some_and(|current| current.doc_version > doc_version)
		{
			return slot.change_id;
		}
		let tree_id = slot.alloc_tree_id();
		slot.full = Some(InstalledRenderSyntax {
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
	///
	/// Returns the updated manager-local syntax version. Older installs are ignored.
	pub fn install_viewport_stage_a(
		&mut self, doc_id: DocumentId, key: ViewportKey, syntax: RenderSyntax, doc_version: u64,
	) -> u64 {
		self.install_viewport(doc_id, key, syntax, doc_version, false)
	}

	/// Installs the enriched viewport parse for a viewport key.
	///
	/// Returns the updated manager-local syntax version. Older installs are ignored.
	pub fn install_viewport_stage_b(
		&mut self, doc_id: DocumentId, key: ViewportKey, syntax: RenderSyntax, doc_version: u64,
	) -> u64 {
		self.install_viewport(doc_id, key, syntax, doc_version, true)
	}

	fn install_viewport(
		&mut self, doc_id: DocumentId, key: ViewportKey, syntax: RenderSyntax, doc_version: u64, enriched: bool,
	) -> u64 {
		assert!(
			syntax.is_viewport(),
			"viewport installs require a viewport-backed render tree"
		);

		let slot = self.document_mut(doc_id);
		// Once any newer tree lands for the document, older viewport work is just stale background
		// work and should not churn cache state or selection order.
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
		*target = Some(InstalledRenderSyntax {
			syntax,
			doc_version,
			tree_id,
		});
		slot.updated = true;
		slot.change_id = slot.change_id.wrapping_add(1);
		slot.change_id
	}
}

fn overlaps(coverage: Option<Range<u32>>, viewport: &Range<u32>) -> bool {
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

fn consider_candidate<'a>(
	best_overlapping: &mut Option<(&'a InstalledRenderSyntax, CandidateScore)>,
	best_any: &mut Option<(&'a InstalledRenderSyntax, CandidateScore)>, tree: &'a InstalledRenderSyntax,
	enriched: bool, doc_version: u64, viewport: &Range<u32>,
) {
	let score = CandidateScore::new(tree.doc_version, tree.syntax.is_full(), enriched, doc_version);
	if overlaps(tree.syntax.coverage(), viewport) {
		if best_overlapping.as_ref().is_none_or(|(_, prev)| score > *prev) {
			*best_overlapping = Some((tree, score));
		}
	} else if best_any.as_ref().is_none_or(|(_, prev)| score > *prev) {
		*best_any = Some((tree, score));
	}
}

#[cfg(test)]
mod tests;
