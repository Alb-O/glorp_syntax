use std::path::PathBuf;

fn push_unique(dirs: &mut Vec<PathBuf>, path: PathBuf) {
	if !dirs.contains(&path) {
		dirs.push(path);
	}
}

pub fn runtime_dir() -> PathBuf {
	std::env::var("GLORP_SYNTAX_RUNTIME").map_or_else(
		|_| data_local_dir().map_or_else(|| PathBuf::from("."), |dir| dir.join("glorp_syntax")),
		PathBuf::from,
	)
}

pub fn cache_dir() -> Option<PathBuf> {
	#[cfg(unix)]
	{
		std::env::var_os("XDG_CACHE_HOME")
			.map(PathBuf::from)
			.or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
			.map(|path| path.join("glorp_syntax"))
	}
	#[cfg(windows)]
	{
		std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("glorp_syntax").join("cache"))
	}
	#[cfg(not(any(unix, windows)))]
	{
		None
	}
}

pub fn grammar_search_paths() -> Vec<PathBuf> {
	let mut dirs = Vec::with_capacity(8);

	if let Ok(runtime) = std::env::var("GLORP_SYNTAX_RUNTIME") {
		push_unique(&mut dirs, PathBuf::from(runtime).join("grammars"));
	}

	if let Ok(exe) = std::env::current_exe()
		&& let Some(bin_dir) = exe.parent()
	{
		push_unique(
			&mut dirs,
			bin_dir.join("..").join("share").join("glorp_syntax").join("grammars"),
		);
	}

	if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR")
		&& let Some(workspace) = PathBuf::from(manifest).ancestors().nth(2)
	{
		push_unique(&mut dirs, workspace.join("target").join("grammars"));
	}

	if let Some(cache) = cache_dir() {
		push_unique(&mut dirs, cache.join("grammars"));
	}

	if let Some(data) = data_local_dir() {
		push_unique(&mut dirs, data.join("glorp_syntax").join("grammars"));
	}

	for helix_dir in helix_runtime_dirs() {
		push_unique(&mut dirs, helix_dir.join("grammars"));
	}

	dirs
}

pub fn query_search_paths() -> Vec<PathBuf> {
	let mut dirs = Vec::with_capacity(4);

	if let Ok(runtime) = std::env::var("GLORP_SYNTAX_RUNTIME") {
		push_unique(&mut dirs, PathBuf::from(runtime).join("queries"));
	}

	if let Some(data) = data_local_dir() {
		push_unique(&mut dirs, data.join("glorp_syntax").join("queries"));
	}

	for helix_dir in helix_runtime_dirs() {
		push_unique(&mut dirs, helix_dir.join("queries"));
	}

	dirs
}

fn data_local_dir() -> Option<PathBuf> {
	#[cfg(unix)]
	{
		std::env::var_os("XDG_DATA_HOME")
			.map(PathBuf::from)
			.or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share")))
	}
	#[cfg(windows)]
	{
		std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
	}
	#[cfg(not(any(unix, windows)))]
	{
		None
	}
}

fn helix_runtime_dirs() -> Vec<PathBuf> {
	let mut dirs = Vec::with_capacity(3);

	if let Ok(runtime) = std::env::var("HELIX_RUNTIME") {
		push_unique(&mut dirs, PathBuf::from(runtime));
	}

	#[cfg(unix)]
	if let Some(config) = std::env::var_os("XDG_CONFIG_HOME")
		.map(PathBuf::from)
		.or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
	{
		let helix_runtime = config.join("helix").join("runtime");
		if helix_runtime.is_dir() {
			push_unique(&mut dirs, helix_runtime);
		}
	}

	if let Some(data) = data_local_dir() {
		let helix_runtime = data.join("helix").join("runtime");
		if helix_runtime.is_dir() {
			push_unique(&mut dirs, helix_runtime);
		}
	}

	dirs
}
