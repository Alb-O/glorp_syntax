use {
	glorp_syntax_tree::{ReadQueryError as ResolveQueryError, read_query as resolve_inherits},
	std::path::PathBuf,
	thiserror::Error,
};

/// Errors returned while reading a single query file and expanding `; inherits`.
#[derive(Debug, Error)]
pub enum QueryReadError {
	#[error("query file not found: {language}/{filename}")]
	RootNotFound { language: String, filename: String },
	#[error("inherited query file not found: {language}/{filename}")]
	InheritedNotFound { language: String, filename: String },
	#[error("failed to read query file {path}: {source}")]
	Io {
		path: PathBuf,
		#[source]
		source: std::io::Error,
	},
	#[error("cyclic query inherits: {chain:?}")]
	InheritCycle { chain: Vec<String> },
}

/// Reads a query from the default runtime/query search paths and resolves
/// `; inherits` directives recursively.
#[cfg(feature = "default-runtime-paths")]
pub fn read_query(lang: &str, filename: &str) -> Result<String, QueryReadError> {
	read_query_from_paths(lang, filename, &crate::runtime_paths::query_search_paths())
}

/// Reads a query from the supplied query roots and resolves `; inherits`
/// directives recursively.
pub fn read_query_from_paths(lang: &str, filename: &str, roots: &[PathBuf]) -> Result<String, QueryReadError> {
	read_optional_query_from_paths(lang, filename, roots)?.ok_or_else(|| QueryReadError::RootNotFound {
		language: lang.to_owned(),
		filename: filename.to_owned(),
	})
}

/// Reads a query if present, returning `Ok(None)` when the root file does not exist.
pub fn read_optional_query_from_paths(
	lang: &str, filename: &str, roots: &[PathBuf],
) -> Result<Option<String>, QueryReadError> {
	match resolve_inherits(lang, |query_lang| read_query_text(roots, query_lang, filename)) {
		Ok(query) => Ok(Some(query)),
		// A missing root file means "this optional query kind is absent", but the same
		// condition for an inherited file is a real configuration error.
		Err(ResolveQueryError::Read {
			language,
			source: QueryFileError::NotFound { .. },
		}) if language.as_ref() == lang => Ok(None),
		Err(ResolveQueryError::Read { source, .. }) => Err(source.into_query_error(lang)),
		Err(ResolveQueryError::Cycle { chain }) => Err(QueryReadError::InheritCycle {
			chain: chain.into_iter().map(Into::into).collect(),
		}),
	}
}

#[derive(Debug)]
enum QueryFileError {
	NotFound { language: String, filename: String },
	Io { path: PathBuf, source: std::io::Error },
}

impl QueryFileError {
	fn into_query_error(self, root_language: &str) -> QueryReadError {
		match self {
			Self::NotFound { language, filename } if language == root_language => {
				QueryReadError::RootNotFound { language, filename }
			}
			Self::NotFound { language, filename } => QueryReadError::InheritedNotFound { language, filename },
			Self::Io { path, source } => QueryReadError::Io { path, source },
		}
	}
}

impl std::fmt::Display for QueryFileError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotFound { language, filename } => write!(f, "query file not found: {language}/{filename}"),
			Self::Io { path, source } => write!(f, "failed to read {}: {source}", path.display()),
		}
	}
}

impl std::error::Error for QueryFileError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::NotFound { .. } => None,
			Self::Io { source, .. } => Some(source),
		}
	}
}

