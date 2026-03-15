use {
	glorp_syntax_language::{GrammarLocator, LanguageId, LanguageRegistry, LanguageSpec, QueryLocator},
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

	let mut registry = LanguageRegistry::new(GrammarLocator::default(), QueryLocator::new([query_root.clone()]));
	registry.insert(LanguageSpec::new(LanguageId::new("rust"), "tree-sitter-rust"));

	let bundle = registry.query_bundle(&LanguageId::new("rust"))?;
	assert_eq!(bundle.get("highlights"), Some("(identifier) @variable\n"));

	println!("loaded query kinds={}", bundle.iter().count());
	fs::remove_dir_all(query_root)?;
	Ok(())
}
