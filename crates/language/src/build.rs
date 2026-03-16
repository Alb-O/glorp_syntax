use {
	crate::runtime_paths::{cache_dir, runtime_dir},
	std::{
		fs,
		path::{Path, PathBuf},
		process::{Command, Stdio},
		sync::OnceLock,
	},
	thiserror::Error,
	tracing::info,
};

#[derive(Debug, Clone)]
pub struct GrammarConfig {
	pub grammar_id: String,
	pub source: GrammarSource,
}

#[derive(Debug, Clone)]
pub enum GrammarSource {
	Local {
		path: String,
	},
	Git {
		remote: String,
		revision: String,
		subpath: Option<String>,
	},
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchStatus {
	UpToDate,
	Updated,
	Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildStatus {
	AlreadyBuilt,
	Built,
}

#[derive(Debug, Error)]
pub enum GrammarBuildError {
	#[error("git is not available on PATH")]
	GitNotAvailable,
	#[error("git command failed: {0}")]
	GitCommand(String),
	#[error("compilation failed: {0}")]
	Compilation(String),
	#[error("no parser.c found in {0}")]
	NoParserSource(PathBuf),
	#[error("I/O error: {0}")]
	Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, GrammarBuildError>;
type ResolvedCompilers = (Option<Box<str>>, Option<Box<str>>);

pub fn grammar_sources_dir() -> PathBuf {
	cache_dir().unwrap_or_else(runtime_dir).join("grammars").join("sources")
}

pub fn grammar_lib_dir() -> PathBuf {
	if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR")
		&& let Some(workspace) = Path::new(&manifest).ancestors().nth(2)
	{
		return workspace.join("target").join("grammars");
	}

	cache_dir().map_or_else(|| runtime_dir().join("grammars"), |cache| cache.join("grammars"))
}

pub fn get_grammar_src_dir(grammar: &GrammarConfig) -> PathBuf {
	match &grammar.source {
		GrammarSource::Local { path } => PathBuf::from(path).join("src"),
		GrammarSource::Git { subpath, .. } => {
			let base = grammar_sources_dir().join(&grammar.grammar_id);
			match subpath {
				Some(subpath) => base.join(subpath).join("src"),
				None => base.join("src"),
			}
		}
	}
}

#[cfg(target_os = "windows")]
pub fn library_extension() -> &'static str {
	"dll"
}

#[cfg(target_os = "macos")]
pub fn library_extension() -> &'static str {
	"dylib"
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn library_extension() -> &'static str {
	"so"
}

pub fn fetch_grammar(grammar: &GrammarConfig) -> Result<FetchStatus> {
	let GrammarSource::Git { remote, revision, .. } = &grammar.source else {
		return Ok(FetchStatus::Local);
	};

	ensure_git_available()?;

	let grammar_dir = grammar_sources_dir().join(&grammar.grammar_id);

	if is_valid_git_repo(&grammar_dir) {
		update_existing_repo(&grammar_dir, &grammar.grammar_id, revision)
	} else {
		clone_fresh(&grammar_dir, &grammar.grammar_id, remote, revision)
	}
}

pub fn build_grammar(grammar: &GrammarConfig) -> Result<BuildStatus> {
	let src_dir = get_grammar_src_dir(grammar);
	if !src_dir.join("parser.c").exists() {
		return Err(GrammarBuildError::NoParserSource(src_dir));
	}

	let lib_dir = grammar_lib_dir();
	fs::create_dir_all(&lib_dir)?;

	let lib_path = lib_dir.join(format!(
		"lib{}.{}",
		grammar.grammar_id.replace('-', "_"),
		library_extension()
	));
	if !needs_recompile(&src_dir, &lib_path) {
		return Ok(BuildStatus::AlreadyBuilt);
	}

	info!(grammar = %grammar.grammar_id, lib_path = %lib_path.display(), "Compiling grammar");

	let needs_cxx = src_dir.join("scanner.cc").exists();
	let compilers = resolve_compilers();
	let compiler = if needs_cxx {
		compilers.1.as_deref().ok_or_else(|| {
			GrammarBuildError::Compilation(format!(
				"C++ compiler required for {} but none found. Install clang++/g++ or set CXX.",
				grammar.grammar_id
			))
		})?
	} else {
		compilers.0.as_deref().ok_or_else(|| {
			GrammarBuildError::Compilation("C compiler required but none found. Install clang/gcc or set CC.".into())
		})?
	};

	link_shared_library(&src_dir, &lib_path, compiler, needs_cxx)?;

	if !lib_path.exists() {
		return Err(GrammarBuildError::Compilation(format!(
			"compilation succeeded but library not found at {}",
			lib_path.display()
		)));
	}

	Ok(BuildStatus::Built)
}

fn ensure_git_available() -> Result<()> {
	Command::new("git")
		.arg("--version")
		.output()
		.map_err(|_| GrammarBuildError::GitNotAvailable)?;
	Ok(())
}

fn is_valid_git_repo(dir: &Path) -> bool {
	dir.join(".git").join("HEAD").exists()
}

fn update_existing_repo(grammar_dir: &Path, grammar_id: &str, revision: &str) -> Result<FetchStatus> {
	let current_revision = git_rev_parse(grammar_dir)?;
	if current_revision.starts_with(revision) || revision.starts_with(&current_revision) {
		return Ok(FetchStatus::UpToDate);
	}

	info!(grammar = %grammar_id, "Updating grammar");
	git_fetch(grammar_dir, revision)?;
	git_checkout(grammar_dir, "FETCH_HEAD")?;

	Ok(FetchStatus::Updated)
}

fn clone_fresh(grammar_dir: &Path, grammar_id: &str, remote: &str, revision: &str) -> Result<FetchStatus> {
	if grammar_dir.exists() {
		fs::remove_dir_all(grammar_dir)?;
	}
	if let Some(parent) = grammar_dir.parent() {
		fs::create_dir_all(parent)?;
	}

	info!(grammar = %grammar_id, "Cloning grammar");
	git_clone(remote, grammar_dir)?;
	git_fetch(grammar_dir, revision).or_else(|_| git_fetch_full(grammar_dir, revision))?;
	git_checkout(grammar_dir, revision).or_else(|_| git_checkout(grammar_dir, "FETCH_HEAD"))?;

	Ok(FetchStatus::Updated)
}

fn git_rev_parse(dir: &Path) -> Result<String> {
	let output = Command::new("git")
		.args(["rev-parse", "HEAD"])
		.current_dir(dir)
		.output()
		.map_err(|error| GrammarBuildError::GitCommand(error.to_string()))?;
	if output.status.success() {
		Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
	} else {
		Err(GrammarBuildError::GitCommand(
			String::from_utf8_lossy(&output.stderr).into_owned(),
		))
	}
}

fn git_fetch(dir: &Path, revision: &str) -> Result<()> {
	run_git(dir, &["fetch", "--depth", "1", "origin", revision])
}

fn git_fetch_full(dir: &Path, revision: &str) -> Result<()> {
	run_git(dir, &["fetch", "origin", revision])
}

fn git_checkout(dir: &Path, target: &str) -> Result<()> {
	run_git(dir, &["checkout", target])
}

fn git_clone(remote: &str, dest: &Path) -> Result<()> {
	let output = Command::new("git")
		.args(["clone", "--depth", "1", "--single-branch", remote])
		.arg(dest)
		.output()
		.map_err(|error| GrammarBuildError::GitCommand(error.to_string()))?;
	if output.status.success() {
		Ok(())
	} else {
		Err(GrammarBuildError::GitCommand(
			String::from_utf8_lossy(&output.stderr).into_owned(),
		))
	}
}

fn run_git(dir: &Path, args: &[&str]) -> Result<()> {
	let output = Command::new("git")
		.args(args)
		.current_dir(dir)
		.output()
		.map_err(|error| GrammarBuildError::GitCommand(error.to_string()))?;
	if output.status.success() {
		Ok(())
	} else {
		Err(GrammarBuildError::GitCommand(
			String::from_utf8_lossy(&output.stderr).into_owned(),
		))
	}
}

fn find_compiler<'a>(candidates: &[&'a str]) -> Option<&'a str> {
	candidates.iter().copied().find(|name| {
		Command::new(name)
			.arg("--version")
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status()
			.is_ok()
	})
}

