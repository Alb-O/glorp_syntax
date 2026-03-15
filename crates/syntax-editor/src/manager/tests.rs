use {
	super::*,
	crate::{Language, SingleLanguageLoader, Syntax, SyntaxOptions, tree_sitter::Grammar},
	ropey::Rope,
};

fn rust_syntax(src: &str) -> Syntax {
	let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
	let loader =
		SingleLanguageLoader::from_queries(Language::new(0), grammar, "", "", "").expect("loader should build");
	let rope = Rope::from_str(src);
	Syntax::new(rope.slice(..), loader.language(), &loader, SyntaxOptions::default()).expect("syntax should parse")
}

#[test]
fn candidate_score_prefers_exact_full_then_enriched() {
	let exact_full = candidate_score(4, true, false, 4);
	let exact_viewport_enriched = candidate_score(4, false, true, 4);
	let stale_full = candidate_score(3, true, false, 4);

	assert!(exact_full > exact_viewport_enriched);
	assert!(exact_viewport_enriched > stale_full);
}

#[test]
fn syntax_slot_marks_restores_as_updates() {
	let mut slot = SyntaxSlot::default();
	assert!(!slot.take_updated());
	slot.updated = true;
	assert!(slot.take_updated());
	assert!(!slot.take_updated());
}

#[test]
fn stale_viewport_installs_are_ignored() {
	let mut manager = SyntaxManager::new();
	let doc_id = DocumentId(1);
	let key = ViewportKey(9);
	let fresh = rust_syntax("fn fresh() {}\n");
	let stale = rust_syntax("fn stale() {}\n");

	let change_id = manager.install_viewport_stage_b(doc_id, key, fresh, 0..12, 3);
	let returned = manager.install_viewport_stage_a(doc_id, key, stale, 0..12, 2);

	assert_eq!(returned, change_id);
	let entry = manager
		.document(doc_id)
		.and_then(|slot| slot.viewport_cache.map.get(&key))
		.expect("viewport entry should exist");
	assert!(entry.stage_a.is_none());
	assert_eq!(entry.stage_b.as_ref().map(|tree| tree.doc_version), Some(3));
}

#[test]
fn stale_viewports_do_not_reappear_after_newer_full_tree() {
	let mut manager = SyntaxManager::new();
	let doc_id = DocumentId(2);
	let key = ViewportKey(3);
	let full = rust_syntax("fn full() {}\n");
	let stale = rust_syntax("fn stale() {}\n");

	let change_id = manager.install_full(doc_id, full, 5);
	let returned = manager.install_viewport_stage_b(doc_id, key, stale, 0..11, 4);

	assert_eq!(returned, change_id);
	assert!(
		manager
			.document(doc_id)
			.is_some_and(|slot| !slot.viewport_cache.map.contains_key(&key))
	);
}
