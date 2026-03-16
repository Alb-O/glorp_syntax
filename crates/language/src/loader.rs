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

impl RegistryLanguageLoader {
	/// Loads grammars, queries, and injection matchers for every registry entry.
	///
	/// The resulting [`Language`] values are scoped to this loader instance and
	/// are assigned in the deterministic iteration order of the registry.
	pub fn from_registry(registry: &LanguageRegistry) -> Result<Self, RegistryLanguageLoaderError> {
		let language_count = registry.iter().size_hint().0;
		let mut languages = Vec::new();
		let mut languages_by_id = BTreeMap::new();
		let mut exact_names = HashMap::with_capacity(language_count);
		let mut content_regexes = Vec::with_capacity(language_count);
		let mut filename_regexes = Vec::with_capacity(language_count);
		let mut shebang_regexes = Vec::with_capacity(language_count);

		for (idx, (id, spec)) in registry.iter().enumerate() {
			// `Language` is a loader-local token, so deterministic registry order is enough.
			let language = Language::from_raw(idx as u32);
			languages_by_id.insert(id.clone(), language);

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
			// Missing optional query files are treated as empty query sources.
			let config = LanguageConfig::new(grammar, &highlights, &injections, &locals).map_err(|source| {
				RegistryLanguageLoaderError::Parse {
					language: id.clone(),
					source: Box::new(source),
				}
			})?;
			languages.push(LoadedLanguage { id: id.clone(), config });

			register_exact_name(&mut exact_names, id.as_str(), language, id)?;
			for alias in &spec.injection_names {
				register_exact_name(&mut exact_names, alias, language, id)?;
			}
			compile_regexes(&mut content_regexes, language, id, "content", &spec.content_regexes)?;
			compile_regexes(&mut filename_regexes, language, id, "filename", &spec.filename_regexes)?;
			compile_regexes(&mut shebang_regexes, language, id, "shebang", &spec.shebang_regexes)?;
		}

		Ok(Self {
			languages,
			languages_by_id,
			exact_names,
			content_regexes,
			filename_regexes,
			shebang_regexes,
		})
	}

	/// Returns the loader-scoped engine language ID for a registry language ID.
	pub fn language(&self, id: &LanguageId) -> Option<Language> {
		self.languages_by_id.get(id).copied()
	}

	/// Returns the registry language ID for a loader-scoped engine language ID.
	pub fn language_id(&self, language: Language) -> Option<&LanguageId> {
		self.languages.get(language.idx()).map(|loaded| &loaded.id)
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

fn register_exact_name(
	exact_names: &mut HashMap<Box<str>, ExactNameMatcher>, name: &str, language: Language, current: &LanguageId,
) -> Result<(), RegistryLanguageLoaderError> {
	match exact_names.get(name) {
		Some(existing) if existing.language != language => Err(RegistryLanguageLoaderError::DuplicateInjectionName {
			name: name.to_owned(),
			first: existing.owner.clone(),
			second: current.clone(),
		}),
		Some(_) => Ok(()),
		None => {
			exact_names.insert(
				name.into(),
				ExactNameMatcher {
					language,
					owner: current.clone(),
				},
			);
			Ok(())
		}
	}
}

fn compile_regexes(
	dst: &mut Vec<RegexMatcher>, language: Language, id: &LanguageId, matcher_kind: &'static str, patterns: &[String],
) -> Result<(), RegistryLanguageLoaderError> {
	for pattern in patterns {
		let regex = Regex::new(pattern).map_err(|source| RegistryLanguageLoaderError::InvalidRegex {
			language: id.clone(),
			matcher_kind,
			pattern: pattern.clone(),
			source,
		})?;
		dst.push(RegexMatcher { language, regex });
	}
	Ok(())
}

fn longest_regex_match(matchers: &[RegexMatcher], text: &str) -> Option<Language> {
	let mut best = None;
	for matcher in matchers {
		for matched in matcher.regex.find_iter(text) {
			let len = matched.end() - matched.start();
			// Injection markers are intentionally resolved by the most specific regex match,
			// not by registration order.
			if best.is_none_or(|(best_len, _)| len > best_len) {
				best = Some((len, matcher.language));
			}
		}
	}
	best.map(|(_, language)| language)
}

#[cfg(test)]
mod tests {
	use {super::*, glorp_syntax_tree::tree_sitter::Grammar, ropey::Rope};

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
}
