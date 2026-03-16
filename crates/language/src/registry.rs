use {
	crate::{
		bundle::{QueryBundle, QueryBundleError, load_query_bundle, load_raw_query_bundle},
		grammar::{GrammarError, load_grammar_from_paths},
		query::{QueryReadError, read_optional_query_from_paths, read_query_from_paths},
	},
	glorp_syntax_tree::tree_sitter::Grammar,
	std::{collections::BTreeMap, fmt, path::PathBuf},
};

/// Error returned when inserting a language whose [`LanguageId`] is already present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateLanguageIdError {
	pub id: LanguageId,
}

impl fmt::Display for DuplicateLanguageIdError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "duplicate language id: {}", self.id)
	}
}

impl std::error::Error for DuplicateLanguageIdError {}

/// Stable string identifier used by runtime registries and query bundles.
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

/// Runtime metadata for one language entry in a [`LanguageRegistry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageSpec {
	pub id: LanguageId,
	pub grammar_name: String,
	pub grammar_paths: Vec<PathBuf>,
	pub query_roots: Vec<PathBuf>,
	/// Exact names accepted by `(#set! injection.language "...")`.
	pub injection_names: Vec<String>,
	/// Regex matchers used for document-content based injection markers.
	pub content_regexes: Vec<String>,
	/// Regex matchers used for filename-based injection markers.
	pub filename_regexes: Vec<String>,
	/// Regex matchers used for shebang-based injection markers.
	pub shebang_regexes: Vec<String>,
}

impl LanguageSpec {
	pub fn new(id: impl Into<LanguageId>, grammar_name: impl Into<String>) -> Self {
		Self {
			id: id.into(),
			grammar_name: grammar_name.into(),
			grammar_paths: Vec::new(),
			query_roots: Vec::new(),
			injection_names: Vec::new(),
			content_regexes: Vec::new(),
			filename_regexes: Vec::new(),
			shebang_regexes: Vec::new(),
		}
	}
}

/// Grammar search helper shared by runtime registries.
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

/// Query-root search helper shared by runtime registries.
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

	pub fn read_query(&self, language: &LanguageId, filename: &str) -> Result<String, QueryReadError> {
		read_query_from_paths(language.as_str(), filename, &self.roots)
	}

	pub fn read_optional_query(&self, language: &LanguageId, filename: &str) -> Result<Option<String>, QueryReadError> {
		read_optional_query_from_paths(language.as_str(), filename, &self.roots)
	}

	/// Loads the resolved query bundle for `language`.
	///
	/// This is equivalent to calling [`load_query_bundle`](crate::load_query_bundle)
	/// with this locator's roots.
	pub fn bundle(&self, language: &LanguageId) -> Result<QueryBundle, QueryBundleError> {
		load_query_bundle(language.as_str(), &self.roots)
	}

	/// Loads the raw on-disk query bundle for `language` without resolving `; inherits`.
	pub fn raw_bundle(&self, language: &LanguageId) -> std::io::Result<QueryBundle> {
		load_raw_query_bundle(language.as_str(), &self.roots)
	}
}

