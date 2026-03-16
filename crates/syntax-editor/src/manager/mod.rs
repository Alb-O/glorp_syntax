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
pub struct DocumentId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewportKey(pub u32);

#[derive(Debug, Clone)]
pub struct InstalledSyntax {
	pub syntax: Syntax,
	pub doc_version: u64,
	pub tree_id: u64,
}

#[derive(Debug, Clone)]
pub struct ViewportSyntax {
	pub syntax: Syntax,
	pub doc_version: u64,
	pub tree_id: u64,
	pub coverage: Range<u32>,
}

#[derive(Debug, Default, Clone)]
pub struct ViewportEntry {
	pub stage_a: Option<ViewportSyntax>,
	pub stage_b: Option<ViewportSyntax>,
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
pub struct ViewportCache {
	cap: usize,
	order: VecDeque<ViewportKey>,
	pub map: HashMap<ViewportKey, ViewportEntry>,
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

	pub fn touch(&mut self, key: ViewportKey) {
		self.promote(key);
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
pub struct SyntaxSlot {
	pub full: Option<InstalledSyntax>,
	pub viewport_cache: ViewportCache,
	pub dirty: bool,
	pub updated: bool,
	pub change_id: u64,
	pub next_tree_id: u64,
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
	pub syntax: &'a Syntax,
	pub tree_id: u64,
	pub tree_doc_version: u64,
	pub coverage: Option<Range<u32>>,
}

#[derive(Debug, Default, Clone)]
pub struct SyntaxManager {
	entries: HashMap<DocumentId, SyntaxSlot>,
}

impl SyntaxManager {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn document(&self, doc_id: DocumentId) -> Option<&SyntaxSlot> {
		self.entries.get(&doc_id)
	}

	pub fn document_mut(&mut self, doc_id: DocumentId) -> &mut SyntaxSlot {
		self.entries.entry(doc_id).or_default()
	}

	pub fn remove_document(&mut self, doc_id: DocumentId) -> Option<SyntaxSlot> {
		self.entries.remove(&doc_id)
	}

	pub fn has_syntax(&self, doc_id: DocumentId) -> bool {
		self.document(doc_id).is_some_and(SyntaxSlot::has_any_tree)
	}

	pub fn is_dirty(&self, doc_id: DocumentId) -> bool {
		self.document(doc_id).is_some_and(|slot| slot.dirty)
	}

	pub fn mark_dirty(&mut self, doc_id: DocumentId) {
		let slot = self.document_mut(doc_id);
		slot.dirty = true;
	}

	pub fn syntax_version(&self, doc_id: DocumentId) -> u64 {
		self.document(doc_id).map_or(0, |slot| slot.change_id)
	}

	pub fn syntax_for_doc(&self, doc_id: DocumentId) -> Option<&Syntax> {
		let slot = self.document(doc_id)?;
		if let Some(full) = slot.full.as_ref() {
			return Some(&full.syntax);
		}
		slot.viewport_cache
			.entries_mru()
			.find_map(|entry| entry.stages().next().map(|(tree, _)| &tree.syntax))
	}

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
			.or(best_any)
			.map(|(selection, _)| selection.into_selection())
	}

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

	pub fn install_viewport_stage_a(
		&mut self, doc_id: DocumentId, key: ViewportKey, syntax: Syntax, coverage: Range<u32>, doc_version: u64,
	) -> u64 {
		self.install_viewport(doc_id, key, syntax, coverage, doc_version, false)
	}

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
