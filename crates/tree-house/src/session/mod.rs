use {
	crate::{
		DocumentSnapshot, Error, Language, LanguageLoader, Syntax,
		change::{ChangeSet, Revision, SnapshotId, TextEdit, UpdateResult},
		text::{ByteRangeText, DocumentText, TextStorage},
		tree_sitter::{InputEdit, Point},
	},
	ropey::Rope,
	std::time::Duration,
};

const DEFAULT_PARSE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineConfig {
	pub parse_timeout: Duration,
}

impl Default for EngineConfig {
	fn default() -> Self {
		Self {
			parse_timeout: DEFAULT_PARSE_TIMEOUT,
		}
	}
}

#[derive(Debug, Clone)]
pub struct DocumentSession {
	language: Language,
	config: EngineConfig,
	revision: Revision,
	snapshot_id: SnapshotId,
	generation: u64,
	text: Rope,
	syntax: Syntax,
}

impl DocumentSession {
	pub fn new(
		language: Language, text: &impl TextStorage, loader: &impl LanguageLoader, config: EngineConfig,
	) -> Result<Self, Error> {
		let rope = text.to_rope();
		let syntax = Syntax::new(rope.slice(..), language, config.parse_timeout, loader)?;
		Ok(Self {
			language,
			config,
			revision: Revision(0),
			snapshot_id: SnapshotId(1),
			generation: 0,
			text: rope,
			syntax,
		})
	}

	pub fn language(&self) -> Language {
		self.language
	}

	pub fn revision(&self) -> Revision {
		self.revision
	}

	pub fn config(&self) -> EngineConfig {
		self.config
	}

	pub fn text(&self) -> DocumentText<'_> {
		DocumentText::new(self.text.slice(..))
	}

	pub fn len_bytes(&self) -> u32 {
		self.text.len_bytes() as u32
	}

	pub fn snapshot(&self) -> DocumentSnapshot {
		DocumentSnapshot::new(
			self.snapshot_id,
			self.revision,
			self.generation,
			self.text.clone(),
			self.syntax.clone(),
		)
	}

	pub fn apply_edits(&mut self, edits: &ChangeSet, loader: &impl LanguageLoader) -> Result<UpdateResult, Error> {
		let normalized = normalize_edits(&self.text, edits)?;
		if edits.is_empty() {
			return Ok(UpdateResult {
				revision: self.revision,
				snapshot_id: self.snapshot_id,
				changed_ranges: Vec::new(),
				timed_out: false,
				snapshot_changed: false,
				affected_layers: self.syntax.layer_count(),
			});
		}
		if normalized.is_empty() {
			return Ok(UpdateResult {
				revision: self.revision,
				snapshot_id: self.snapshot_id,
				changed_ranges: Vec::new(),
				timed_out: false,
				snapshot_changed: false,
				affected_layers: self.syntax.layer_count(),
			});
		}

		let mut text = self.text.clone();
		let mut syntax = self.syntax.clone();
		let changed_ranges = coalesce_ranges(normalized.iter().map(invalidated_range).collect());

		for edit in &normalized {
			let input_edit = build_input_edit(&text, edit);
			apply_edit(&mut text, edit)?;
			if let Err(error) = syntax.update(text.slice(..), self.config.parse_timeout, &[input_edit], loader) {
				return match error {
					Error::Timeout => Ok(UpdateResult {
						revision: self.revision,
						snapshot_id: self.snapshot_id,
						changed_ranges,
						timed_out: true,
						snapshot_changed: false,
						affected_layers: self.syntax.layer_count(),
					}),
					other => Err(other),
				};
			}
		}

		self.text = text;
		self.syntax = syntax;
		self.revision = Revision(self.revision.0.wrapping_add(1));
		self.snapshot_id = SnapshotId(self.snapshot_id.0.wrapping_add(1));
		self.generation = self.generation.wrapping_add(1);

		Ok(UpdateResult {
			revision: self.revision,
			snapshot_id: self.snapshot_id,
			changed_ranges,
			timed_out: false,
			snapshot_changed: true,
			affected_layers: self.syntax.layer_count(),
		})
	}
}

fn apply_edit(text: &mut Rope, edit: &TextEdit) -> Result<(), Error> {
	validate_edit(text, edit)?;
	let start_char = text
		.try_byte_to_char(edit.range.start as usize)
		.map_err(|_| Error::InvalidRanges)?;
	let end_char = text
		.try_byte_to_char(edit.range.end as usize)
		.map_err(|_| Error::InvalidRanges)?;
	text.remove(start_char..end_char);
	text.insert(start_char, &edit.replacement);
	Ok(())
}

fn validate_edit(text: &Rope, edit: &TextEdit) -> Result<(), Error> {
	if edit.range.start > edit.range.end || edit.range.end > text.len_bytes() as u32 {
		return Err(Error::InvalidRanges);
	}

	text.try_byte_to_char(edit.range.start as usize)
		.map_err(|_| Error::InvalidRanges)?;
	text.try_byte_to_char(edit.range.end as usize)
		.map_err(|_| Error::InvalidRanges)?;
	Ok(())
}