/// Registry of language runtime metadata, grammar lookup, and query roots.
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

	/// Inserts `spec` if its [`LanguageId`] is not already present.
	///
	/// Returns [`DuplicateLanguageIdError`] instead of silently replacing an
	/// existing language entry.
	pub fn insert(&mut self, spec: LanguageSpec) -> Result<(), DuplicateLanguageIdError> {
		if self.specs.contains_key(&spec.id) {
			return Err(DuplicateLanguageIdError { id: spec.id });
		}
		self.specs.insert(spec.id.clone(), spec);
		Ok(())
	}

	/// Replaces the entry with the same [`LanguageId`], returning the previous spec if any.
	pub fn replace(&mut self, spec: LanguageSpec) -> Option<LanguageSpec> {
		self.specs.insert(spec.id.clone(), spec)
	}

	/// Returns the registered spec for `id`.
	pub fn language(&self, id: &LanguageId) -> Option<&LanguageSpec> {
		self.specs.get(id)
	}

	/// Iterates registry entries in deterministic key order.
	pub fn iter(&self) -> impl Iterator<Item = (&LanguageId, &LanguageSpec)> {
		self.specs.iter()
	}

	/// Loads the compiled grammar for `id`.
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

	/// Loads and merges the available resolved query files for `id`.
	///
	/// Query text is returned after `; inherits` expansion.
	pub fn query_bundle(&self, id: &LanguageId) -> Result<QueryBundle, QueryBundleError> {
		let spec = self
			.language(id)
			.ok_or_else(|| QueryBundleError::LanguageNotFound { language: id.clone() })?;
		if spec.query_roots.is_empty() {
			self.default_query_locator.bundle(id)
		} else {
			load_query_bundle(id.as_str(), &spec.query_roots)
		}
	}

	/// Loads and merges the available raw query files for `id` without resolving `; inherits`.
	///
	/// Prefer [`Self::query_bundle`] unless the caller explicitly needs the
	/// on-disk query text.
	pub fn raw_query_bundle(&self, id: &LanguageId) -> std::io::Result<QueryBundle> {
		let spec = self
			.language(id)
			.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("language not found: {id}")))?;
		if spec.query_roots.is_empty() {
			self.default_query_locator.raw_bundle(id)
		} else {
			load_raw_query_bundle(id.as_str(), &spec.query_roots)
		}
	}

	/// Reads `filename` for `id`, expanding `; inherits` directives.
	pub fn read_query(&self, id: &LanguageId, filename: &str) -> Result<String, QueryReadError> {
		let spec = self.language(id).ok_or_else(|| QueryReadError::RootNotFound {
			language: id.to_string(),
			filename: filename.to_owned(),
		})?;
		if spec.query_roots.is_empty() {
			self.default_query_locator.read_query(id, filename)
		} else {
			read_query_from_paths(id.as_str(), filename, &spec.query_roots)
		}
	}

	/// Reads `filename` for `id` if it exists, expanding `; inherits` directives.
	pub fn read_optional_query(&self, id: &LanguageId, filename: &str) -> Result<Option<String>, QueryReadError> {
		let spec = self.language(id).ok_or_else(|| QueryReadError::RootNotFound {
			language: id.to_string(),
			filename: filename.to_owned(),
		})?;
		if spec.query_roots.is_empty() {
			self.default_query_locator.read_optional_query(id, filename)
		} else {
			read_optional_query_from_paths(id.as_str(), filename, &spec.query_roots)
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
		let root = std::env::temp_dir().join(format!("glorp_syntax-registry-{name}-{nonce}"));
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

	#[test]
	fn insert_rejects_duplicate_language_ids() {
		let mut registry = LanguageRegistry::new(GrammarLocator::default(), QueryLocator::default());
		registry
			.insert(LanguageSpec::new("rust", "tree-sitter-rust"))
			.expect("first insert should succeed");
		let error = registry
			.insert(LanguageSpec::new("rust", "tree-sitter-rust"))
			.expect_err("duplicate insert should fail");

		assert_eq!(error.id, LanguageId::new("rust"));
	}

	#[test]
	fn replace_swaps_existing_language_spec() {
		let mut registry = LanguageRegistry::new(GrammarLocator::default(), QueryLocator::default());
		registry
			.insert(LanguageSpec::new("rust", "tree-sitter-rust"))
			.expect("first insert should succeed");

		let replaced = registry.replace(LanguageSpec::new("rust", "tree-sitter-rust-alt"));

		assert_eq!(
			replaced.map(|spec| spec.grammar_name),
			Some("tree-sitter-rust".to_owned())
		);
		assert_eq!(
			registry
				.language(&LanguageId::new("rust"))
				.map(|spec| spec.grammar_name.as_str()),
			Some("tree-sitter-rust-alt")
		);
	}

	#[test]
	fn raw_bundle_preserves_unresolved_query_text() {
		let root = temp_root("raw-queries");
		let base_dir = root.join("base");
		let rust_dir = root.join("rust");
		fs::create_dir_all(&base_dir).expect("base dir should exist");
		fs::create_dir_all(&rust_dir).expect("rust dir should exist");
		fs::write(base_dir.join("highlights.scm"), "(identifier) @variable\n").expect("base query should be written");
		fs::write(
			rust_dir.join("highlights.scm"),
			"; inherits base\n(type_identifier) @type\n",
		)
		.expect("rust query should be written");

		let locator = QueryLocator::new([root.clone()]);
		let bundle = locator
			.raw_bundle(&LanguageId::new("rust"))
			.expect("raw query bundle should load");
		assert_eq!(
			bundle.get("highlights"),
			Some("; inherits base\n(type_identifier) @type\n")
		);

		fs::remove_dir_all(root).expect("root should be removed");
	}
}
