//! Standalone syntax state and highlighting utilities.
//!
//! This crate provides a syntax wrapper, per-document tree state,
//! viewport/full-tree selection, and tiled highlight caching.

mod highlight;
mod highlight_cache;
mod manager;
mod sealed_source;
mod syntax;

pub use {
	highlight::{HighlightSpan, Highlighter},
	highlight_cache::{HighlightKey, HighlightSpanQuery, HighlightTile, HighlightTiles, TILE_SIZE},
	liney_tree_house::{
		Language, LanguageConfig, LanguageLoader, SingleLanguageLoader, TreeCursor, highlighter::Highlight, tree_sitter,
	},
	manager::{
		DocumentId, InstalledSyntax, SyntaxManager, SyntaxSelection, SyntaxSlot, ViewportEntry, ViewportKey,
		ViewportSyntax,
	},
	sealed_source::SealedSource,
	syntax::{Syntax, SyntaxOptions, ViewportMetadata},
};
