//! Standalone syntax state and highlighting utilities.
//!
//! This crate provides a syntax wrapper, per-document tree state,
//! viewport/full-tree selection, and tiled highlight caching.

mod highlight_cache;
mod manager;
mod sealed_source;
mod syntax;

pub use {
	glorp_syntax_tree::{
		Language, LanguageConfig, LanguageLoader, SingleLanguageLoader, TreeCursor,
		highlighter::{Highlight, HighlightSpan, HighlightSpans},
		tree_sitter,
	},
	highlight_cache::{HighlightKey, HighlightSpanQuery, HighlightTile, HighlightTiles, TILE_SIZE},
	manager::{DocumentId, RenderSyntaxSelection, SyntaxManager, ViewportKey},
	sealed_source::SealedSource,
	syntax::{Syntax, SyntaxOptions, ViewportMetadata, ViewportSyntax},
};