fn normalize_edits(text: &Rope, edits: &ChangeSet) -> Result<Vec<TextEdit>, Error> {
	let mut normalized = Vec::new();
	for edit in edits.iter() {
		validate_edit(text, edit)?;
		if text.byte_text(edit.range.clone()) != edit.replacement {
			normalized.push(edit.clone());
		}
	}
	Ok(normalized)
}

fn invalidated_range(edit: &TextEdit) -> std::ops::Range<u32> {
	let new_end = edit.range.start + edit.replacement.len() as u32;
	edit.range.start..edit.range.end.max(new_end)
}

fn coalesce_ranges(mut ranges: Vec<std::ops::Range<u32>>) -> Vec<std::ops::Range<u32>> {
	ranges.sort_by_key(|range| range.start);
	let mut merged: Vec<std::ops::Range<u32>> = Vec::with_capacity(ranges.len());
	for range in ranges {
		if let Some(prev) = merged.last_mut()
			&& range.start <= prev.end
		{
			prev.end = prev.end.max(range.end);
		} else {
			merged.push(range);
		}
	}
	merged
}

fn build_input_edit(text: &Rope, edit: &TextEdit) -> InputEdit {
	let start_byte = edit.range.start;
	let old_end_byte = edit.range.end;
	let new_end_byte = start_byte + edit.replacement.len() as u32;

	InputEdit {
		start_byte,
		old_end_byte,
		new_end_byte,
		start_point: point_for_byte(text, start_byte),
		old_end_point: point_for_byte(text, old_end_byte),
		new_end_point: point_after_insert(point_for_byte(text, start_byte), &edit.replacement),
	}
}

fn point_for_byte(text: &Rope, byte: u32) -> Point {
	let line = text.byte_to_line(byte as usize);
	let line_start = text.line_to_byte(line);
	Point {
		row: line as u32,
		col: byte - line_start as u32,
	}
}

fn point_after_insert(start: Point, inserted: &str) -> Point {
	let mut row = start.row;
	let mut col = start.col;
	let mut last_line_start = 0usize;
	let mut newline_count = 0u32;

	for (idx, ch) in inserted.char_indices() {
		if ch == '\n' {
			newline_count += 1;
			last_line_start = idx + ch.len_utf8();
		}
	}

	if newline_count == 0 {
		col += inserted.len() as u32;
	} else {
		row += newline_count;
		col = inserted[last_line_start..].len() as u32;
	}

	Point { row, col }
}

#[cfg(test)]
mod tests {
	use {
		super::*,
		crate::{SingleLanguageLoader, StringText, tree_sitter::Grammar},
	};

	fn rust_session(src: &str) -> DocumentSession {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(crate::Language::new(0), grammar, "", "", "")
			.expect("loader should build");
		DocumentSession::new(
			loader.language(),
			&StringText::new(src),
			&loader,
			EngineConfig::default(),
		)
		.expect("session should parse")
	}

	#[test]
	fn apply_edits_updates_snapshot_text_and_revision() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(crate::Language::new(0), grammar, "", "", "")
			.expect("loader should build");
		let mut session = rust_session("fn alpha() {}\n");

		let result = session
			.apply_edits(&ChangeSet::single(3..8, "beta"), &loader)
			.expect("edit should apply");

		assert_eq!(result.revision, Revision(1));
		assert_eq!(session.snapshot().byte_text(0..14), "fn beta() {}\n");
		assert!(session.snapshot().named_node_at(3, 7).is_some());
	}

	#[test]
	fn oversized_edits_leave_session_unchanged() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(crate::Language::new(0), grammar, "", "", "")
			.expect("loader should build");
		let mut session = rust_session("fn alpha() {}\n");

		let error = session
			.apply_edits(&ChangeSet::single(0..99, ""), &loader)
			.expect_err("edit should fail");
		assert_eq!(error, Error::InvalidRanges);
		assert_eq!(session.snapshot().byte_text(0..14), "fn alpha() {}\n");
	}

	#[test]
	fn noop_edits_do_not_advance_revision() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(crate::Language::new(0), grammar, "", "", "")
			.expect("loader should build");
		let mut session = rust_session("fn alpha() {}\n");

		let result = session
			.apply_edits(&ChangeSet::single(3..8, "alpha"), &loader)
			.expect("edit should apply");

		assert!(!result.snapshot_changed);
		assert_eq!(result.revision, Revision(0));
		assert_eq!(result.snapshot_id, SnapshotId(1));
		assert!(result.changed_ranges.is_empty());
	}

	#[test]
	fn update_ranges_cover_old_and_new_extents() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(crate::Language::new(0), grammar, "", "", "")
			.expect("loader should build");
		let mut session = rust_session("fn alpha() {}\n");

		let result = session
			.apply_edits(&ChangeSet::single(3..8, "beta_gamma"), &loader)
			.expect("edit should apply");

		assert_eq!(result.changed_ranges, vec![3..13]);
	}
}
