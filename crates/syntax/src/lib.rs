//! Standalone syntax state and highlighting utilities.
//!
//! This crate restores the reusable part of Xeno's syntax layer without the
//! editor runtime coupling: a syntax wrapper, per-document tree state,
//! viewport/full-tree selection, and tiled highlight caching.

mod highlight;
mod highlight_cache;
mod manager;
mod syntax;

pub use {
	highlight::{HighlightSpan, Highlighter},
	highlight_cache::{HighlightKey, HighlightSpanQuery, HighlightTile, HighlightTiles, TILE_SIZE},
	liney_tree_house::{
		Language, LanguageConfig, LanguageLoader, SealedSource, TreeCursor, highlighter::Highlight, tree_sitter,
	},
	manager::{
		DocumentId, InstalledSyntax, SyntaxManager, SyntaxSelection, SyntaxSlot, ViewportEntry, ViewportKey,
		ViewportSyntax,
	},
	syntax::{Syntax, SyntaxOptions, ViewportMetadata},
};
