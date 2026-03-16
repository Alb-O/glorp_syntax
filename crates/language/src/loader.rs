use {
	crate::{GrammarError, LanguageId, LanguageRegistry, QueryReadError},
	glorp_syntax_tree::{
		InjectionLanguageMarker, Language, LanguageConfig, LanguageLoader, tree_sitter::query::ParseError,
	},
	regex::Regex,
	std::collections::{BTreeMap, HashMap},
	thiserror::Error,
};

/// [`LanguageLoader`] implementation backed by a [`LanguageRegistry`].
///
/// This is the preferred integration surface for editors that need multiple
/// languages, runtime query loading, and injection resolution.
#[derive(Debug)]
pub struct RegistryLanguageLoader {
	languages: Vec<LoadedLanguage>,
	languages_by_id: BTreeMap<LanguageId, Language>,
	exact_names: HashMap<Box<str>, ExactNameMatcher>,
	content_regexes: Vec<RegexMatcher>,
	filename_regexes: Vec<RegexMatcher>,
	shebang_regexes: Vec<RegexMatcher>,
}

#[derive(Debug)]
struct LoadedLanguage {
	id: LanguageId,
	config: LanguageConfig,
}

#[derive(Debug)]
struct RegexMatcher {
	language: Language,
	regex: Regex,
}

#[derive(Debug, Clone)]
struct ExactNameMatcher {
	language: Language,
	owner: LanguageId,
}

struct PendingLanguage {
	id: LanguageId,
	config: LanguageConfig,
	exact_names: Vec<Box<str>>,
	content_regexes: Vec<Regex>,
	filename_regexes: Vec<Regex>,
	shebang_regexes: Vec<Regex>,
}

/// Errors that can occur while building a [`RegistryLanguageLoader`].
#[derive(Debug, Error)]
pub enum RegistryLanguageLoaderError {
	#[error("failed to load grammar for language {language}: {source}")]
	Grammar {
		language: LanguageId,
		#[source]
		source: GrammarError,
	},
	#[error("failed to read {filename} for language {language}: {source}")]
	Query {
		language: LanguageId,
		filename: &'static str,
		#[source]
		source: QueryReadError,
	},
	#[error("failed to parse queries for language {language}: {source}")]
	Parse {
		language: LanguageId,
		#[source]
		source: Box<ParseError>,
	},
	#[error("invalid {matcher_kind} regex for language {language}: {pattern}: {source}")]
	InvalidRegex {
		language: LanguageId,
		matcher_kind: &'static str,
		pattern: String,
		#[source]
		source: regex::Error,
	},
	#[error("duplicate injection name {name} for languages {first} and {second}")]
	DuplicateInjectionName {
		name: String,
		first: LanguageId,
		second: LanguageId,
	},
}

/// Outcome of tolerant registry loading.
#[derive(Debug)]
pub struct RegistryLanguageLoadReport {
	/// Loader containing every language that loaded successfully.
	pub loader: RegistryLanguageLoader,
	/// Per-language issues encountered while building the loader.
	pub issues: Vec<RegistryLanguageLoaderError>,
}

impl RegistryLanguageLoader {
	/// Loads grammars, queries, and injection matchers for every registry entry.
	///
	/// The resulting [`Language`] values are scoped to this loader instance and
	/// are assigned in the deterministic iteration order of the registry.
	///
	/// This constructor is strict: the first load error aborts the whole build.
	pub fn from_registry(registry: &LanguageRegistry) -> Result<Self, RegistryLanguageLoaderError> {
		let mut loader = Self::with_capacity(registry.iter().size_hint().0);
		for (id, spec) in registry.iter() {
			let pending = load_pending_language(registry, &loader.exact_names, id, spec)?;
			loader.commit_language(pending);
		}
		Ok(loader)
	}

	/// Loads as many registry languages as possible, recording per-language failures.
	///
	/// Languages that fail grammar loading, query loading/parsing, regex compilation, or
	/// exact-name registration are skipped and reported in [`RegistryLanguageLoadReport::issues`].
	pub fn from_registry_tolerant(registry: &LanguageRegistry) -> RegistryLanguageLoadReport {
		let mut loader = Self::with_capacity(registry.iter().size_hint().0);
		let mut issues = Vec::new();

		for (id, spec) in registry.iter() {
			match load_pending_language(registry, &loader.exact_names, id, spec) {
				Ok(pending) => loader.commit_language(pending),
				Err(error) => issues.push(error),
			}
		}

		RegistryLanguageLoadReport { loader, issues }
	}