fn resolve_compilers() -> &'static ResolvedCompilers {
	static COMPILERS: OnceLock<ResolvedCompilers> = OnceLock::new();
	COMPILERS.get_or_init(|| {
		#[cfg(unix)]
		const CC_CANDIDATES: &[&str] = &["cc", "clang", "gcc"];
		#[cfg(unix)]
		const CXX_CANDIDATES: &[&str] = &["c++", "clang++", "g++"];
		#[cfg(windows)]
		const CC_CANDIDATES: &[&str] = &["cl", "clang-cl", "clang", "gcc"];
		#[cfg(windows)]
		const CXX_CANDIDATES: &[&str] = &["cl", "clang-cl", "clang++", "g++"];
		#[cfg(not(any(unix, windows)))]
		const CC_CANDIDATES: &[&str] = &["cc", "clang", "gcc"];
		#[cfg(not(any(unix, windows)))]
		const CXX_CANDIDATES: &[&str] = &["c++", "clang++", "g++"];

		let cc = std::env::var("CC")
			.ok()
			.map(String::into_boxed_str)
			.or_else(|| find_compiler(CC_CANDIDATES).map(Into::into));
		let cxx = std::env::var("CXX")
			.ok()
			.map(String::into_boxed_str)
			.or_else(|| find_compiler(CXX_CANDIDATES).map(Into::into));
		(cc, cxx)
	})
}

