use {
	crate::{
		Language,
		highlighter::{Highlight, HighlightQuery},
		injections_query::{InjectionLanguageMarker, InjectionsQuery},
	},
	regex::Regex,
	std::{fmt::Write, sync::LazyLock},
	tree_sitter::{Grammar, query},
};

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

/// reads a query by invoking `read_query_text`, handles any `inherits` directives
pub fn read_query(language: &str, mut read_query_text: impl FnMut(&str) -> String) -> String {
	fn read_query_impl(language: &str, read_query_text: &mut impl FnMut(&str) -> String) -> String {
		let query = read_query_text(language);

		// replaces all "; inherits <language>(,<language>)*" with the queries of the given language(s)
		INHERITS_REGEX
			.replace_all(&query, |captures: &regex::Captures| {
				captures[1].split(',').fold(String::new(), |mut output, language| {
					// `write!` to a String cannot fail.
					write!(output, "\n{}\n", read_query_impl(language, &mut *read_query_text)).unwrap();
					output
				})
			})
			.into_owned()
	}
	read_query_impl(language, &mut read_query_text)
}

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
