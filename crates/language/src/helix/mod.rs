#![cfg(feature = "helix-runtime")]

use {
	crate::bundle::load_query_bundle,
	std::{
		fs,
		path::{Path, PathBuf},
		process::Command,
		time::{SystemTime, UNIX_EPOCH},
	},
	thiserror::Error,
};

/// Pin identifying the Helix runtime query snapshot to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelixRuntimeLock {
	pub upstream: String,
	pub commit: String,
}

/// Errors that can occur while fetching Helix runtime queries.
#[derive(Debug, Error)]
pub enum HelixQueryError {
	#[error("git command failed in {dir}: {message}")]
	Git { dir: String, message: String },
	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),
	#[error("helix checkout resolved to {actual} but lock requires {expected}")]
	CommitMismatch { actual: String, expected: String },
	#[error("helix runtime checkout missing queries dir: {0}")]
	MissingQueriesDir(String),
}

/// Ensures a sparse checkout containing `runtime/queries` exists under
/// `cache_root/<commit>`, returning the resolved query directory.
pub fn ensure_helix_queries_checkout(cache_root: &Path, lock: &HelixRuntimeLock) -> Result<PathBuf, HelixQueryError> {
	let checkout_dir = cache_root.join(&lock.commit);
	let queries_dir = checkout_dir.join("runtime").join("queries");

	if is_valid_checkout(&checkout_dir, &lock.commit) {
		return Ok(queries_dir);
	}

	if checkout_dir.exists() {
		fs::remove_dir_all(&checkout_dir)?;
	}
	fs::create_dir_all(cache_root)?;

	let nonce = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.expect("time should be after unix epoch")
		.as_nanos();
	let staging = cache_root.join(format!(".tmp-helix-runtime-{}-{nonce}", std::process::id()));
	if staging.exists() {
		fs::remove_dir_all(&staging)?;
	}

	clone_sparse_helix_queries(&staging, lock)?;
	match fs::rename(&staging, &checkout_dir) {
		Ok(()) => {}
		Err(_) if is_valid_checkout(&checkout_dir, &lock.commit) => {
			fs::remove_dir_all(&staging)?;
		}
		Err(error) => return Err(HelixQueryError::Io(error)),
	}

	let queries_dir = checkout_dir.join("runtime").join("queries");
	if !queries_dir.is_dir() {
		return Err(HelixQueryError::MissingQueriesDir(queries_dir.display().to_string()));
	}

	Ok(queries_dir)
}

/// Merges `.scm` files for `language` from `roots`, with later roots overriding
/// earlier roots by query kind.
pub fn merge_language_queries(
	language: &str, roots: &[PathBuf],
) -> std::io::Result<std::collections::BTreeMap<String, String>> {
	load_query_bundle(language, roots).map(crate::bundle::QueryBundle::into_queries)
}

fn is_valid_checkout(checkout_dir: &Path, commit: &str) -> bool {
	if !checkout_dir.join("runtime").join("queries").is_dir() {
		return false;
	}

	match run_git_capture(checkout_dir, &["rev-parse", "HEAD"]) {
		Ok(head) => head.trim() == commit,
		Err(_) => false,
	}
}

fn clone_sparse_helix_queries(checkout_dir: &Path, lock: &HelixRuntimeLock) -> Result<(), HelixQueryError> {
	fs::create_dir_all(checkout_dir)?;

	run_git(checkout_dir, &["init", "-q"])?;
	run_git(checkout_dir, &["remote", "add", "origin", &lock.upstream])?;
	run_git(checkout_dir, &["config", "core.sparseCheckout", "true"])?;

	let sparse_checkout = checkout_dir.join(".git").join("info").join("sparse-checkout");
	fs::write(&sparse_checkout, "/runtime/queries/\n")?;

	run_git(checkout_dir, &["fetch", "--depth=1", "origin", &lock.commit, "-q"])?;
	run_git(checkout_dir, &["checkout", "FETCH_HEAD", "-q"])?;

	let head = run_git_capture(checkout_dir, &["rev-parse", "HEAD"])?;
	if head.trim() != lock.commit {
		return Err(HelixQueryError::CommitMismatch {
			actual: head.trim().to_string(),
			expected: lock.commit.clone(),
		});
	}

	Ok(())
}

fn run_git(dir: &Path, args: &[&str]) -> Result<(), HelixQueryError> {
	run_git_capture(dir, args).map(|_| ())
}

fn run_git_capture(dir: &Path, args: &[&str]) -> Result<String, HelixQueryError> {
	let output = Command::new("git")
		.args(args)
		.current_dir(dir)
		.output()
		.map_err(|error| HelixQueryError::Git {
			dir: dir.display().to_string(),
			message: error.to_string(),
		})?;

	if output.status.success() {
		Ok(String::from_utf8_lossy(&output.stdout).into_owned())
	} else {
		Err(HelixQueryError::Git {
			dir: dir.display().to_string(),
			message: String::from_utf8_lossy(&output.stderr).into_owned(),
		})
	}
}

#[cfg(test)]
mod tests;
