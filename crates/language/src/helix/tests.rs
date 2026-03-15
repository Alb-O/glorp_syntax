use super::*;

fn temp_root(name: &str) -> PathBuf {
	let nonce = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.expect("time should be after unix epoch")
		.as_nanos();
	let root = std::env::temp_dir().join(format!("glorp_syntax-helix-{name}-{nonce}"));
	fs::create_dir_all(&root).expect("temp root should be created");
	root
}

#[test]
fn later_roots_override_earlier_query_files() {
	let root_a = temp_root("merge-a");
	let root_b = temp_root("merge-b");
	fs::create_dir_all(root_a.join("rust")).expect("root a rust dir should exist");
	fs::create_dir_all(root_b.join("rust")).expect("root b rust dir should exist");

	fs::write(root_a.join("rust").join("highlights.scm"), "(a) @variable\n").expect("root a query should be written");
	fs::write(root_a.join("rust").join("locals.scm"), "(a) @local.scope\n").expect("root a locals should be written");
	fs::write(root_b.join("rust").join("highlights.scm"), "(b) @type\n").expect("root b query should be written");

	let merged = merge_language_queries("rust", &[root_a.clone(), root_b.clone()]).expect("queries should merge");
	assert_eq!(merged.get("highlights").map(String::as_str), Some("(b) @type\n"));
	assert_eq!(merged.get("locals").map(String::as_str), Some("(a) @local.scope\n"));

	fs::remove_dir_all(root_a).expect("root a should be removed");
	fs::remove_dir_all(root_b).expect("root b should be removed");
}
