use {
	super::*,
	crate::{RenderSyntax, SealedSource, SingleLanguageLoader, SyntaxOptions, tree_sitter::Grammar},
	ropey::Rope,
	std::{ops::Range, sync::Arc},
};

fn loader() -> SingleLanguageLoader {
	let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
	SingleLanguageLoader::from_queries(grammar, "", "", "").expect("loader should build")
}

fn rust_full_render_syntax(src: &str) -> RenderSyntax {
	let loader = loader();
	let rope = Rope::from_str(src);
	RenderSyntax::new_full(rope.slice(..), loader.language(), &loader, SyntaxOptions::default())
		.expect("syntax should parse")
}

fn rust_viewport_render_syntax(src: &str, coverage: Range<u32>) -> RenderSyntax {
	let loader = loader();
	let rope = Rope::from_str(src);
	let sealed = Arc::new(SealedSource::from_byte_range_with_newline_padding(
		rope.slice(..),
		coverage.clone(),
	));
	RenderSyntax::new_viewport(
		sealed,
		loader.language(),
		&loader,
		SyntaxOptions::default(),
		coverage.start,
	)
	.expect("viewport syntax should parse")
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
	let content = Rope::from_str("fn alpha() {}\n");

	assert!(!manager.take_updated(doc_id));
	assert!(!manager.has_syntax(doc_id));
	manager.install_full(doc_id, rust_full_render_syntax("fn alpha() {}\n"), 1);
	assert!(manager.take_updated(doc_id));
	assert!(!manager.take_updated(doc_id));
	manager.remember_full_tree_for_content(doc_id, &content, 7);
	assert!(manager.restore_full_tree_for_content(doc_id, &content, 7, 2));
	assert!(manager.take_updated(doc_id));
}

#[test]
fn miss_paths_do_not_create_document_slots() {
	let mut manager = SyntaxManager::new();
	let doc_id = DocumentId(88);
	let content = Rope::from_str("fn alpha() {}\n");

	manager.mark_dirty(doc_id);
	assert!(!manager.take_updated(doc_id));
	manager.drop_full(doc_id);
	manager.drop_viewports(doc_id);
	manager.drop_all_trees(doc_id);
	manager.remember_full_tree_for_content(doc_id, &content, 7);
	assert!(!manager.restore_full_tree_for_content(doc_id, &content, 7, 1));
	assert!(!manager.has_syntax(doc_id));
	assert_eq!(manager.syntax_version(doc_id), 0);
	assert!(!manager.remove_document(doc_id));
}

#[test]
fn restore_requires_matching_compatibility_key() {
	let mut manager = SyntaxManager::new();
	let doc_id = DocumentId(99);
	let content = Rope::from_str("fn alpha() {}\n");

	manager.install_full(doc_id, rust_full_render_syntax("fn alpha() {}\n"), 1);
	manager.remember_full_tree_for_content(doc_id, &content, 10);

	assert!(!manager.restore_full_tree_for_content(doc_id, &content, 11, 2));
	assert!(manager.restore_full_tree_for_content(doc_id, &content, 10, 2));
}

#[test]
fn full_document_selection_prefers_full_render_tree() {
	let mut manager = SyntaxManager::new();
	let doc_id = DocumentId(33);
	let key = ViewportKey(5);
	let viewport = rust_viewport_render_syntax("fn viewport() {}\n", 0..16);

	manager.install_viewport_stage_b(doc_id, key, viewport, 2);
	manager.install_full(doc_id, rust_full_render_syntax("fn viewport() {}\n"), 3);

	let selection = manager
		.syntax_for_viewport(doc_id, 3, 0..16)
		.expect("render selection should exist");

	assert!(selection.syntax().is_full());
	assert_eq!(selection.coverage(), None);
}

#[test]
fn render_selection_reports_viewport_coverage_explicitly() {
	let mut manager = SyntaxManager::new();
	let doc_id = DocumentId(34);
	let key = ViewportKey(8);

	manager.install_viewport_stage_b(doc_id, key, rust_viewport_render_syntax("fn viewport() {}\n", 0..16), 2);

	let selection = manager
		.syntax_for_viewport(doc_id, 2, 0..16)
		.expect("viewport selection should exist");
	assert_eq!(selection.coverage(), Some(0..16));
	assert!(!selection.syntax().is_full());
}

#[test]
fn stale_viewport_installs_are_ignored() {
	let mut manager = SyntaxManager::new();
	let doc_id = DocumentId(1);
	let key = ViewportKey(9);
	let fresh = rust_viewport_render_syntax("fn fresh() {}\n", 0..12);
	let stale = rust_viewport_render_syntax("fn stale() {}\n", 0..12);

	let change_id = manager.install_viewport_stage_b(doc_id, key, fresh, 3);
	let returned = manager.install_viewport_stage_a(doc_id, key, stale, 2);

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
	let full = rust_full_render_syntax("fn full() {}\n");
	let stale = rust_viewport_render_syntax("fn stale() {}\n", 0..11);

	let change_id = manager.install_full(doc_id, full, 5);
	let returned = manager.install_viewport_stage_b(doc_id, key, stale, 4);

	assert_eq!(returned, change_id);
	assert!(
		manager
			.document(doc_id)
			.is_some_and(|slot| !slot.viewport_cache.map.contains_key(&key))
	);
}
