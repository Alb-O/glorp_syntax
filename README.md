# liney-syntax

Reusable Rust crates for tree-sitter engines, runtime loading, editor adapters,
and structural query helpers.

## Crates

- `crates/syntax-tree`
  Generic engine. `DocumentSession` for writes, `DocumentSnapshot` for reads.
- `crates/language`
  Runtime and registry helpers. Explicit-path APIs are core; runtime-path, JIT,
  and Helix helpers are feature-gated.
- `crates/syntax-editor`
  Editor adapter. Viewports, document IDs, sealed windows, and highlight tiles.
- `crates/queries`
  Reusable query products built on engine snapshots.

## Use

```rust
use liney_syntax_tree::{
    DocumentSession, EngineConfig, Language, SingleLanguageLoader, StringText,
    tree_sitter::Grammar,
};

let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE)?;
let loader = SingleLanguageLoader::from_queries(Language::new(0), grammar, "", "", "")?;
let session = DocumentSession::new(
    loader.language(),
    &StringText::new("fn answer() -> i32 { 42 }\n"),
    &loader,
    EngineConfig::default(),
)?;
let snapshot = session.snapshot();
let node = snapshot.named_node_at(3, 9);
```

## Features

- `liney-language` defaults: `default-runtime-paths`, `jit-grammars`, `helix-runtime`
- `liney-language --no-default-features`: explicit-path runtime and registry only

## Examples

- `crates/syntax-tree/examples/engine_basic.rs`
  Parse and highlight through `DocumentSession` and `DocumentSnapshot`.
- `crates/syntax-tree/examples/engine_edits.rs`
  Apply edits through the engine session API and inspect revision metadata.
- `crates/language/examples/runtime_registry.rs`
  Load queries from explicit roots through the runtime registry layer.
- `crates/syntax-editor/examples/editor_viewport_manager.rs`
  Use the optional editor adapter for viewport selection and tile caching.
- `crates/queries/examples/queries_tags.rs`
  Run reusable tag queries against an engine snapshot.
