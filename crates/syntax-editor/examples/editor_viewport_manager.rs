use {
	glorp_syntax_editor::{
		DocumentId, Highlight, HighlightSpanQuery, HighlightTiles, SealedSource, SingleLanguageLoader, Syntax,
		SyntaxManager, SyntaxOptions, ViewportKey, ViewportSyntax, tree_sitter::Grammar,
	},
	ropey::Rope,
	std::error::Error,
};

const SOURCE: &str = r#"const BEFORE: u32 = 1;

fn middle(value: i32) -> i32 {
    let label = "mid";
    value + 1
}

const AFTER: u32 = 2;
"#;

const HIGHLIGHT_QUERY: &str = r#"
(identifier) @identifier
(type_identifier) @type
(primitive_type) @type.builtin
(string_literal) @string
(integer_literal) @number
"#;

fn main() -> Result<(), Box<dyn Error>> {
	let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE)?;
	let loader = SingleLanguageLoader::with_highlights(grammar, HIGHLIGHT_QUERY, "", "", |name| {
		Some(match name {
			"identifier" => Highlight::new(1),
			"type" => Highlight::new(2),
			"type.builtin" => Highlight::new(3),
			"string" => Highlight::new(4),
			"number" => Highlight::new(5),
			_ => return None,
		})
	})?;
	let rope = Rope::from_str(SOURCE);
	let full = Syntax::new(rope.slice(..), loader.language(), &loader, SyntaxOptions::default())?;
	let viewport_start = SOURCE.find("fn middle").expect("viewport start should exist") as u32;
	let viewport_end = SOURCE.find("\n\nconst AFTER").expect("viewport end should exist") as u32;
	let sealed = SealedSource::from_byte_range_with_newline_padding(rope.slice(..), viewport_start..viewport_end);
	let viewport = ViewportSyntax::new(
		sealed.into(),
		loader.language(),
		&loader,
		SyntaxOptions::default(),
		viewport_start,
	)?;

	let doc_id = DocumentId(7);
	let viewport_key = ViewportKey(1);
	let mut manager = SyntaxManager::new();
	manager.install_full(doc_id, full, 1);
	manager.install_viewport_stage_b(doc_id, viewport_key, viewport, viewport_start..viewport_end, 2);

	let selection = manager
		.syntax_for_viewport(doc_id, 2, viewport_start..viewport_end)
		.expect("viewport syntax should be selected");
	let mut tiles = HighlightTiles::new();
	let spans = tiles.get_spans(&HighlightSpanQuery {
		doc_id,
		syntax_version: manager.syntax_version(doc_id),
		rope: &rope,
		selection: selection.clone(),
		loader: &loader,
		style_resolver: Highlight::get,
		start_line: rope.byte_to_line(viewport_start as usize),
		end_line: rope.byte_to_line(viewport_end as usize) + 1,
	});

	assert!(spans.iter().any(|(_, style)| *style == 4));
	println!("selected tree={} cached spans={}", selection.tree_id(), spans.len());
	Ok(())
}
