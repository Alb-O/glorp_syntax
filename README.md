# liney-syntax

Reusable Rust libraries for tree-sitter parsing, injections, locals, highlighting,
and structural queries.

- `crates/tree-house`
  Tree-sitter integration layer with incremental parsing, injection handling,
  locals tracking, and highlight iteration.
- `crates/syntax`
  Standalone syntax document state restored from the legacy editor: viewport/full
  tree selection, viewport-aware syntax wrappers, and tiled highlight caching.
- `crates/tree-sitter-queries`
  Reusable query helpers for indentation, text objects, tags, and rainbow scopes.
