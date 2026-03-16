use {
	crate::{query::QueryReadError, registry::LanguageId},
	std::{
		collections::{BTreeMap, BTreeSet},
		fs,
		path::PathBuf,
	},
	thiserror::Error,
};

/// Collection of named tree-sitter query texts for one language.
///
/// Query kinds are stored by file stem, so `highlights.scm` is exposed as
/// `highlights`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryBundle {
	language: LanguageId,
	queries: BTreeMap<String, String>,
}

impl QueryBundle {
	/// Creates an empty bundle for `language`.
	pub fn new(language: impl Into<LanguageId>) -> Self {
		Self {
			language: language.into(),
			queries: BTreeMap::new(),
		}
	}

	/// Returns the language this bundle belongs to.
	pub fn language(&self) -> &LanguageId {
		&self.language
	}

	/// Returns the query text for `kind`, if present.
	pub fn get(&self, kind: &str) -> Option<&str> {
		self.queries.get(kind).map(String::as_str)
	}

	/// Iterates query kinds and texts in deterministic key order.
	pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
		self.queries.iter().map(|(kind, text)| (kind.as_str(), text.as_str()))
	}

	/// Inserts or replaces one query kind.
	pub fn insert(&mut self, kind: impl Into<String>, text: impl Into<String>) {
		self.queries.insert(kind.into(), text.into());
	}

	/// Extends this bundle, letting `other` override duplicate kinds.
	pub fn merge(&mut self, other: QueryBundle) {
		self.queries.extend(other.queries);
	}

	/// Converts the bundle into its owned query map.
	pub fn into_queries(self) -> BTreeMap<String, String> {
		self.queries
	}
}

/// Errors produced while loading a resolved query bundle.
#[derive(Debug, Error)]
pub enum QueryBundleError {
	/// The requested language was not present in the registry path that invoked bundle loading.
	#[error("language not found: {language}")]
	LanguageNotFound { language: LanguageId },
	/// Reading one query root directory failed while discovering available query kinds.
	#[error("failed to scan query roots for {language}: {path}: {source}")]
	Io {
		language: LanguageId,
		path: PathBuf,
		#[source]
		source: std::io::Error,
	},
	/// A discovered query path did not have a valid UTF-8 file stem.
	#[error("invalid query filename for {language}: {path}")]
	InvalidFilename { language: LanguageId, path: PathBuf },
	/// Resolving one query kind through `read_query_from_paths` failed.
	#[error("failed to resolve query kind {kind} for {language}: {source}")]
	Read {
		language: LanguageId,
		kind: String,
		#[source]
		source: QueryReadError,
	},
}

/// Loads all available query kinds for `language`, resolving `; inherits` directives.
///
/// Query kinds are discovered by scanning every `<root>/<language>/*.scm` file.
/// For each discovered kind, query text is then loaded through
/// [`crate::read_query_from_paths`], so later roots still override earlier ones
/// and inherited queries are expanded before insertion into the bundle.
pub fn load_query_bundle(language: impl Into<LanguageId>, roots: &[PathBuf]) -> Result<QueryBundle, QueryBundleError> {
	let language = language.into();
	let kinds = collect_query_kinds(&language, roots)?;
	let mut bundle = QueryBundle::new(language.clone());

	for kind in kinds {
		// Resolve each discovered kind through the normal query loader so inherit
		// handling and later-root precedence stay identical to one-off reads.
		let filename = format!("{kind}.scm");
		let text = crate::query::read_query_from_paths(language.as_str(), &filename, roots).map_err(|source| {
			QueryBundleError::Read {
				language: language.clone(),
				kind: kind.clone(),
				source,
			}
		})?;
		bundle.insert(kind, text);
	}

	Ok(bundle)
}

