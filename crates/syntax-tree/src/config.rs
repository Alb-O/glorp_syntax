use {
	crate::{
		Language,
		highlighter::{Highlight, HighlightQuery},
		injections_query::{InjectionLanguageMarker, InjectionsQuery},
	},
	regex::Regex,
	std::{fmt, sync::LazyLock},
	tree_sitter::{Grammar, query},
};

/// Parsed tree-sitter grammar and queries for one logical language.
#[derive(Debug)]
pub struct LanguageConfig {
	pub grammar: Grammar,
	pub highlight_query: HighlightQuery,
	pub injection_query: InjectionsQuery,
}

impl LanguageConfig {
	pub fn new(
		grammar: Grammar, highlight_query_text: &str, injection_query_text: &str, local_query_text: &str,
	) -> Result<Self, query::ParseError> {
		// NOTE: the injection queries are parsed first since the local query is passed as-is
		// to `Query::new` in `InjectionsQuery::new`. This ensures that the more readable error
		// bubbles up first if the locals queries have an issue.
		let injection_query = InjectionsQuery::new(grammar, injection_query_text, local_query_text)?;
		let highlight_query = HighlightQuery::new(grammar, highlight_query_text, local_query_text)?;

		Ok(Self {
			grammar,
			highlight_query,
			injection_query,
		})
	}

	pub fn configure(&self, mut f: impl FnMut(&str) -> Option<Highlight>) {
		self.highlight_query.configure(&mut f);
		self.injection_query.configure(&mut f);
	}
}

/// Minimal [`LanguageLoader`] implementation for a single language.
#[derive(Debug)]
pub struct SingleLanguageLoader {
	language: Language,
	config: LanguageConfig,
}

impl SingleLanguageLoader {
	pub fn new(language: Language, config: LanguageConfig) -> Self {
		Self { language, config }
	}

	pub fn from_queries(
		language: Language, grammar: Grammar, highlight_query_text: &str, injection_query_text: &str,
		local_query_text: &str,
	) -> Result<Self, query::ParseError> {
		let config = LanguageConfig::new(grammar, highlight_query_text, injection_query_text, local_query_text)?;
		Ok(Self::new(language, config))
	}

	pub fn with_highlights(
		language: Language, grammar: Grammar, highlight_query_text: &str, injection_query_text: &str,
		local_query_text: &str, configure: impl FnMut(&str) -> Option<Highlight>,
	) -> Result<Self, query::ParseError> {
		let loader = Self::from_queries(
			language,
			grammar,
			highlight_query_text,
			injection_query_text,
			local_query_text,
		)?;
		loader.configure(configure);
		Ok(loader)
	}

	pub fn language(&self) -> Language {
		self.language
	}

	pub fn grammar(&self) -> Grammar {
		self.config.grammar
	}

	pub fn config(&self) -> &LanguageConfig {
		&self.config
	}

	pub fn configure(&self, f: impl FnMut(&str) -> Option<Highlight>) {
		self.config.configure(f);
	}
}

static INHERITS_REGEX: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r";+\s*inherits\s*:?\s*([a-z_,()-]+)\s*").unwrap());

/// Errors produced while resolving a query and expanding `; inherits` directives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadQueryError<E> {
	Read { language: Box<str>, source: E },
	Cycle { chain: Vec<Box<str>> },
}

impl<E> fmt::Display for ReadQueryError<E>
where
	E: fmt::Display,
{
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Read { language, source } => write!(f, "failed to read query for language {language}: {source}"),
			Self::Cycle { chain } => {
				let mut chain = chain.iter();
				if let Some(first) = chain.next() {
					f.write_str("cyclic query inherits: ")?;
					f.write_str(first)?;
				}
				for language in chain {
					write!(f, " -> {language}")?;
				}
				Ok(())
			}
		}
	}
}

impl<E> std::error::Error for ReadQueryError<E> where E: std::error::Error + 'static {}

/// Reads a query by invoking `read_query_text`, handling `inherits` directives recursively.
pub fn read_query<E>(
	language: &str, mut read_query_text: impl FnMut(&str) -> Result<String, E>,
) -> Result<String, ReadQueryError<E>> {
	fn read_query_impl<E>(
		language: &str, read_query_text: &mut impl FnMut(&str) -> Result<String, E>, stack: &mut Vec<Box<str>>,
	) -> Result<String, ReadQueryError<E>> {
		if let Some(pos) = stack.iter().position(|current| current.as_ref() == language) {
			let mut chain = stack[pos..].to_vec();
			chain.push(language.into());
			return Err(ReadQueryError::Cycle { chain });
		}

		stack.push(language.into());
		let result = (|| {
			let query = read_query_text(language).map_err(|source| ReadQueryError::Read {
				language: language.into(),
				source,
			})?;
			let mut output = String::with_capacity(query.len());
			let mut offset = 0;
			for captures in INHERITS_REGEX.captures_iter(&query) {
				let matched = captures.get(0).expect("inherits capture should include full match");
				output.push_str(&query[offset..matched.start()]);
				for language in captures[1].split(',') {
					output.push('\n');
					output.push_str(&read_query_impl(language, read_query_text, stack)?);
					output.push('\n');
				}
				offset = matched.end();
			}
			output.push_str(&query[offset..]);
			Ok(output)
		})();
		stack.pop();
		result
	}

	read_query_impl(language, &mut read_query_text, &mut Vec::new())
}

/// Resolves syntax configuration for the root language and injected languages.
pub trait LanguageLoader {
	fn language_for_marker(&self, marker: InjectionLanguageMarker) -> Option<Language>;
	fn get_config(&self, lang: Language) -> Option<&LanguageConfig>;
}

impl<T> LanguageLoader for &'_ T
where
	T: LanguageLoader,
{
	fn language_for_marker(&self, marker: InjectionLanguageMarker) -> Option<Language> {
		T::language_for_marker(self, marker)
	}

	fn get_config(&self, lang: Language) -> Option<&LanguageConfig> {
		T::get_config(self, lang)
	}
}

impl LanguageLoader for SingleLanguageLoader {
	fn language_for_marker(&self, _marker: InjectionLanguageMarker) -> Option<Language> {
		Some(self.language)
	}

	fn get_config(&self, lang: Language) -> Option<&LanguageConfig> {
		(lang == self.language).then_some(&self.config)
	}
}
