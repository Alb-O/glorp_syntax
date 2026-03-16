use {
	glorp_syntax_queries::{RainbowQuery, TagQuery, TextObjectQuery},
	glorp_syntax_tree::{DocumentSession, EngineConfig, SingleLanguageLoader, StringText, tree_sitter::Grammar},
	std::error::Error,
};

const SOURCE: &str = r#"struct User {
    name: String,
}

fn build_user(name: &str) -> User {
    User { name: name.to_owned() }
}

fn greet(user: &User) -> String {
    user.name.clone()
}

fn main() {
    let user = build_user("Ada");
    let greeting = greet(&user);
    println!("{greeting}");
}
"#;

const TEXT_OBJECT_QUERY: &str = r#"
(function_item
  body: (block) @function.inner) @function.outer

(call_expression
  arguments: (arguments) @call.arguments)
"#;

const TAG_QUERY: &str = r#"
(function_item
  name: (identifier) @name) @definition.function
"#;

const RAINBOW_QUERY: &str = r#"
[
  "{"
  "}"
  "("
  ")"
] @rainbow.bracket

[
  (block)
  (parameters)
  (arguments)
] @rainbow.scope
"#;

fn snippet(start: usize, end: usize) -> &'static str {
	&SOURCE[start..end]
}

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

	let text_objects = TextObjectQuery::new(loader.grammar(), TEXT_OBJECT_QUERY)?;
	let functions: Vec<_> = text_objects
		.capture_nodes("function.outer", &snapshot)
		.expect("function.outer capture should exist")
		.map(|node| {
			let range = node.byte_range();
			snippet(range.start, range.end)
				.trim()
				.lines()
				.next()
				.unwrap()
				.to_owned()
		})
		.collect();
	assert_eq!(functions.len(), 3);
	assert!(
		functions
			.iter()
			.any(|line| line == "fn build_user(name: &str) -> User {")
	);
	assert!(functions.iter().any(|line| line == "fn greet(user: &User) -> String {"));
	assert!(functions.iter().any(|line| line == "fn main() {"));

	let call_arguments: Vec<_> = text_objects
		.capture_nodes("call.arguments", &snapshot)
		.expect("call.arguments capture should exist")
		.map(|node| {
			let range = node.byte_range();
			snippet(range.start, range.end).to_owned()
		})
		.collect();
	assert!(call_arguments.iter().any(|args| args == "(\"Ada\")"));
	assert!(call_arguments.iter().any(|args| args == "(&user)"));

	let tag_query = TagQuery::new(loader.grammar(), TAG_QUERY)?;
	let tagged_functions: Vec<_> = tag_query
		.capture_nodes("name", &snapshot)
		.expect("name capture should exist")
		.map(|node| {
			let range = node.byte_range();
			snippet(range.start as usize, range.end as usize).to_owned()
		})
		.collect();
	assert_eq!(
		tagged_functions,
		vec!["build_user".to_owned(), "greet".to_owned(), "main".to_owned()]
	);

	let rainbow_query = RainbowQuery::new(loader.grammar(), RAINBOW_QUERY)?;
	let bracket_count = rainbow_query
		.bracket_nodes(&snapshot)
		.expect("rainbow.bracket capture should exist")
		.count();
	assert!(bracket_count >= 10);

	println!(
		"parsed {} functions, tagged {:?}, and matched {bracket_count} rainbow brackets",
		functions.len(),
		tagged_functions
	);

	Ok(())
}