	/// Returns the loader-scoped engine language token for a registry language ID.
	pub fn language(&self, id: &LanguageId) -> Option<Language> {
		self.languages_by_id.get(id).copied()
	}

	/// Returns the registry language ID for a loader-scoped engine language token.
	pub fn language_id(&self, language: Language) -> Option<&LanguageId> {
		self.languages.get(language.idx()).map(|loaded| &loaded.id)
	}

	fn with_capacity(language_count: usize) -> Self {
		Self {
			languages: Vec::new(),
			languages_by_id: BTreeMap::new(),
			exact_names: HashMap::with_capacity(language_count),
			content_regexes: Vec::with_capacity(language_count),
			filename_regexes: Vec::with_capacity(language_count),
			shebang_regexes: Vec::with_capacity(language_count),
		}
	}

	fn commit_language(&mut self, pending: PendingLanguage) {
		let PendingLanguage {
			id,
			config,
			exact_names,
			content_regexes,
			filename_regexes,
			shebang_regexes,
		} = pending;
		// Tokens are assigned only when a language successfully commits so tolerant loading
		// keeps `Language` ids dense and deterministic over the surviving registry entries.
		let language = Language::from_raw(self.languages.len() as u32);
		self.languages_by_id.insert(id.clone(), language);
		self.languages.push(LoadedLanguage { id: id.clone(), config });
		for name in exact_names {
			self.exact_names.insert(
				name,
				ExactNameMatcher {
					language,
					owner: id.clone(),
				},
			);
		}
		self.content_regexes.extend(
			content_regexes
				.into_iter()
				.map(|regex| RegexMatcher { language, regex }),
		);
		self.filename_regexes.extend(
			filename_regexes
				.into_iter()
				.map(|regex| RegexMatcher { language, regex }),
		);
		self.shebang_regexes.extend(
			shebang_regexes
				.into_iter()
				.map(|regex| RegexMatcher { language, regex }),
		);
	}
}

impl LanguageLoader for RegistryLanguageLoader {
	fn language_for_marker(&self, marker: InjectionLanguageMarker) -> Option<Language> {
		match marker {
			InjectionLanguageMarker::Name(name) => self.exact_names.get(name).map(|matcher| matcher.language),
			InjectionLanguageMarker::Match(text) => longest_regex_match(&self.content_regexes, &text.to_string()),
			InjectionLanguageMarker::Filename(text) => longest_regex_match(&self.filename_regexes, &text.to_string()),
			InjectionLanguageMarker::Shebang(text) => longest_regex_match(&self.shebang_regexes, &text.to_string()),
		}
	}

	fn get_config(&self, lang: Language) -> Option<&LanguageConfig> {
		self.languages.get(lang.idx()).map(|loaded| &loaded.config)
	}
}

fn load_pending_language(
	registry: &LanguageRegistry, exact_names: &HashMap<Box<str>, ExactNameMatcher>, id: &LanguageId,
	spec: &crate::LanguageSpec,
) -> Result<PendingLanguage, RegistryLanguageLoaderError> {
	let grammar = registry
		.load_grammar(id)
		.map_err(|source| RegistryLanguageLoaderError::Grammar {
			language: id.clone(),
			source,
		})?;
	let highlights = registry
		.read_optional_query(id, "highlights.scm")
		.map_err(|source| RegistryLanguageLoaderError::Query {
			language: id.clone(),
			filename: "highlights.scm",
			source,
		})?
		.unwrap_or_default();
	let injections = registry
		.read_optional_query(id, "injections.scm")
		.map_err(|source| RegistryLanguageLoaderError::Query {
			language: id.clone(),
			filename: "injections.scm",
			source,
		})?
		.unwrap_or_default();
	let locals = registry
		.read_optional_query(id, "locals.scm")
		.map_err(|source| RegistryLanguageLoaderError::Query {
			language: id.clone(),
			filename: "locals.scm",
			source,
		})?
		.unwrap_or_default();
	let config = LanguageConfig::new(grammar, &highlights, &injections, &locals).map_err(|source| {
		RegistryLanguageLoaderError::Parse {
			language: id.clone(),
			source: Box::new(source),
		}
	})?;

	let exact_names_for_language = std::iter::once(Box::<str>::from(id.as_str()))
		.chain(
			spec.injection_names
				.iter()
				.map(|alias| Box::<str>::from(alias.as_str())),
		)
		.collect::<Vec<_>>();
	check_exact_names(exact_names, &exact_names_for_language, id)?;

	Ok(PendingLanguage {
		id: id.clone(),
		config,
		exact_names: exact_names_for_language,
		content_regexes: compile_regexes(id, "content", &spec.content_regexes)?,
		filename_regexes: compile_regexes(id, "filename", &spec.filename_regexes)?,
		shebang_regexes: compile_regexes(id, "shebang", &spec.shebang_regexes)?,
	})
}