fn needs_recompile(src_dir: &Path, lib_path: &Path) -> bool {
	let Ok(lib_mtime) = fs::metadata(lib_path).and_then(|metadata| metadata.modified()) else {
		return true;
	};

	["parser.c", "scanner.c", "scanner.cc"].iter().any(|file| {
		fs::metadata(src_dir.join(file))
			.and_then(|metadata| metadata.modified())
			.is_ok_and(|src_mtime| src_mtime > lib_mtime)
	})
}

fn link_shared_library(src_dir: &Path, lib_path: &Path, compiler: &str, needs_cxx: bool) -> Result<()> {
	let scanner_cc = src_dir.join("scanner.cc");
	let scanner_c = src_dir.join("scanner.c");

	#[cfg(unix)]
	{
		let mut cmd = Command::new(compiler);
		cmd.args(["-shared", "-fPIC", "-O3", "-fno-exceptions"])
			.arg("-I")
			.arg(src_dir)
			.arg("-o")
			.arg(lib_path)
			.arg(src_dir.join("parser.c"));

		if needs_cxx && scanner_cc.exists() {
			cmd.args(["-std=c++14", "-lstdc++"]).arg(&scanner_cc);
		} else if scanner_c.exists() {
			cmd.arg(&scanner_c);
		}

		#[cfg(target_os = "linux")]
		cmd.arg("-Wl,-z,relro,-z,now");

		run_compiler(cmd)
	}

	#[cfg(windows)]
	{
		let mut cmd = Command::new("cl.exe");
		cmd.args(["/nologo", "/LD", "/O2", "/utf-8"])
			.arg(format!("/I{}", src_dir.display()))
			.arg(format!("/Fe:{}", lib_path.display()))
			.arg(src_dir.join("parser.c"));

		if needs_cxx && scanner_cc.exists() {
			cmd.arg("/std:c++14").arg(&scanner_cc);
		} else if scanner_c.exists() {
			cmd.arg(&scanner_c);
		}

		run_compiler(cmd)
	}
}

fn run_compiler(mut cmd: Command) -> Result<()> {
	let output = cmd
		.output()
		.map_err(|error| GrammarBuildError::Compilation(error.to_string()))?;
	if output.status.success() {
		Ok(())
	} else {
		Err(GrammarBuildError::Compilation(
			String::from_utf8_lossy(&output.stderr).into_owned(),
		))
	}
}