fn read_query_text(roots: &[PathBuf], query_lang: &str, filename: &str) -> Result<String, QueryFileError> {
	let Some(path) = roots
		.iter()
		.rev()
		// Later roots override earlier ones, matching bundle loading precedence.
		.find_map(|root| {
			// Build the candidate path only once so the existence check and returned value stay in sync.
			let path = root.join(query_lang).join(filename);
			path.exists().then_some(path)
		})
	else {
		return Err(QueryFileError::NotFound {
			language: query_lang.to_owned(),
			filename: filename.to_owned(),
		});
	};

	std::fs::read_to_string(&path).map_err(|source| QueryFileError::Io { path, source })
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
		let root = std::env::temp_dir().join(format!("glorp_syntax_language-{name}-{nonce}"));
		fs::create_dir_all(&root).expect("temp root should be created");
		root
	}

	#[test]
	fn read_query_resolves_inherited_language_queries() {
		let root = temp_root("inherits");
		let base_dir = root.join("base");
		let rust_dir = root.join("rust");
		fs::create_dir_all(&base_dir).expect("base query dir should exist");
		fs::create_dir_all(&rust_dir).expect("rust query dir should exist");

		fs::write(base_dir.join("highlights.scm"), "(identifier) @variable\n").expect("base query should be written");
		fs::write(
			rust_dir.join("highlights.scm"),
			"; inherits base\n(type_identifier) @type\n",
		)
		.expect("rust query should be written");

		let query =
			read_query_from_paths("rust", "highlights.scm", std::slice::from_ref(&root)).expect("query should resolve");
		assert!(query.contains("(identifier) @variable"));
		assert!(query.contains("(type_identifier) @type"));

		fs::remove_dir_all(root).expect("temp root should be removed");
	}

	#[test]
	fn later_roots_override_earlier_query_files() {
		let root_a = temp_root("override-a");
		let root_b = temp_root("override-b");
		fs::create_dir_all(root_a.join("rust")).expect("root a rust query dir should exist");
		fs::create_dir_all(root_b.join("rust")).expect("root b rust query dir should exist");
		fs::write(root_a.join("rust").join("highlights.scm"), "(identifier) @variable\n")
			.expect("root a query should be written");
		fs::write(root_b.join("rust").join("highlights.scm"), "(type_identifier) @type\n")
			.expect("root b query should be written");

		let query = read_query_from_paths("rust", "highlights.scm", &[root_a.clone(), root_b.clone()])
			.expect("query should resolve");
		assert_eq!(query, "(type_identifier) @type\n");

		fs::remove_dir_all(root_a).expect("root a should be removed");
		fs::remove_dir_all(root_b).expect("root b should be removed");
	}

	#[test]
	fn read_query_errors_on_missing_inherited_query() {
		let root = temp_root("missing-inherit");
		let rust_dir = root.join("rust");
		fs::create_dir_all(&rust_dir).expect("rust query dir should exist");
		fs::write(
			rust_dir.join("highlights.scm"),
			"; inherits base\n(type_identifier) @type\n",
		)
		.expect("rust query should be written");

		let error = read_query_from_paths("rust", "highlights.scm", std::slice::from_ref(&root))
			.expect_err("missing inherited query should error");
		assert!(matches!(
			error,
			QueryReadError::InheritedNotFound {
				ref language,
				ref filename,
			} if language == "base" && filename == "highlights.scm"
		));

		fs::remove_dir_all(root).expect("root should be removed");
	}

	#[test]
	fn read_query_errors_on_missing_root_query() {
		let root = temp_root("missing-root");
		fs::create_dir_all(root.join("rust")).expect("rust query dir should exist");

		let error = read_query_from_paths("rust", "highlights.scm", std::slice::from_ref(&root))
			.expect_err("missing root query should error");
		assert!(matches!(
			error,
			QueryReadError::RootNotFound {
				ref language,
				ref filename,
			} if language == "rust" && filename == "highlights.scm"
		));

		fs::remove_dir_all(root).expect("root should be removed");
	}

	#[test]
	fn read_query_errors_on_inherit_cycle() {
		let root = temp_root("cycle");
		let rust_dir = root.join("rust");
		let base_dir = root.join("base");
		fs::create_dir_all(&rust_dir).expect("rust query dir should exist");
		fs::create_dir_all(&base_dir).expect("base query dir should exist");
		fs::write(
			rust_dir.join("highlights.scm"),
			"; inherits base\n(type_identifier) @type\n",
		)
		.expect("rust query should be written");
		fs::write(
			base_dir.join("highlights.scm"),
			"; inherits rust\n(identifier) @variable\n",
		)
		.expect("base query should be written");

		let error = read_query_from_paths("rust", "highlights.scm", std::slice::from_ref(&root))
			.expect_err("inherit cycle should error");
		assert!(matches!(
			error,
			QueryReadError::InheritCycle { ref chain } if chain == &["rust".to_owned(), "base".to_owned(), "rust".to_owned()]
		));

		fs::remove_dir_all(root).expect("root should be removed");
	}
}
