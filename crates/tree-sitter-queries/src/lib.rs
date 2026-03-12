#![forbid(unsafe_code)]
#![deny(clippy::print_stderr)]

//! Structural tree-sitter query helpers built on top of `liney_tree_house`.

use {
	liney_tree_house::{
		TREE_SITTER_MATCH_LIMIT,
		tree_sitter::{
			Grammar, InactiveQueryCursor, Node, Query, RopeInput,
			query::{InvalidPredicateError, UserPredicate},
		},
	},
	ropey::RopeSlice,
	std::iter,
};

pub use liney_tree_house::read_query;

/// Query for computing indentation.
#[derive(Debug)]
#[allow(dead_code, reason = "captures reserved for future indentation features")]
pub struct IndentQuery {
	query: Query,
	indent_capture: Option<liney_tree_house::tree_sitter::Capture>,
	dedent_capture: Option<liney_tree_house::tree_sitter::Capture>,
	extend_capture: Option<liney_tree_house::tree_sitter::Capture>,
}

impl IndentQuery {
	pub fn new(grammar: Grammar, source: &str) -> Result<Self, liney_tree_house::tree_sitter::query::ParseError> {
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
	pub fn new(grammar: Grammar, source: &str) -> Result<Self, liney_tree_house::tree_sitter::query::ParseError> {
		let query = Query::new(grammar, source, |_, _| Ok(()))?;
		Ok(Self { query })
	}

	pub fn capture_nodes<'a>(
		&'a self, capture_name: &str, node: &Node<'a>, source: RopeSlice<'a>,
	) -> Option<impl Iterator<Item = CapturedNode<'a>>> {
		let capture = self.query.get_capture(capture_name)?;

		let mut cursor = InactiveQueryCursor::new(0..u32::MAX, TREE_SITTER_MATCH_LIMIT).execute_query(
			&self.query,
			node,
			RopeInput::new(source),
		);

		let capture_node = iter::from_fn(move || {
			let mat = cursor.next_match()?;
			Some(mat.nodes_for_capture(capture).cloned().collect())
		})
		.filter_map(|nodes: Vec<_>| {
			if nodes.len() > 1 {
				Some(CapturedNode::Grouped(nodes))
			} else {
				nodes.into_iter().map(CapturedNode::Single).next()
			}
		});

		Some(capture_node)
	}

	pub fn capture_nodes_any<'a>(
		&'a self, capture_names: &[&str], node: &Node<'a>, source: RopeSlice<'a>,
	) -> Option<impl Iterator<Item = CapturedNode<'a>>> {
		let capture = capture_names.iter().find_map(|name| self.query.get_capture(name))?;

		let mut cursor = InactiveQueryCursor::new(0..u32::MAX, TREE_SITTER_MATCH_LIMIT).execute_query(
			&self.query,
			node,
			RopeInput::new(source),
		);

		let capture_node = iter::from_fn(move || {
			let mat = cursor.next_match()?;
			Some(mat.nodes_for_capture(capture).cloned().collect())
		})
		.filter_map(|nodes: Vec<_>| {
			if nodes.len() > 1 {
				Some(CapturedNode::Grouped(nodes))
			} else {
				nodes.into_iter().map(CapturedNode::Single).next()
			}
		});

		Some(capture_node)
	}
}

/// A captured node or group of nodes from a text object query.
#[derive(Debug)]
pub enum CapturedNode<'a> {
	Single(Node<'a>),
	Grouped(Vec<Node<'a>>),
}

impl CapturedNode<'_> {
	pub fn start_byte(&self) -> usize {
		match self {
			Self::Single(node) => node.start_byte() as usize,
			Self::Grouped(nodes) => nodes[0].start_byte() as usize,
		}
	}

	pub fn end_byte(&self) -> usize {
		match self {
			Self::Single(node) => node.end_byte() as usize,
			Self::Grouped(nodes) => nodes.last().unwrap().end_byte() as usize,
		}
	}

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
	pub fn new(grammar: Grammar, source: &str) -> Result<Self, liney_tree_house::tree_sitter::query::ParseError> {
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
}

/// Query for rainbow bracket highlighting.
#[derive(Debug)]
pub struct RainbowQuery {
	pub query: Query,
	pub scope_capture: Option<liney_tree_house::tree_sitter::Capture>,
	pub bracket_capture: Option<liney_tree_house::tree_sitter::Capture>,
}

impl RainbowQuery {
	pub fn new(grammar: Grammar, source: &str) -> Result<Self, liney_tree_house::tree_sitter::query::ParseError> {
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
}
