use {
	glorp_syntax_tree::{
		ChangeSet, DocumentSession, EngineConfig, SingleLanguageLoader, StringText, tree_sitter::Grammar,
	},
	std::error::Error,
};

fn main() -> Result<(), Box<dyn Error>> {
	let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE)?;
	let loader = SingleLanguageLoader::from_queries(grammar, "", "", "")?;
	let mut session = DocumentSession::new(
		loader.language(),
		&StringText::new("fn alpha() -> i32 { 1 }\n"),
		&loader,
		EngineConfig::default(),
	)?;

	let update = session.apply_edits(&ChangeSet::single(3..8, "beta"), &loader)?;
	let snapshot = session.snapshot();

	assert_eq!(update.revision.0, 1);
	assert_eq!(snapshot.byte_text(0..snapshot.len_bytes()), "fn beta() -> i32 { 1 }\n");
	assert!(snapshot.named_node_at(3, 7).is_some());

	println!(
		"revision={} generation={} bytes={}",
		snapshot.revision().0,
		snapshot.generation(),
		snapshot.len_bytes()
	);
	Ok(())
}