fn check_exact_names(
	exact_names: &HashMap<Box<str>, ExactNameMatcher>, names: &[Box<str>], current: &LanguageId,
) -> Result<(), RegistryLanguageLoaderError> {
	for name in names {
		if let Some(existing) = exact_names.get(name.as_ref()) {
			return Err(RegistryLanguageLoaderError::DuplicateInjectionName {
				name: name.to_string(),
				first: existing.owner.clone(),
				second: current.clone(),
			});
		}
	}
	Ok(())
}

fn compile_regexes(
	id: &LanguageId, matcher_kind: &'static str, patterns: &[String],
) -> Result<Vec<Regex>, RegistryLanguageLoaderError> {
	let mut compiled = Vec::with_capacity(patterns.len());
	for pattern in patterns {
		let regex = Regex::new(pattern).map_err(|source| RegistryLanguageLoaderError::InvalidRegex {
			language: id.clone(),
			matcher_kind,
			pattern: pattern.clone(),
			source,
		})?;
		compiled.push(regex);
	}
	Ok(compiled)
}

fn longest_regex_match(matchers: &[RegexMatcher], text: &str) -> Option<Language> {
	let mut best = None;
	for matcher in matchers {
		for matched in matcher.regex.find_iter(text) {
			let len = matched.end() - matched.start();
			if best.is_none_or(|(best_len, _)| len > best_len) {
				best = Some((len, matcher.language));
			}
		}
	}
	best.map(|(_, language)| language)
}

#[cfg(test)]
mod tests {
	use {
		super::*,
		crate::{
			GrammarLocator, LanguageSpec, QueryLocator,
			build::{GrammarConfig, GrammarSource, build_grammar, grammar_lib_dir},
		},
		glorp_syntax_tree::tree_sitter::Grammar,
		ropey::Rope,
		std::{
			fs,
			path::PathBuf,
			sync::OnceLock,
			time::{SystemTime, UNIX_EPOCH},
		},
	};

	fn loader() -> RegistryLanguageLoader {
		let language = Language::from_raw(0);
		let config = LanguageConfig::new(
			Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load"),
			"",
			"",
			"",
		)
		.expect("config should build");
		RegistryLanguageLoader {
			languages: vec![LoadedLanguage {
				id: LanguageId::new("rust"),
				config,
			}],
			languages_by_id: BTreeMap::from([(LanguageId::new("rust"), language)]),
			exact_names: HashMap::from([
				(
					Box::<str>::from("rust"),
					ExactNameMatcher {
						language,
						owner: LanguageId::new("rust"),
					},
				),
				(
					Box::<str>::from("rs"),
					ExactNameMatcher {
						language,
						owner: LanguageId::new("rust"),
					},
				),
			]),
			content_regexes: vec![RegexMatcher {
				language,
				regex: Regex::new("rust|rs").expect("regex should compile"),
			}],
			filename_regexes: vec![RegexMatcher {
				language,
				regex: Regex::new("\\.rs$").expect("regex should compile"),
			}],
			shebang_regexes: vec![RegexMatcher {
				language,
				regex: Regex::new("cargo").expect("regex should compile"),
			}],
		}
	}

	fn temp_root(name: &str) -> std::path::PathBuf {
		let nonce = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.expect("time should be after unix epoch")
			.as_nanos();
		let root = std::env::temp_dir().join(format!("glorp_syntax-loader-{name}-{nonce}"));
		fs::create_dir_all(&root).expect("temp root should be created");
		root
	}

	fn cargo_home() -> PathBuf {
		std::env::var_os("CARGO_HOME").map_or_else(
			|| {
				PathBuf::from(std::env::var_os("HOME").expect("HOME should be set"))
					.join(".local")
					.join("share")
					.join("cargo")
			},
			PathBuf::from,
		)
	}