/// Loads all available query kinds for `language` without resolving `; inherits`.
///
/// This is the raw on-disk view of query files. Later roots override earlier
/// roots by query kind.
pub fn load_raw_query_bundle(language: impl Into<LanguageId>, roots: &[PathBuf]) -> std::io::Result<QueryBundle> {
	let language = language.into();
	let mut bundle = QueryBundle {
		language,
		queries: BTreeMap::new(),
	};

	for root in roots {
		let lang_dir = root.join(bundle.language.as_str());
		if !lang_dir.is_dir() {
			continue;
		}

		for path in collect_raw_files_sorted(&lang_dir, "scm")? {
			let kind = path
				.file_stem()
				.and_then(|stem| stem.to_str())
				.ok_or_else(|| std::io::Error::other(format!("invalid query filename: {}", path.display())))?;
			bundle.insert(kind, fs::read_to_string(&path)?);
		}
	}

	Ok(bundle)
}

fn collect_query_kinds(language: &LanguageId, roots: &[PathBuf]) -> Result<BTreeSet<String>, QueryBundleError> {
	let mut kinds = BTreeSet::new();
	for root in roots {
		let lang_dir = root.join(language.as_str());
		if !lang_dir.is_dir() {
			continue;
		}
		let read_dir = fs::read_dir(&lang_dir).map_err(|source| QueryBundleError::Io {
			language: language.clone(),
			path: lang_dir.clone(),
			source,
		})?;
		for entry in read_dir {
			let entry = entry.map_err(|source| QueryBundleError::Io {
				language: language.clone(),
				path: lang_dir.clone(),
				source,
			})?;
			let path = entry.path();
			if path.extension().and_then(|ext| ext.to_str()) != Some("scm") {
				continue;
			}
			let kind =
				path.file_stem()
					.and_then(|stem| stem.to_str())
					.ok_or_else(|| QueryBundleError::InvalidFilename {
						language: language.clone(),
						path: path.clone(),
					})?;
			kinds.insert(kind.to_owned());
		}
	}
	Ok(kinds)
}

fn collect_raw_files_sorted(dir: &PathBuf, extension: &str) -> std::io::Result<Vec<PathBuf>> {
	let mut files = Vec::new();
	for entry in fs::read_dir(dir)? {
		let entry = entry?;
		let path = entry.path();
		if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
			files.push(path);
		}
	}
	files.sort();
	Ok(files)
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
		let root = std::env::temp_dir().join(format!("glorp_syntax-bundle-{name}-{nonce}"));
		fs::create_dir_all(&root).expect("temp root should be created");
		root
	}

	#[test]
	fn later_roots_override_earlier_query_files() {
		let root_a = temp_root("merge-a");
		let root_b = temp_root("merge-b");
		fs::create_dir_all(root_a.join("rust")).expect("root a rust dir should exist");
		fs::create_dir_all(root_b.join("rust")).expect("root b rust dir should exist");

		fs::write(root_a.join("rust").join("highlights.scm"), "(a) @variable\n")
			.expect("root a query should be written");
		fs::write(root_a.join("rust").join("locals.scm"), "(a) @local.scope\n")
			.expect("root a locals should be written");
		fs::write(root_b.join("rust").join("highlights.scm"), "(b) @type\n").expect("root b query should be written");

		let merged = load_raw_query_bundle("rust", &[root_a.clone(), root_b.clone()]).expect("queries should merge");
		assert_eq!(merged.get("highlights"), Some("(b) @type\n"));
		assert_eq!(merged.get("locals"), Some("(a) @local.scope\n"));

		fs::remove_dir_all(root_a).expect("root a should be removed");
		fs::remove_dir_all(root_b).expect("root b should be removed");
	}

	#[test]
	fn resolved_bundle_expands_inherits() {
		let root = temp_root("resolved");
		fs::create_dir_all(root.join("base")).expect("base dir should exist");
		fs::create_dir_all(root.join("rust")).expect("rust dir should exist");
		fs::write(root.join("base").join("highlights.scm"), "(identifier) @variable\n")
			.expect("base query should be written");
		fs::write(
			root.join("rust").join("highlights.scm"),
			"; inherits base\n(type_identifier) @type\n",
		)
		.expect("rust query should be written");

		let bundle = load_query_bundle("rust", std::slice::from_ref(&root)).expect("bundle should resolve");
		assert_eq!(
			bundle.get("highlights"),
			Some("\n(identifier) @variable\n\n(type_identifier) @type\n")
		);

		fs::remove_dir_all(root).expect("root should be removed");
	}
}
