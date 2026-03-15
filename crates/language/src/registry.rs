use {
	crate::{
		bundle::{QueryBundle, load_query_bundle},
		grammar::{GrammarError, load_grammar_from_paths},
		query::read_query_from_paths,
	},
	liney_syntax_tree::tree_sitter::Grammar,
	std::{collections::BTreeMap, fmt, path::PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LanguageId(String);

impl LanguageId {
	pub fn new(id: impl Into<String>) -> Self {
		Self(id.into())
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl fmt::Display for LanguageId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0)
	}
}

impl From<&str> for LanguageId {
	fn from(value: &str) -> Self {
		Self::new(value)
	}
}

impl From<String> for LanguageId {
	fn from(value: String) -> Self {
		Self(value)
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageSpec {
	pub id: LanguageId,
	pub grammar_name: String,
	pub grammar_paths: Vec<PathBuf>,
	pub query_roots: Vec<PathBuf>,
}

impl LanguageSpec {
	pub fn new(id: impl Into<LanguageId>, grammar_name: impl Into<String>) -> Self {
		Self {
			id: id.into(),
			grammar_name: grammar_name.into(),
			grammar_paths: Vec::new(),
			query_roots: Vec::new(),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrammarLocator {
	search_paths: Vec<PathBuf>,
}

impl GrammarLocator {
	pub fn new(search_paths: impl IntoIterator<Item = PathBuf>) -> Self {
		Self {
			search_paths: search_paths.into_iter().collect(),
		}
	}

	pub fn search_paths(&self) -> &[PathBuf] {
		&self.search_paths
	}

	pub fn locate(&self, grammar_name: &str) -> Option<PathBuf> {
		crate::grammar::locate_grammar_library(grammar_name, &self.search_paths)
	}

	pub fn load(&self, grammar_name: &str) -> Result<Grammar, GrammarError> {
		load_grammar_from_paths(grammar_name, &self.search_paths)
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryLocator {
	roots: Vec<PathBuf>,
}

impl QueryLocator {
	pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
		Self {
			roots: roots.into_iter().collect(),
		}
	}

	pub fn roots(&self) -> &[PathBuf] {
		&self.roots
	}

	pub fn read_query(&self, language: &LanguageId, filename: &str) -> String {
		read_query_from_paths(language.as_str(), filename, &self.roots)
	}

	pub fn bundle(&self, language: &LanguageId) -> std::io::Result<QueryBundle> {
		load_query_bundle(language.as_str(), &self.roots)
	}
}

#[derive(Debug, Clone, Default)]
pub struct LanguageRegistry {
	specs: BTreeMap<LanguageId, LanguageSpec>,
	default_grammar_locator: GrammarLocator,
	default_query_locator: QueryLocator,
}

impl LanguageRegistry {
	pub fn new(default_grammar_locator: GrammarLocator, default_query_locator: QueryLocator) -> Self {
		Self {
			specs: BTreeMap::new(),
			default_grammar_locator,
			default_query_locator,
		}
	}

	pub fn insert(&mut self, spec: LanguageSpec) {
		self.specs.insert(spec.id.clone(), spec);
	}

	pub fn language(&self, id: &LanguageId) -> Option<&LanguageSpec> {
		self.specs.get(id)
	}

	pub fn load_grammar(&self, id: &LanguageId) -> Result<Grammar, GrammarError> {
		let spec = self
			.language(id)
			.ok_or_else(|| GrammarError::NotFound(id.to_string()))?;
		if spec.grammar_paths.is_empty() {
			self.default_grammar_locator.load(&spec.grammar_name)
		} else {
			load_grammar_from_paths(&spec.grammar_name, &spec.grammar_paths)
		}
	}

	pub fn query_bundle(&self, id: &LanguageId) -> std::io::Result<QueryBundle> {
		let spec = self
			.language(id)
			.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("language not found: {id}")))?;
		if spec.query_roots.is_empty() {
			self.default_query_locator.bundle(id)
		} else {
			load_query_bundle(id.as_str(), &spec.query_roots)
		}
	}
}

#[cfg(test)]
mod tests {
	use {
		super::*,
		std::{
			fs,
			time::{SystemTime, UNIX_EPOCH},
		},
	};

	fn temp_root(name: &str) -> PathBuf {
		let nonce = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.expect("time should be after unix epoch")
			.as_nanos();
		let root = std::env::temp_dir().join(format!("liney-registry-{name}-{nonce}"));
		fs::create_dir_all(&root).expect("temp root should be created");
		root
	}

	#[test]
	fn query_locator_reads_from_explicit_roots_without_env_vars() {
		let root = temp_root("queries");
		let rust_dir = root.join("rust");
		fs::create_dir_all(&rust_dir).expect("rust query dir should exist");
		fs::write(rust_dir.join("highlights.scm"), "(identifier) @variable\n").expect("query should be written");

		let locator = QueryLocator::new([root.clone()]);
		let bundle = locator
			.bundle(&LanguageId::new("rust"))
			.expect("query bundle should load");
		assert_eq!(bundle.get("highlights"), Some("(identifier) @variable\n"));

		fs::remove_dir_all(root).expect("temp root should be removed");
	}
}
