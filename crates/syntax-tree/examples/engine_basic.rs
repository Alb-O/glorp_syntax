use {
	glorp_syntax_tree::{
		DocumentSession, EngineConfig, HighlightSpan, SingleLanguageLoader, StringText, highlighter::Highlight,
		tree_sitter::Grammar,
	},
	std::error::Error,
};

const SOURCE: &str = r#"fn answer() -> i32 {
    42
}
"#;

const HIGHLIGHTS: &str = r#"
(identifier) @function
(integer_literal) @number
"#;

fn main() -> Result<(), Box<dyn Error>> {
	let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE)?;
	let loader = SingleLanguageLoader::with_highlights(grammar, HIGHLIGHTS, "", "", |name| {
		Some(match name {
			"function" => Highlight::new(1),
			"number" => Highlight::new(2),
			_ => return None,
		})
	})?;
	let snapshot = DocumentSession::new(
		loader.language(),
		&StringText::new(SOURCE),
		&loader,
		EngineConfig::default(),
	)?
	.snapshot();

	let spans: Vec<HighlightSpan> = snapshot.highlight_spans(&loader, ..).collect();
	assert!(
		spans
			.iter()
			.any(|span| &SOURCE[span.start as usize..span.end as usize] == "answer")
	);
	assert!(
		spans
			.iter()
			.any(|span| &SOURCE[span.start as usize..span.end as usize] == "42")
	);

	println!("snapshot revision={} spans={}", snapshot.revision().0, spans.len());
	Ok(())
}
