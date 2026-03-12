#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Revision(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SnapshotId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
	pub range: std::ops::Range<u32>,
	pub replacement: String,
}

impl TextEdit {
	pub fn new(range: std::ops::Range<u32>, replacement: impl Into<String>) -> Self {
		Self {
			range,
			replacement: replacement.into(),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangeSet {
	edits: Vec<TextEdit>,
}

impl ChangeSet {
	pub fn new(edits: impl IntoIterator<Item = TextEdit>) -> Self {
		Self {
			edits: edits.into_iter().collect(),
		}
	}

	pub fn single(range: std::ops::Range<u32>, replacement: impl Into<String>) -> Self {
		Self::new([TextEdit::new(range, replacement)])
	}

	pub fn push(&mut self, edit: TextEdit) {
		self.edits.push(edit);
	}

	pub fn is_empty(&self) -> bool {
		self.edits.is_empty()
	}

	pub fn iter(&self) -> impl Iterator<Item = &TextEdit> {
		self.edits.iter()
	}
}

impl IntoIterator for ChangeSet {
	type Item = TextEdit;
	type IntoIter = std::vec::IntoIter<TextEdit>;

	fn into_iter(self) -> Self::IntoIter {
		self.edits.into_iter()
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateResult {
	pub revision: Revision,
	pub snapshot_id: SnapshotId,
	pub changed_ranges: Vec<std::ops::Range<u32>>,
	pub timed_out: bool,
	pub snapshot_changed: bool,
	pub affected_layers: usize,
}
