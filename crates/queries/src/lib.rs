#![forbid(unsafe_code)]
#![deny(clippy::print_stderr)]

//! Structural tree-sitter query helpers built on top of `glorp_syntax_tree`.

/// Re-export of [`glorp_syntax_tree::read_query`] for query-file resolution.
pub use glorp_syntax_tree::read_query;
use glorp_syntax_tree::{
	DocumentSnapshot,
	tree_sitter::{
		Grammar, Node, Query,
		query::{InvalidPredicateError, UserPredicate},
	},
};

/// Query for computing indentation.
#[derive(Debug)]
#[allow(dead_code, reason = "captures reserved for future indentation features")]
pub struct IndentQuery {
	query: Query,
	indent_capture: Option<glorp_syntax_tree::tree_sitter::Capture>,
	dedent_capture: Option<glorp_syntax_tree::tree_sitter::Capture>,
	extend_capture: Option<glorp_syntax_tree::tree_sitter::Capture>,
}

impl IndentQuery {
	/// Parses an indentation query for `grammar`.
	pub fn new(grammar: Grammar, source: &str) -> Result<Self, glorp_syntax_tree::tree_sitter::query::ParseError> {
		let query = Query::new(grammar, source, |_pattern, predicate| match predicate {
			UserPredicate::SetProperty {
				key:
					"indent.begin" | "indent.end" | "indent.dedent" | "indent.branch" | "indent.ignore" | "indent.align",
				..
			} => Ok(()),
			_ => Err(InvalidPredicateError::unknown(predicate)),
		})?;

		Ok(Self {
			indent_capture: query.get_capture("indent"),
			dedent_capture: query.get_capture("dedent"),
			extend_capture: query.get_capture("extend"),
			query,
		})
	}

	/// Returns the underlying tree-sitter query.
	pub fn query(&self) -> &Query {
		&self.query
	}
}

/// Query for text object selection.
#[derive(Debug)]
pub struct TextObjectQuery {
	query: Query,
}

impl TextObjectQuery {
	/// Parses a text-object query for `grammar`.
	pub fn new(grammar: Grammar, source: &str) -> Result<Self, glorp_syntax_tree::tree_sitter::query::ParseError> {
		let query = Query::new(grammar, source, |_, _| Ok(()))?;
		Ok(Self { query })
	}

	/// Streams nodes captured under `capture_name`.
	pub fn capture_nodes<'a>(
		&'a self, capture_name: &str, snapshot: &'a DocumentSnapshot,
	) -> Option<impl Iterator<Item = CapturedNode<'a>>> {
		let capture = self.query.get_capture(capture_name)?;
		let root = snapshot.root_node();
		Some(
			snapshot
				.capture_matches(&self.query, capture, &root)
				.filter_map(CapturedNode::from_nodes),
		)
	}

	/// Streams nodes for the first capture name present in `capture_names`.
	pub fn capture_nodes_any<'a>(
		&'a self, capture_names: &[&str], snapshot: &'a DocumentSnapshot,
	) -> Option<impl Iterator<Item = CapturedNode<'a>>> {
		let capture = capture_names.iter().find_map(|name| self.query.get_capture(name))?;
		let root = snapshot.root_node();
		Some(
			snapshot
				.capture_matches(&self.query, capture, &root)
				.filter_map(CapturedNode::from_nodes),
		)
	}
}

/// A captured node or group of nodes from a text object query.
#[derive(Debug)]
pub enum CapturedNode<'a> {
	Single(Node<'a>),
	Grouped(Vec<Node<'a>>),
}

impl<'a> CapturedNode<'a> {
	fn from_nodes(nodes: Vec<Node<'a>>) -> Option<Self> {
		if nodes.len() > 1 {
			Some(Self::Grouped(nodes))
		} else {
			nodes.into_iter().next().map(Self::Single)
		}
	}

	/// Returns the first byte covered by the captured node group.
	pub fn start_byte(&self) -> usize {
		match self {
			Self::Single(node) => node.start_byte() as usize,
			Self::Grouped(nodes) => nodes[0].start_byte() as usize,
		}
	}

	/// Returns the exclusive end byte covered by the captured node group.
	pub fn end_byte(&self) -> usize {
		match self {
			Self::Single(node) => node.end_byte() as usize,
			Self::Grouped(nodes) => nodes.last().unwrap().end_byte() as usize,
		}
	}

	/// Returns the covered byte range.
	pub fn byte_range(&self) -> std::ops::Range<usize> {
		self.start_byte()..self.end_byte()
	}
}

/// Query for symbol tags.
#[derive(Debug)]
pub struct TagQuery {
	pub query: Query,
}

impl TagQuery {
	/// Parses a tag query for `grammar`.
	pub fn new(grammar: Grammar, source: &str) -> Result<Self, glorp_syntax_tree::tree_sitter::query::ParseError> {
		let query = Query::new(grammar, source, |_pattern, predicate| match predicate {
			UserPredicate::IsPropertySet { key: "local", .. } => Ok(()),
			UserPredicate::Other(pred) => match pred.name() {
				"strip!" | "select-adjacent!" => Ok(()),
				_ => Err(InvalidPredicateError::unknown(predicate)),
			},
			_ => Err(InvalidPredicateError::unknown(predicate)),
		})?;

		Ok(Self { query })
	}

	/// Streams nodes captured under `capture_name`.
	pub fn capture_nodes<'a>(
		&'a self, capture_name: &str, snapshot: &'a DocumentSnapshot,
	) -> Option<impl Iterator<Item = Node<'a>>> {
		let capture = self.query.get_capture(capture_name)?;
		let root = snapshot.root_node();
		Some(
			snapshot
				.capture_matches(&self.query, capture, &root)
				.flat_map(|nodes| nodes.into_iter()),
		)
	}
}

