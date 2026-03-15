use {
	crate::registry::LanguageId,
	std::{
		collections::BTreeMap,
		fs,
		path::{Path, PathBuf},
	},
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryBundle {
	language: LanguageId,
	queries: BTreeMap<String, String>,
}

impl QueryBundle {
	pub fn new(language: impl Into<LanguageId>) -> Self {
		Self {
			language: language.into(),
			queries: BTreeMap::new(),
		}
	}

	pub fn language(&self) -> &LanguageId {
		&self.language
	}

	pub fn get(&self, kind: &str) -> Option<&str> {
		self.queries.get(kind).map(String::as_str)
	}

	pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
		self.queries.iter().map(|(kind, text)| (kind.as_str(), text.as_str()))
	}

	pub fn insert(&mut self, kind: impl Into<String>, text: impl Into<String>) {
		self.queries.insert(kind.into(), text.into());
	}

	pub fn merge(&mut self, other: QueryBundle) {
		self.queries.extend(other.queries);
	}

	pub fn into_queries(self) -> BTreeMap<String, String> {
		self.queries
	}
}

pub fn load_query_bundle(language: impl Into<LanguageId>, roots: &[PathBuf]) -> std::io::Result<QueryBundle> {
	let language = language.into();
	let mut bundle = QueryBundle::new(language.clone());

	for root in roots {
		let lang_dir = root.join(language.as_str());
		if !lang_dir.is_dir() {
			continue;
		}

		for path in collect_files_sorted(&lang_dir, "scm")? {
			let kind = path
				.file_stem()
				.and_then(|stem| stem.to_str())
				.ok_or_else(|| std::io::Error::other(format!("invalid query filename: {}", path.display())))?;
			bundle.insert(kind, fs::read_to_string(&path)?);
		}
	}

	Ok(bundle)
}

fn collect_files_sorted(dir: &Path, extension: &str) -> std::io::Result<Vec<PathBuf>> {
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
		let root = std::env::temp_dir().join(format!("liney-bundle-{name}-{nonce}"));
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

		let merged = load_query_bundle("rust", &[root_a.clone(), root_b.clone()]).expect("queries should merge");
		assert_eq!(merged.get("highlights"), Some("(b) @type\n"));
		assert_eq!(merged.get("locals"), Some("(a) @local.scope\n"));

		fs::remove_dir_all(root_a).expect("root a should be removed");
		fs::remove_dir_all(root_b).expect("root b should be removed");
	}
}
