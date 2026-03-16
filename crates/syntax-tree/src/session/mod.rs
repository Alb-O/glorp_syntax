use {
	crate::{
		DocumentSnapshot, Error, Language, LanguageLoader, Syntax,
		change::{ChangeSet, Revision, SnapshotId, TextEdit, UpdateResult},
		text::{DocumentText, TextStorage},
		tree_sitter::{InputEdit, Point},
	},
	ropey::Rope,
	std::{sync::Arc, time::Duration},
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
	text: Arc<Rope>,
	syntax: Arc<Syntax>,
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
			text: Arc::new(rope),
			syntax: Arc::new(syntax),
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

	pub fn set_config(&mut self, config: EngineConfig) {
		self.config = config;
	}

	pub fn text(&self) -> DocumentText<'_> {
		DocumentText::new(self.text.as_ref().slice(..))
	}

	pub fn len_bytes(&self) -> u32 {
		self.text.len_bytes() as u32
	}

	pub fn snapshot(&self) -> DocumentSnapshot {
		DocumentSnapshot::new(
			self.snapshot_id,
			self.revision,
			self.generation,
			Arc::clone(&self.text),
			Arc::clone(&self.syntax),
		)
	}

	pub fn apply_edits(&mut self, edits: &ChangeSet, loader: &impl LanguageLoader) -> Result<UpdateResult, Error> {
		if edits.is_empty() {
			return Ok(self.unchanged_result());
		}
		let original_text = self.text.as_ref();
		let normalized = normalize_edits(original_text, edits)?;
		if normalized.is_empty() {
			return Ok(self.unchanged_result());
		}

		let input_edits: Vec<_> = normalized
			.iter()
			.map(|edit| build_input_edit(original_text, edit))
			.collect();
		let mut text = original_text.clone();
		let mut syntax = self.syntax.as_ref().clone();
		let changed_ranges = coalesce_sorted_ranges(normalized.iter().map(invalidated_range));

		for edit in normalized.iter().rev() {
			apply_edit(&mut text, edit)?;
		}
		if let Err(error) = syntax.update(text.slice(..), self.config.parse_timeout, &input_edits, loader) {
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

		self.text = Arc::new(text);
		self.syntax = Arc::new(syntax);
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

	fn unchanged_result(&self) -> UpdateResult {
		UpdateResult {
			revision: self.revision,
			snapshot_id: self.snapshot_id,
			changed_ranges: Vec::new(),
			timed_out: false,
			snapshot_changed: false,
			affected_layers: self.syntax.layer_count(),
		}
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
	let mut normalized = Vec::with_capacity(edits.iter().size_hint().0);
	for edit in edits.iter() {
		validate_edit(text, edit)?;
		if !byte_range_eq(text, edit.range.clone(), &edit.replacement) {
			normalized.push(edit.clone());
		}
	}
	normalized.sort_by_key(|edit| edit.range.start);
	if normalized
		.windows(2)
		.any(|pair| pair[0].range.end > pair[1].range.start)
	{
		return Err(Error::InvalidRanges);
	}
	Ok(normalized)
}

fn byte_range_eq(text: &Rope, range: std::ops::Range<u32>, expected: &str) -> bool {
	let slice = text.byte_slice(range.start as usize..range.end as usize);
	slice.len_bytes() == expected.len()
		&& slice
			.chunks()
			.try_fold(expected, |remaining, chunk| remaining.strip_prefix(chunk))
			.is_some_and(str::is_empty)
}

fn invalidated_range(edit: &TextEdit) -> std::ops::Range<u32> {
	let new_end = edit.range.start + edit.replacement.len() as u32;
	edit.range.start..edit.range.end.max(new_end)
}

fn coalesce_sorted_ranges(ranges: impl IntoIterator<Item = std::ops::Range<u32>>) -> Vec<std::ops::Range<u32>> {
	let ranges = ranges.into_iter();
	let mut merged: Vec<std::ops::Range<u32>> = Vec::with_capacity(ranges.size_hint().0);
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
	let start_point = point_for_byte(text, start_byte);

	InputEdit {
		start_byte,
		old_end_byte,
		new_end_byte,
		start_point,
		old_end_point: point_for_byte(text, old_end_byte),
		new_end_point: point_after_insert(start_point, &edit.replacement),
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
		std::fmt::Write,
	};

	fn rust_session(src: &str) -> DocumentSession {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
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
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
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
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
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
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
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
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
		let mut session = rust_session("fn alpha() {}\n");

		let result = session
			.apply_edits(&ChangeSet::single(3..8, "beta_gamma"), &loader)
			.expect("edit should apply");

		assert_eq!(result.changed_ranges, vec![3..13]);
	}

	#[test]
	fn multiple_non_overlapping_edits_parse_once_against_final_text() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
		let mut session = rust_session("fn alpha() {\n    beta();\n}\n");
		let edits = ChangeSet::new([TextEdit::new(3..8, "gamma"), TextEdit::new(17..21, "delta")]);

		let result = session.apply_edits(&edits, &loader).expect("edits should apply");

		assert!(result.snapshot_changed);
		assert_eq!(result.revision, Revision(1));
		assert_eq!(session.snapshot().byte_text(0..28), "fn gamma() {\n    delta();\n}\n");
		assert!(session.snapshot().named_node_at(3, 8).is_some());
		assert!(session.snapshot().named_node_at(17, 22).is_some());
	}

	#[test]
	fn overlapping_edits_are_rejected() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
		let mut session = rust_session("fn alpha() {}\n");
		let edits = ChangeSet::new([TextEdit::new(3..8, "beta"), TextEdit::new(5..8, "amma")]);

		let error = session
			.apply_edits(&edits, &loader)
			.expect_err("overlapping edits should fail");

		assert_eq!(error, Error::InvalidRanges);
		assert_eq!(session.snapshot().byte_text(0..14), "fn alpha() {}\n");
	}

	#[test]
	fn timeout_returns_without_mutating_session() {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
		let mut source = String::new();
		for idx in 0..20_000 {
			writeln!(&mut source, "fn value_{idx}() -> i32 {{ {idx} }}").expect("source should build");
		}
		let mut session = DocumentSession::new(
			loader.language(),
			&StringText::new(source.clone()),
			&loader,
			EngineConfig::default(),
		)
		.expect("session should parse");
		session.config.parse_timeout = Duration::from_micros(1);
		let before = session.snapshot();

		let result = session
			.apply_edits(&ChangeSet::single(3..8, "updated"), &loader)
			.expect("timeout should be reported as an update result");

		assert!(result.timed_out);
		assert!(!result.snapshot_changed);
		assert_eq!(result.revision, before.revision());
		assert_eq!(result.snapshot_id, before.id());
		assert_eq!(session.snapshot().byte_text(0..15), before.byte_text(0..15));
	}
}
