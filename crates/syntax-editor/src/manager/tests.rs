use {
	super::*,
	crate::{SingleLanguageLoader, Syntax, SyntaxOptions, tree_sitter::Grammar},
	ropey::Rope,
};

fn rust_syntax(src: &str) -> Syntax {
	let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
	let loader = SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build");
	let rope = Rope::from_str(src);
	Syntax::new(rope.slice(..), loader.language(), &loader, SyntaxOptions::default()).expect("syntax should parse")
}

#[test]
fn candidate_score_prefers_exact_full_then_enriched() {
	let exact_full = CandidateScore::new(4, true, false, 4);
	let exact_viewport_enriched = CandidateScore::new(4, false, true, 4);
	let stale_full = CandidateScore::new(3, true, false, 4);

	assert!(exact_full > exact_viewport_enriched);
	assert!(exact_viewport_enriched > stale_full);
}

#[test]
fn syntax_manager_tracks_updates_for_installed_documents() {
	let mut manager = SyntaxManager::new();
	let doc_id = DocumentId(17);

	assert!(!manager.take_updated(doc_id));
	assert!(!manager.has_syntax(doc_id));
	manager.install_full(doc_id, rust_syntax("fn alpha() {}\n"), 1);
	assert!(manager.take_updated(doc_id));
	assert!(!manager.take_updated(doc_id));
	manager.remember_full_tree_for_content(doc_id, &Rope::from_str("fn alpha() {}\n"));
	assert!(manager.restore_full_tree_for_content(doc_id, &Rope::from_str("fn alpha() {}\n"), 2));
	assert!(manager.take_updated(doc_id));
}

#[test]
fn miss_paths_do_not_create_document_slots() {
	let mut manager = SyntaxManager::new();
	let doc_id = DocumentId(88);

	manager.mark_dirty(doc_id);
	assert!(!manager.take_updated(doc_id));
	manager.drop_full(doc_id);
	manager.drop_viewports(doc_id);
	manager.drop_all_trees(doc_id);
	manager.remember_full_tree_for_content(doc_id, &Rope::from_str("fn alpha() {}\n"));
	assert!(!manager.restore_full_tree_for_content(doc_id, &Rope::from_str("fn alpha() {}\n"), 1));
	assert!(!manager.has_syntax(doc_id));
	assert_eq!(manager.syntax_version(doc_id), 0);
	assert!(!manager.remove_document(doc_id));
}

#[test]
fn full_and_best_document_selection_stay_distinct() {
	let mut manager = SyntaxManager::new();
	let doc_id = DocumentId(33);
	let key = ViewportKey(5);
	let viewport = rust_syntax("fn viewport() {}\n");

	manager.install_viewport_stage_b(doc_id, key, viewport.clone(), 0..16, 2);

	assert!(manager.full_syntax_for_doc(doc_id).is_none());
	let selection = manager
		.best_syntax_for_doc(doc_id)
		.expect("best syntax should be available");
	assert_eq!(selection.coverage, Some(0..16));

	manager.install_full(doc_id, viewport, 3);
	assert!(manager.full_syntax_for_doc(doc_id).is_some());
	assert_eq!(
		manager
			.best_syntax_for_doc(doc_id)
			.expect("full syntax should be selected")
			.coverage,
		None
	);
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