	fn rust_grammar_dir() -> PathBuf {
		static GRAMMAR_DIR: OnceLock<PathBuf> = OnceLock::new();
		GRAMMAR_DIR
			.get_or_init(|| {
				let registry_src = cargo_home().join("registry").join("src");
				let source = fs::read_dir(&registry_src)
					.expect("cargo registry source root should exist")
					.filter_map(Result::ok)
					.map(|entry| entry.path().join("tree-sitter-rust-0.24.0"))
					.find(|path| path.exists())
					.expect("tree-sitter-rust source should exist in cargo registry");
				build_grammar(&GrammarConfig {
					grammar_id: "rust".to_owned(),
					source: GrammarSource::Local {
						path: source.to_string_lossy().into_owned(),
					},
				})
				.expect("tree-sitter-rust grammar should build");
				grammar_lib_dir()
			})
			.clone()
	}

	fn rust_spec(id: &str) -> LanguageSpec {
		let mut spec = LanguageSpec::new(id, "rust");
		spec.grammar_paths.push(rust_grammar_dir());
		spec
	}

	fn registry() -> (LanguageRegistry, std::path::PathBuf) {
		let root = temp_root("queries");
		let registry = LanguageRegistry::new(GrammarLocator::default(), QueryLocator::new([root.clone()]));
		(registry, root)
	}

	#[test]
	fn registry_loader_matches_exact_and_regex_markers() {
		let loader = loader();
		let language = Language::from_raw(0);
		let fence = Rope::from_str("```rust");
		let filename = Rope::from_str("main.rs");
		let shebang = Rope::from_str("#!/usr/bin/env cargo");
		assert_eq!(loader.language(&LanguageId::new("rust")), Some(language));
		assert_eq!(loader.language_id(language).map(LanguageId::as_str), Some("rust"));
		assert_eq!(
			loader.language_for_marker(InjectionLanguageMarker::Name("rs")),
			Some(language)
		);
		assert_eq!(
			loader.language_for_marker(InjectionLanguageMarker::Match(fence.slice(..))),
			Some(language)
		);
		assert_eq!(
			loader.language_for_marker(InjectionLanguageMarker::Filename(filename.slice(..))),
			Some(language)
		);
		assert_eq!(
			loader.language_for_marker(InjectionLanguageMarker::Shebang(shebang.slice(..))),
			Some(language)
		);
	}

	#[test]
	fn tolerant_loader_keeps_valid_languages_when_regex_is_invalid() {
		let (mut registry, root) = registry();
		registry.insert(rust_spec("rust")).expect("rust should insert");
		let mut broken = rust_spec("broken");
		broken.content_regexes.push("[".to_owned());
		registry.insert(broken).expect("broken should insert");

		let report = RegistryLanguageLoader::from_registry_tolerant(&registry);

		assert!(report.loader.language(&LanguageId::new("rust")).is_some());
		assert!(report.loader.language(&LanguageId::new("broken")).is_none());
		assert!(matches!(
			report.issues.as_slice(),
			[RegistryLanguageLoaderError::InvalidRegex { language, .. }] if language == &LanguageId::new("broken")
		));
		fs::remove_dir_all(root).expect("temp root should be removed");
	}

	#[test]
	fn tolerant_loader_skips_later_duplicate_injection_name() {
		let (mut registry, root) = registry();
		let mut rust = rust_spec("rust");
		rust.injection_names.push("rs".to_owned());
		registry.insert(rust).expect("rust should insert");
		let mut second = rust_spec("second");
		second.injection_names.push("rs".to_owned());
		registry.insert(second).expect("second should insert");

		let report = RegistryLanguageLoader::from_registry_tolerant(&registry);

		assert!(report.loader.language(&LanguageId::new("rust")).is_some());
		assert!(report.loader.language(&LanguageId::new("second")).is_none());
		assert!(matches!(
			report.issues.as_slice(),
			[RegistryLanguageLoaderError::DuplicateInjectionName { second, .. }]
				if second == &LanguageId::new("second")
		));
		fs::remove_dir_all(root).expect("temp root should be removed");
	}

	#[test]
	fn strict_loader_still_fails_fast() {
		let (mut registry, root) = registry();
		let mut broken = rust_spec("broken");
		broken.content_regexes.push("[".to_owned());
		registry.insert(broken).expect("broken should insert");

		let error = RegistryLanguageLoader::from_registry(&registry).expect_err("strict load should fail");

		assert!(matches!(error, RegistryLanguageLoaderError::InvalidRegex { .. }));
		fs::remove_dir_all(root).expect("temp root should be removed");
	}
}
