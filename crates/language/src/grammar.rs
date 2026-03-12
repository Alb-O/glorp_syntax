#[cfg(feature = "jit-grammars")]
use {
	crate::build::{BuildStatus, FetchStatus, GrammarBuildError, GrammarConfig, build_grammar, fetch_grammar},
	tracing::{info, warn},
};
use {
	liney_tree_house::tree_sitter::Grammar,
	std::path::{Path, PathBuf},
	thiserror::Error,
};

#[derive(Debug, Clone)]
pub enum GrammarSource {
	Library(PathBuf),
	Builtin(&'static str),
}

#[derive(Debug, Error)]
pub enum GrammarError {
	#[error("grammar not found: {0}")]
	NotFound(String),
	#[error("failed to load grammar library: {0}")]
	LoadError(String),
	#[error("failed to build grammar: {0}")]
	BuildFailed(String),
	#[error("jit grammar build disabled: {0}")]
	JitDisabled(String),
	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),
}

#[cfg(feature = "jit-grammars")]
impl From<GrammarBuildError> for GrammarError {
	fn from(value: GrammarBuildError) -> Self {
		Self::BuildFailed(value.to_string())
	}
}

pub fn locate_grammar_library(name: &str, search_paths: &[PathBuf]) -> Option<PathBuf> {
	let lib_name = grammar_library_name(name);
	search_paths.iter().find_map(|path| {
		let candidate = if path.extension().is_some() {
			path.clone()
		} else {
			path.join(&lib_name)
		};
		candidate.exists().then_some(candidate)
	})
}

pub fn load_grammar_from_path(path: &Path, name: &str) -> Result<Grammar, GrammarError> {
	unsafe { Grammar::new(name, path).map_err(|e| GrammarError::LoadError(format!("{}: {}", path.display(), e))) }
}

pub fn load_grammar_from_paths(name: &str, search_paths: &[PathBuf]) -> Result<Grammar, GrammarError> {
	let path = locate_grammar_library(name, search_paths).ok_or_else(|| GrammarError::NotFound(name.to_string()))?;
	load_grammar_from_path(&path, name)
}

#[cfg(feature = "default-runtime-paths")]
pub fn load_grammar(name: &str) -> Result<Grammar, GrammarError> {
	load_grammar_from_paths(name, &crate::runtime_paths::grammar_search_paths())
}

#[cfg(feature = "jit-grammars")]
pub fn load_or_build_grammar_from_paths(
	config: &GrammarConfig, search_paths: &[PathBuf],
) -> Result<Grammar, GrammarError> {
	match load_grammar_from_paths(&config.grammar_id, search_paths) {
		Ok(grammar) => return Ok(grammar),
		Err(GrammarError::NotFound(_)) => {
			info!(grammar = %config.grammar_id, "Grammar not found in explicit search paths; JIT build is fallback");
		}
		Err(error) => return Err(error),
	}

	if std::env::var_os("LINEY_DISABLE_JIT_GRAMMARS").is_some() {
		return Err(GrammarError::JitDisabled(config.grammar_id.clone()));
	}

	let fetch_status = fetch_grammar(config).map_err(|e| {
		warn!(grammar = %config.grammar_id, error = %e, "Failed to fetch grammar source");
		GrammarError::BuildFailed(format!("fetch failed: {e}"))
	})?;
	match fetch_status {
		FetchStatus::Updated => info!(grammar = %config.grammar_id, "Fetched grammar source"),
		FetchStatus::UpToDate | FetchStatus::Local => {}
	}

	let build_status = build_grammar(config).map_err(|e| {
		warn!(grammar = %config.grammar_id, error = %e, "Failed to build grammar");
		GrammarError::BuildFailed(format!("build failed: {e}"))
	})?;
	match build_status {
		BuildStatus::Built => info!(grammar = %config.grammar_id, "Compiled grammar"),
		BuildStatus::AlreadyBuilt => {}
	}

	load_grammar_from_paths(&config.grammar_id, search_paths)
}

#[cfg(all(feature = "jit-grammars", feature = "default-runtime-paths"))]
pub fn load_or_build_grammar(config: &GrammarConfig) -> Result<Grammar, GrammarError> {
	load_or_build_grammar_from_paths(config, &crate::runtime_paths::grammar_search_paths())
}

fn grammar_library_name(name: &str) -> String {
	let safe_name = name.replace('-', "_");
	#[cfg(target_os = "macos")]
	{
		format!("lib{safe_name}.dylib")
	}
	#[cfg(target_os = "windows")]
	{
		format!("{safe_name}.dll")
	}
	#[cfg(not(any(target_os = "macos", target_os = "windows")))]
	{
		format!("lib{safe_name}.so")
	}
}