/// Query for rainbow bracket highlighting.
#[derive(Debug)]
pub struct RainbowQuery {
	pub query: Query,
	pub scope_capture: Option<glorp_syntax_tree::tree_sitter::Capture>,
	pub bracket_capture: Option<glorp_syntax_tree::tree_sitter::Capture>,
}

impl RainbowQuery {
	/// Parses a rainbow-bracket query for `grammar`.
	pub fn new(grammar: Grammar, source: &str) -> Result<Self, glorp_syntax_tree::tree_sitter::query::ParseError> {
		let query = Query::new(grammar, source, |_pattern, predicate| match predicate {
			UserPredicate::SetProperty {
				key: "rainbow.include-children",
				val,
			} => {
				if val.is_some() {
					return Err("property 'rainbow.include-children' does not take an argument".into());
				}
				Ok(())
			}
			_ => Err(InvalidPredicateError::unknown(predicate)),
		})?;

		Ok(Self {
			scope_capture: query.get_capture("rainbow.scope"),
			bracket_capture: query.get_capture("rainbow.bracket"),
			query,
		})
	}

	/// Streams nodes captured under `capture_name`.
	pub fn capture_nodes<'a>(
		&'a self, capture_name: &str, snapshot: &'a DocumentSnapshot,
	) -> Option<impl Iterator<Item = Node<'a>>> {
		let capture = self.query.get_capture(capture_name)?;
		let root = snapshot.root_node();
		Some(
			snapshot
				.capture_matches(&self.query, capture, &root)
				.flat_map(|nodes| nodes.into_iter()),
		)
	}

	/// Streams nodes captured as rainbow brackets.
	pub fn bracket_nodes<'a>(&'a self, snapshot: &'a DocumentSnapshot) -> Option<impl Iterator<Item = Node<'a>>> {
		let capture = self.bracket_capture?;
		let root = snapshot.root_node();
		Some(
			snapshot
				.capture_matches(&self.query, capture, &root)
				.flat_map(|nodes| nodes.into_iter()),
		)
	}

	/// Streams nodes captured as rainbow scopes.
	pub fn scope_nodes<'a>(&'a self, snapshot: &'a DocumentSnapshot) -> Option<impl Iterator<Item = Node<'a>>> {
		let capture = self.scope_capture?;
		let root = snapshot.root_node();
		Some(
			snapshot
				.capture_matches(&self.query, capture, &root)
				.flat_map(|nodes| nodes.into_iter()),
		)
	}
}

#[cfg(test)]
mod tests {
	use {
		super::*,
		glorp_syntax_tree::{DocumentSession, DocumentSnapshot, EngineConfig, SingleLanguageLoader, StringText},
		std::error::Error,
	};

	const SOURCE: &str = r#"fn alpha() {}

fn beta(arg: i32) -> i32 {
    alpha();
    arg + 1
}
"#;

	const TAG_QUERY: &str = r#"
(function_item
  name: (identifier) @name) @definition.function
"#;

	const RAINBOW_QUERY: &str = r#"
[
  "{"
  "}"
  "("
  ")"
] @rainbow.bracket

[
  (block)
  (parameters)
  (arguments)
] @rainbow.scope
"#;

	fn root() -> Result<(DocumentSnapshot, SingleLanguageLoader), Box<dyn Error>> {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE)?;
		let loader = SingleLanguageLoader::from_queries(grammar, "", "", "")?;
		let session = DocumentSession::new(
			loader.language(),
			&StringText::new(SOURCE),
			&loader,
			EngineConfig::default(),
		)?;
		Ok((session.snapshot(), loader))
	}

	#[test]
	fn tag_query_runs_without_manual_cursor_plumbing() -> Result<(), Box<dyn Error>> {
		let (snapshot, loader) = root()?;
		let query = TagQuery::new(loader.grammar(), TAG_QUERY)?;
		let names: Vec<_> = query
			.capture_nodes("name", &snapshot)
			.expect("name capture should exist")
			.map(|node| SOURCE[node.start_byte() as usize..node.end_byte() as usize].to_owned())
			.collect();

		assert_eq!(names, vec!["alpha".to_owned(), "beta".to_owned()]);
		Ok(())
	}

	#[test]
	fn rainbow_query_exposes_bracket_runner() -> Result<(), Box<dyn Error>> {
		let (snapshot, loader) = root()?;
		let query = RainbowQuery::new(loader.grammar(), RAINBOW_QUERY)?;
		let brackets = query
			.bracket_nodes(&snapshot)
			.expect("bracket capture should exist")
			.count();
		let scopes = query
			.scope_nodes(&snapshot)
			.expect("scope capture should exist")
			.count();

		assert!(brackets >= 6);
		assert!(scopes >= 3);
		Ok(())
	}
}
