//! Generic language-runtime helpers for tree-sitter grammars and queries.
//!
//! This crate provides the pieces needed to:
//! - find and load compiled tree-sitter grammars
//! - fetch grammar sources from git and build shared libraries
//! - read query files with strict `; inherits` resolution
//! - load query bundles either resolved (default) or raw
//! - pin and fetch Helix runtime queries for build-time embedding
//! - build a multi-language `LanguageLoader` from a runtime registry
//! - detect root languages from editor-facing inputs such as filenames and shebangs

#[cfg(feature = "jit-grammars")]
pub mod build;
pub mod bundle;
pub mod grammar;
#[cfg(feature = "helix-runtime")]
pub mod helix;
mod loader;
pub mod query;
pub mod registry;
#[cfg(feature = "default-runtime-paths")]
pub mod runtime_paths;

#[cfg(feature = "helix-runtime")]
pub use helix::{HelixQueryError, HelixRuntimeLock, ensure_helix_queries_checkout, merge_language_queries};
#[cfg(feature = "jit-grammars")]
pub use {
	build::{
		BuildStatus, FetchStatus, GrammarBuildError, GrammarConfig, GrammarSource, build_grammar, fetch_grammar,
		get_grammar_src_dir, grammar_lib_dir, grammar_sources_dir, library_extension,
	},
	grammar::{load_or_build_grammar, load_or_build_grammar_from_paths},
};
pub use {
	bundle::{QueryBundle, QueryBundleError, load_query_bundle, load_raw_query_bundle},
	grammar::{
		GrammarError, GrammarSource as LoadedGrammarSource, load_grammar_from_path, load_grammar_from_paths,
		locate_grammar_library,
	},
	loader::{RegistryLanguageLoadReport, RegistryLanguageLoader, RegistryLanguageLoaderError},
	query::{QueryReadError, read_optional_query_from_paths, read_query_from_paths},
	registry::{DuplicateLanguageIdError, GrammarLocator, LanguageId, LanguageRegistry, LanguageSpec, QueryLocator},
};
#[cfg(feature = "default-runtime-paths")]
pub use {
	grammar::load_grammar,
	query::read_query,
	runtime_paths::{cache_dir, grammar_search_paths, query_search_paths, runtime_dir},
};
