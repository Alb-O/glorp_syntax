use {
	glorp_syntax_queries::TagQuery,
	glorp_syntax_tree::{DocumentSession, EngineConfig, SingleLanguageLoader, StringText, tree_sitter::Grammar},
	std::error::Error,
};

const SOURCE: &str = r#"fn alpha() {}
fn beta() {}
"#;

const TAG_QUERY: &str = r#"
(function_item
  name: (identifier) @name) @definition.function
"#;

fn main() -> Result<(), Box<dyn Error>> {
	let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE)?;
	let loader = SingleLanguageLoader::from_queries(grammar, "", "", "")?;
	let session = DocumentSession::new(
		loader.language(),
		&StringText::new(SOURCE),
		&loader,
		EngineConfig::default(),
	)?;
	let snapshot = session.snapshot();
	let query = TagQuery::new(loader.grammar(), TAG_QUERY)?;
	let tags: Vec<_> = query
		.capture_nodes("name", &snapshot)
		.expect("name capture should exist")
		.map(|node| SOURCE[node.start_byte() as usize..node.end_byte() as usize].to_owned())
		.collect();

	assert_eq!(tags, vec!["alpha".to_owned(), "beta".to_owned()]);
	println!("tags={tags:?}");
	Ok(())
}
