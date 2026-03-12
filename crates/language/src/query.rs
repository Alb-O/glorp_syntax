use {liney_syntax_tree::read_query as resolve_inherits, std::path::PathBuf};

/// Reads a query from the default runtime/query search paths and resolves
/// `; inherits` directives recursively.
#[cfg(feature = "default-runtime-paths")]
pub fn read_query(lang: &str, filename: &str) -> String {
	let roots = crate::runtime_paths::query_search_paths();
	read_query_from_paths(lang, filename, &roots)
}

/// Reads a query from the supplied query roots and resolves `; inherits`
/// directives recursively.
pub fn read_query_from_paths(lang: &str, filename: &str, roots: &[PathBuf]) -> String {
	resolve_inherits(lang, |query_lang| read_query_text(roots, query_lang, filename))
}

fn read_query_text(roots: &[PathBuf], query_lang: &str, filename: &str) -> String {
	for root in roots {
		let path = root.join(query_lang).join(filename);
		if let Ok(content) = std::fs::read_to_string(&path) {
			return content;
		}
	}
	String::new()
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
		let root = std::env::temp_dir().join(format!("liney-language-{name}-{nonce}"));
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

		let query = read_query_from_paths("rust", "highlights.scm", std::slice::from_ref(&root));
		assert!(query.contains("(identifier) @variable"));
		assert!(query.contains("(type_identifier) @type"));

		fs::remove_dir_all(root).expect("temp root should be removed");
	}
}
