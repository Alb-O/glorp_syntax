use {
	glorp_syntax_language::{
		GrammarLocator, LanguageId, LanguageRegistry, LanguageSpec, QueryLocator, RegistryLanguageLoader,
		grammar_search_paths,
	},
	glorp_syntax_tree::{DocumentSession, EngineConfig, StringText},
	std::{
		error::Error,
		fs,
		time::{SystemTime, UNIX_EPOCH},
	},
};

fn temp_root(name: &str) -> std::path::PathBuf {
	let nonce = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.expect("time should be after unix epoch")
		.as_nanos();
	let root = std::env::temp_dir().join(format!("glorp_syntax-runtime-example-{name}-{nonce}"));
	fs::create_dir_all(&root).expect("temp root should be created");
	root
}

fn main() -> Result<(), Box<dyn Error>> {
	let query_root = temp_root("queries");
	fs::create_dir_all(query_root.join("rust"))?;
	fs::write(
		query_root.join("rust").join("highlights.scm"),
		"(identifier) @variable\n",
	)?;

	let mut registry = LanguageRegistry::new(
		GrammarLocator::new(grammar_search_paths()),
		QueryLocator::new([query_root.clone()]),
	);
	let mut rust = LanguageSpec::new(LanguageId::new("rust"), "tree-sitter-rust");
	rust.injection_names.push("rs".to_owned());
	registry.insert(rust)?;

	let loader = RegistryLanguageLoader::from_registry(&registry)?;
	let language = loader
		.language(&LanguageId::new("rust"))
		.expect("registered language should map to a numeric id");
	let session = DocumentSession::new(
		language,
		&StringText::new("fn answer() -> i32 { 42 }\n"),
		&loader,
		EngineConfig::default(),
	)?;
	let snapshot = session.snapshot();
	assert!(snapshot.named_node_at(3, 9).is_some());

	println!(
		"loaded language {:?} revision={}",
		loader.language_id(language),
		snapshot.revision().0
	);
	fs::remove_dir_all(query_root)?;
	Ok(())
}
