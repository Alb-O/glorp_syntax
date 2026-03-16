use {
	crate::{
		Injection, Language, Layer, LayerData, Range, Syntax, TREE_SITTER_MATCH_LIMIT,
		config::{LanguageConfig, LanguageLoader},
		highlighter::Highlight,
		locals::Locals,
		parse::LayerUpdateFlags,
	},
	arc_swap::ArcSwap,
	regex_cursor::engines::meta::Regex,
	ropey::RopeSlice,
	std::{
		cmp::Reverse,
		collections::HashMap,
		iter::{self, Peekable},
		mem::take,
		sync::{Arc, LazyLock},
	},
	tree_sitter::{
		Capture, Grammar, InactiveQueryCursor, MatchedNodeIdx, Node, Pattern, Query, QueryMatch,
		query::{self, InvalidPredicateError, UserPredicate},
	},
};

const SHEBANG: &str = r"#!\s*(?:\S*[/\\](?:env\s+(?:\-\S+\s+)*)?)?([^\s\.\d]+)";
static SHEBANG_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(SHEBANG).unwrap());

#[derive(Clone, Default, Debug)]
pub struct InjectionProperties {
	include_children: IncludedChildren,
	language: Option<Box<str>>,
	combined: bool,
}

/// An indicator in the document or query source file which used by the loader to know which
/// language an injection should use.
///
/// For example if a query sets a property `(#set! injection.language "rust")` then the loader
/// should load the Rust language. Alternatively the loader might be asked to load a language
/// based on some text in the document, for example a markdown code fence language name.
#[derive(Debug, Clone, Copy)]
pub enum InjectionLanguageMarker<'a> {
	/// The language is specified by name in the injection query itself.
	///
	/// For example `(#set! injection.language "rust")`. These names should match exactly and so
	/// they can be looked up by equality - very efficiently.
	Name(&'a str),
	/// The language is specified by name - or similar - within the parsed document.
	///
	/// This is slightly different than the `ExactName` variant: within a document you might
	/// specify Markdown as "md" or "markdown" for example. The loader should look up the language
	/// name by longest matching regex.
	Match(RopeSlice<'a>),
	Filename(RopeSlice<'a>),
	Shebang(RopeSlice<'a>),
}

#[derive(Clone, Debug)]
pub struct InjectionQueryMatch<'tree> {
	include_children: IncludedChildren,
	language: Language,
	scope: Option<InjectionScope>,
	node: Node<'tree>,
	last_match: bool,
	pattern: Pattern,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum InjectionScope {
	Match { id: u32 },
	Pattern { pattern: Pattern, language: Language },
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum IncludedChildren {
	#[default]
	None,
	All,
	Unnamed,
}

#[derive(Debug)]
pub struct InjectionsQuery {
	injection_query: Query,
	injection_properties: Vec<InjectionProperties>,
	injection_content_capture: Option<Capture>,
	injection_language_capture: Option<Capture>,
	injection_filename_capture: Option<Capture>,
	injection_shebang_capture: Option<Capture>,
	// Note that the injections query is concatenated with the locals query.
	pub(crate) local_query: Query,
	pub(crate) not_scope_inherits: Vec<bool>,
	pub(crate) local_scope_capture: Option<Capture>,
	pub(crate) local_definition_captures: ArcSwap<Vec<Option<Highlight>>>,
}

impl InjectionsQuery {
	pub fn new(
		grammar: Grammar, injection_query_text: &str, local_query_text: &str,
	) -> Result<Self, query::ParseError> {
		let mut injection_properties = Vec::new();
		let mut not_scope_inherits = Vec::new();
		let injection_query = Query::new(grammar, injection_query_text, |pattern, predicate| {
			match predicate {
				// injections
				UserPredicate::SetProperty {
					key: "injection.include-unnamed-children",
					val: None,
				} => {
					pattern_properties_mut(&mut injection_properties, pattern).include_children =
						IncludedChildren::Unnamed
				}
				UserPredicate::SetProperty {
					key: "injection.include-children",
					val: None,
				} => pattern_properties_mut(&mut injection_properties, pattern).include_children = IncludedChildren::All,
				UserPredicate::SetProperty {
					key: "injection.language",
					val: Some(lang),
				} => pattern_properties_mut(&mut injection_properties, pattern).language = Some(lang.into()),
				UserPredicate::SetProperty {
					key: "injection.combined",
					val: None,
				} => pattern_properties_mut(&mut injection_properties, pattern).combined = true,
				predicate => {
					return Err(InvalidPredicateError::unknown(predicate));
				}
			}
			Ok(())
		})?;
		let mut local_query = Query::new(grammar, local_query_text, |pattern, predicate| {
			match predicate {
				UserPredicate::SetProperty {
					key: "local.scope-inherits",
					val,
				} => {
					if val.is_some_and(|val| val != "true") {
						set_pattern_flag(&mut not_scope_inherits, pattern);
					}
				}
				predicate => {
					return Err(InvalidPredicateError::unknown(predicate));
				}
			}
			Ok(())
		})?;

		// The injection queries do not track references - these are read by the highlight
		// query instead.
		local_query.disable_capture("local.reference");

		Ok(InjectionsQuery {
			injection_properties,
			injection_content_capture: injection_query.get_capture("injection.content"),
			injection_language_capture: injection_query.get_capture("injection.language"),
			injection_filename_capture: injection_query.get_capture("injection.filename"),
			injection_shebang_capture: injection_query.get_capture("injection.shebang"),
			injection_query,
			not_scope_inherits,
			local_scope_capture: local_query.get_capture("local.scope"),
			local_definition_captures: ArcSwap::from_pointee(vec![None; local_query.num_captures() as usize]),
			local_query,
		})
	}

	pub(crate) fn configure(&self, f: &mut impl FnMut(&str) -> Option<Highlight>) {
		let mut local_definition_captures = vec![None; self.local_query.num_captures() as usize];
		for (capture, name) in self.local_query.captures() {
			let Some(suffix) = name.strip_prefix("local.definition.") else {
				continue;
			};
			let Some(highlight) = f(suffix) else {
				continue;
			};
			local_definition_captures[capture.idx()] = Some(highlight);
		}
		self.local_definition_captures
			.store(Arc::new(local_definition_captures));
	}

	fn process_match<'a, 'tree>(
		&self, query_match: &QueryMatch<'a, 'tree>, node_idx: MatchedNodeIdx, source: RopeSlice<'a>,
		loader: impl LanguageLoader,
	) -> Option<InjectionQueryMatch<'tree>> {
		let properties = self.injection_properties.get(query_match.pattern().idx());

		let mut marker = None;
		let mut last_content_node = 0;
		let mut content_nodes = 0;
		for (i, matched_node) in query_match.matched_nodes().enumerate() {
			let capture = Some(matched_node.capture);
			if capture == self.injection_language_capture {
				let range = matched_node.node.byte_range();
				marker = Some(InjectionLanguageMarker::Match(
					source.byte_slice(range.start as usize..range.end as usize),
				));
			} else if capture == self.injection_filename_capture {
				let range = matched_node.node.byte_range();
				marker = Some(InjectionLanguageMarker::Filename(
					source.byte_slice(range.start as usize..range.end as usize),
				));
			} else if capture == self.injection_shebang_capture {
				let range = matched_node.node.byte_range();
				let node_slice = source.byte_slice(range.start as usize..range.end as usize);

				// some languages allow space and newlines before the actual string content
				// so a shebang could be on either the first or second line
				let lines = node_slice
					.try_line_to_byte(2)
					.map_or(node_slice, |end| node_slice.byte_slice(..end));

				marker = SHEBANG_REGEX
					.captures_iter(regex_cursor::Input::new(lines))
					.map(|cap| {
						let cap = lines.byte_slice(cap.get_group(1).unwrap().range());
						InjectionLanguageMarker::Shebang(cap)
					})
					.next();
			} else if capture == self.injection_content_capture {
				content_nodes += 1;

				last_content_node = i as u32;
			}
		}
		let marker = marker.or_else(|| {
			properties
				.and_then(|p| p.language.as_deref())
				.map(InjectionLanguageMarker::Name)
		})?;

		let language = loader.language_for_marker(marker)?;
		let scope = if properties.is_some_and(|p| p.combined) {
			Some(InjectionScope::Pattern {
				pattern: query_match.pattern(),
				language,
			})
		} else if content_nodes != 1 {
			Some(InjectionScope::Match { id: query_match.id() })
		} else {
			None
		};

		Some(InjectionQueryMatch {
			language,
			scope,
			include_children: properties.map(|p| p.include_children).unwrap_or_default(),
			node: query_match.matched_node(node_idx).node.clone(),
			last_match: last_content_node == node_idx,
			pattern: query_match.pattern(),
		})
	}

	/// Executes the query on the given input and return an iterator of
	/// injection ranges together with their injection properties
	///
	/// The ranges yielded by the iterator have an ascending start range.
	/// The ranges do not overlap exactly (matches of the exact same node are
	/// resolved with normal precedence rules). However, ranges can be nested.
	/// For example:
	///
	/// ``` no-compile
	///   | range 2 |
	/// |   range 1  |
	/// ```
	/// is possible and will always result in iteration order [range1, range2].
	/// This case should be handled by the calling function
	fn execute<'a>(
		&'a self, node: &Node<'a>, source: RopeSlice<'a>, loader: &'a impl LanguageLoader,
	) -> impl Iterator<Item = InjectionQueryMatch<'a>> + 'a {
		let mut cursor = InactiveQueryCursor::new(0..u32::MAX, TREE_SITTER_MATCH_LIMIT).execute_query(
			&self.injection_query,
			node,
			source,
		);
		let injection_content_capture = self.injection_content_capture.unwrap();
		let iter = iter::from_fn(move || {
			loop {
				let (query_match, node_idx) = cursor.next_matched_node()?;
				if query_match.matched_node(node_idx).capture != injection_content_capture {
					continue;
				}
				let Some(mat) = self.process_match(&query_match, node_idx, source, loader) else {
					query_match.remove();
					continue;
				};
				let range = query_match.matched_node(node_idx).node.byte_range();
				if mat.last_match {
					query_match.remove();
				}
				if range.is_empty() {
					continue;
				}
				break Some(mat);
			}
		});
		let mut buf = Vec::new();
		let mut iter = iter.peekable();
		// handle identical/overlapping matches to correctly account for precedence
		iter::from_fn(move || {
			if let Some(mat) = buf.pop() {
				return Some(mat);
			}
			let mut res = iter.next()?;
			// if children are not included then nested injections don't
			// interfere with each other unless exactly identical. Since
			// this is the default setting we have a fastpath for it
			if res.include_children == IncludedChildren::None {
				let mut fast_return = true;
				while let Some(overlap) = iter.next_if(|mat| mat.node.byte_range() == res.node.byte_range()) {
					if overlap.include_children != IncludedChildren::None {
						buf.push(overlap);
						fast_return = false;
						break;
					}
					// Prefer the last capture which matches this exact node.
					res = overlap;
				}
				if fast_return {
					return Some(res);
				}
			}

			// we if can't use the fastpath we accumulate all overlapping matches
			// and then sort them according to precedence rules...
			while let Some(overlap) = iter.next_if(|mat| mat.node.end_byte() <= res.node.end_byte()) {
				buf.push(overlap)
			}
			if buf.is_empty() {
				return Some(res);
			}
			buf.push(res);
			buf.sort_unstable_by_key(|mat| (mat.pattern, Reverse(mat.node.start_byte())));
			buf.pop()
		})
	}
}

fn pattern_properties_mut(properties: &mut Vec<InjectionProperties>, pattern: Pattern) -> &mut InjectionProperties {
	let idx = pattern.idx();
	if idx >= properties.len() {
		properties.resize_with(idx + 1, InjectionProperties::default);
	}
	&mut properties[idx]
}

fn set_pattern_flag(flags: &mut Vec<bool>, pattern: Pattern) {
	let idx = pattern.idx();
	if idx >= flags.len() {
		flags.resize(idx + 1, false);
	}
	flags[idx] = true;
}

#[derive(Debug)]
struct InjectionCandidate {
	language: Language,
	pattern: Pattern,
	scope: Option<InjectionScope>,
	last_match: bool,
	matched_node_range: Range,
	emitted_ranges: Vec<Range>,
}

#[derive(Debug)]
struct MappedOldInjection {
	layer: Layer,
	range: Range,
	matched_node_range: Range,
	language: Language,
	pattern: Option<Pattern>,
	moved: bool,
	modified: bool,
}

#[derive(Debug, Clone, Copy)]
enum PlannedLayerSource {
	Reuse(Layer),
	New,
}

#[derive(Debug)]
struct PlannedLayerAction {
	source: PlannedLayerSource,
	seed_layer: Option<Layer>,
	language: Language,
	pattern: Pattern,
	ranges: Vec<Range>,
	moved: bool,
	modified: bool,
}

#[derive(Debug)]
struct PlannedInjection {
	range: Range,
	matched_node_range: Range,
	layer_action: usize,
}

#[derive(Debug)]
struct InjectionPlan {
	layer_actions: Vec<PlannedLayerAction>,
	injections: Vec<PlannedInjection>,
	retired_layers: Vec<Layer>,
}

impl Syntax {
	pub(crate) fn run_injection_query(
		&mut self, layer: Layer, edits: &[tree_sitter::InputEdit], source: RopeSlice<'_>, loader: &impl LanguageLoader,
		mut parse_layer: impl FnMut(Layer),
	) {
		let layer_data = &mut self.layer_mut(layer);
		let Some(LanguageConfig {
			injection_query: injections_query,
			..
		}) = loader.get_config(layer_data.language)
		else {
			return;
		};
		if injections_query.injection_content_capture.is_none() {
			return;
		}

		let parent_ranges = take(&mut layer_data.ranges);
		let parse_tree = layer_data.parse_tree.take().unwrap();
		let old_injections = take(&mut layer_data.injections);
		let query_matches = injections_query.execute(&parse_tree.root_node(), source, loader);
		let candidates = collect_injection_candidates(query_matches, &parent_ranges);
		let plan = self.plan_injections(candidates, old_injections, edits);
		self.apply_injection_plan(layer, parent_ranges, parse_tree, plan, &mut parse_layer);
	}

	fn plan_injections(
		&self, candidates: Vec<InjectionCandidate>, old_injections: Vec<Injection>, edits: &[tree_sitter::InputEdit],
	) -> InjectionPlan {
		// Reuse decisions must be made against the post-edit topology, not the stale pre-edit ranges.
		let mut old_injections = map_old_injections(old_injections, edits, |layer| {
			let layer_data = self.layer(layer);
			(layer_data.language, layer_data.origin_pattern())
		})
		.into_iter()
		.peekable();
		let mut layer_actions = Vec::new();
		let mut injections = Vec::new();
		let mut combined_layers: HashMap<InjectionScope, usize> = HashMap::with_capacity(32);
		let mut retired_layers = Vec::new();

		for candidate in candidates {
			let reused = take_reusable_injection(
				candidate.language,
				candidate.pattern,
				&candidate.matched_node_range,
				&mut old_injections,
				&mut retired_layers,
			);
			let layer_action = match candidate.scope.as_ref() {
				// Combined injections share one child layer across multiple disjoint content ranges.
				Some(scope @ InjectionScope::Match { .. }) if candidate.last_match => combined_layers
					.remove(scope)
					.unwrap_or_else(|| push_layer_action(&mut layer_actions, &candidate, reused.as_ref())),
				Some(scope) => *combined_layers
					.entry(scope.clone())
					.or_insert_with(|| push_layer_action(&mut layer_actions, &candidate, reused.as_ref())),
				None => push_layer_action(&mut layer_actions, &candidate, reused.as_ref()),
			};

			layer_actions[layer_action].apply_candidate(&candidate, reused.as_ref());
			let matched_node_range = candidate.matched_node_range.clone();
			for range in candidate.emitted_ranges {
				injections.push(PlannedInjection {
					range,
					matched_node_range: matched_node_range.clone(),
					layer_action,
				});
			}
		}

		retired_layers.extend(old_injections.map(|old| old.layer));
		injections.sort_unstable_by_key(|injection| injection.range.start);

		InjectionPlan {
			layer_actions,
			injections,
			retired_layers,
		}
	}

	fn apply_injection_plan(
		&mut self, parent: Layer, parent_ranges: Vec<tree_sitter::Range>, parse_tree: tree_sitter::Tree,
		plan: InjectionPlan, parse_layer: &mut impl FnMut(Layer),
	) {
		for retired_layer in plan.retired_layers {
			self.layer_mut(retired_layer).flags.modified = true;
		}

		let mut layer_ids = Vec::with_capacity(plan.layer_actions.len());
		for action in &plan.layer_actions {
			let layer = match action.source {
				PlannedLayerSource::Reuse(layer) => {
					let layer_data = self.layer_mut(layer);
					debug_assert_eq!(layer_data.parent, Some(parent));
					layer_data.language = action.language;
					layer_data.origin_pattern = Some(action.pattern);
					layer_data.ranges.clear();
					layer_data.flags = LayerUpdateFlags {
						reused: true,
						modified: action.modified,
						moved: action.moved,
						touched: true,
					};
					layer
				}
				PlannedLayerSource::New => {
					let parse_tree = action.seed_layer.and_then(|layer| self.layer(layer).tree().cloned());
					let layer = self.layers.insert(LayerData {
						language: action.language,
						parse_tree,
						origin_pattern: Some(action.pattern),
						ranges: Vec::new(),
						injections: Vec::new(),
						flags: LayerUpdateFlags {
							reused: action.seed_layer.is_some(),
							modified: action.modified,
							moved: action.moved,
							touched: true,
						},
						parent: Some(parent),
						locals: Locals::default(),
					});
					Layer(layer as u32)
				}
			};
			layer_ids.push(layer);
			parse_layer(layer);
		}

		for (idx, action) in plan.layer_actions.into_iter().enumerate() {
			// The planner owns final ordering, so apply can write ranges directly without any
			// rotate/realign bookkeeping.
			self.layer_mut(layer_ids[idx]).ranges = action.ranges.into_iter().map(range_to_tree_sitter).collect();
		}

		let injections = plan
			.injections
			.into_iter()
			.map(|injection| Injection {
				range: injection.range,
				layer: layer_ids[injection.layer_action],
				matched_node_range: injection.matched_node_range,
			})
			.collect();
		let layer_data = self.layer_mut(parent);
		layer_data.ranges = parent_ranges;
		layer_data.parse_tree = Some(parse_tree);
		layer_data.injections = injections;
	}
}

impl PlannedLayerAction {
	fn new(candidate: &InjectionCandidate, reused: Option<&MappedOldInjection>) -> Self {
		let (source, moved, modified) = match reused {
			Some(reused) => (PlannedLayerSource::Reuse(reused.layer), reused.moved, reused.modified),
			None => (PlannedLayerSource::New, false, false),
		};
		Self {
			source,
			seed_layer: None,
			language: candidate.language,
			pattern: candidate.pattern,
			ranges: Vec::new(),
			moved,
			modified,
		}
	}

	fn apply_candidate(&mut self, candidate: &InjectionCandidate, reused: Option<&MappedOldInjection>) {
		self.ranges.extend(candidate.emitted_ranges.iter().cloned());
		match self.source {
			PlannedLayerSource::Reuse(layer) => {
				self.modified |= reused.is_none_or(|reused| {
					reused.layer != layer || reused.matched_node_range != candidate.matched_node_range
				});
				if let Some(reused) = reused.filter(|reused| reused.layer == layer) {
					self.moved |= reused.moved;
					self.modified |= reused.modified;
				}
			}
			PlannedLayerSource::New => {
				if let Some(seed_layer) = self.seed_layer {
					self.modified |= reused.is_none_or(|reused| {
						reused.layer != seed_layer || reused.matched_node_range != candidate.matched_node_range
					});
				} else if let Some(reused) = reused {
					self.seed_layer = Some(reused.layer);
					self.modified = true;
				}
			}
		}
	}
}

fn push_layer_action(
	layer_actions: &mut Vec<PlannedLayerAction>, candidate: &InjectionCandidate, reused: Option<&MappedOldInjection>,
) -> usize {
	layer_actions.push(PlannedLayerAction::new(candidate, reused));
	layer_actions.len() - 1
}

fn collect_injection_candidates<'a>(
	query_matches: impl Iterator<Item = InjectionQueryMatch<'a>>, parent_ranges: &[tree_sitter::Range],
) -> Vec<InjectionCandidate> {
	let mut candidates = Vec::new();
	let mut accepted_ranges: Vec<Range> = Vec::new();

	for query_match in query_matches {
		let matched_node_range = query_match.node.byte_range();
		let emitted_ranges = collect_intersected_ranges(query_match.include_children, &query_match.node, parent_ranges);
		if emitted_ranges.is_empty() {
			continue;
		}

		let mut insert_position = accepted_ranges.len();
		if let Some(last_range) = accepted_ranges
			.last()
			.filter(|range| ranges_intersect(range, &matched_node_range))
		{
			// Query precedence can surface overlapping matches out of positional order; insert
			// accepted ranges where they belong and reject anything still overlapped after that.
			if last_range.start <= matched_node_range.start {
				continue;
			}
			insert_position = accepted_ranges.partition_point(|range| range.end <= matched_node_range.start);
			if accepted_ranges
				.get(insert_position)
				.is_some_and(|range| range.start < matched_node_range.end)
			{
				continue;
			}
		}

		let candidate = InjectionCandidate {
			language: query_match.language,
			pattern: query_match.pattern,
			scope: query_match.scope,
			last_match: query_match.last_match,
			matched_node_range,
			emitted_ranges,
		};
		accepted_ranges.splice(
			insert_position..insert_position,
			candidate.emitted_ranges.iter().cloned(),
		);
		candidates.push(candidate);
	}

	debug_assert!(accepted_ranges.windows(2).all(|pair| pair[0].end <= pair[1].start));

	candidates
}

fn collect_intersected_ranges(
	include_children: IncludedChildren, node: &Node<'_>, parent_ranges: &[tree_sitter::Range],
) -> Vec<Range> {
	let mut ranges = Vec::new();
	intersect_ranges(include_children, node, parent_ranges, |range| ranges.push(range));
	ranges
}

fn map_old_injections(
	old_injections: Vec<Injection>, edits: &[tree_sitter::InputEdit],
	mut layer_info: impl FnMut(Layer) -> (Language, Option<Pattern>),
) -> Vec<MappedOldInjection> {
	if edits.is_empty() {
		return old_injections
			.into_iter()
			.map(|old_injection| {
				let (language, pattern) = layer_info(old_injection.layer);
				MappedOldInjection {
					layer: old_injection.layer,
					range: old_injection.range,
					matched_node_range: old_injection.matched_node_range,
					language,
					pattern,
					moved: false,
					modified: false,
				}
			})
			.collect();
	}

	let mut mapped = Vec::with_capacity(old_injections.len());
	let mut offset = 0;
	let mut edits = edits.iter().peekable();

	for old_injection in old_injections {
		let (language, pattern) = layer_info(old_injection.layer);
		mapped.push(map_old_injection(
			old_injection,
			language,
			pattern,
			&mut edits,
			&mut offset,
		));
	}

	mapped
}

fn map_old_injection<'a>(
	old_injection: Injection, language: Language, pattern: Option<Pattern>,
	edits: &mut Peekable<impl Iterator<Item = &'a tree_sitter::InputEdit>>, offset: &mut i32,
) -> MappedOldInjection {
	let Injection {
		layer,
		mut range,
		mut matched_node_range,
	} = old_injection;
	let mut modified = false;

	debug_assert!(matched_node_range.start <= range.start);
	debug_assert!(matched_node_range.end >= range.end);

	while let Some(edit) = edits.next_if(|edit| edit.old_end_byte < matched_node_range.start) {
		*offset += edit.offset();
	}
	let mut mapped_node_start = shift_by_offset(matched_node_range.start, *offset);
	if let Some(edit) = edits.peek().filter(|edit| edit.start_byte <= matched_node_range.start) {
		// If an edit swallows the old start, snap to the replacement boundary tree-sitter will see.
		mapped_node_start = shift_by_offset(edit.new_end_byte, *offset);
	}

	while let Some(edit) = edits.next_if(|edit| edit.old_end_byte < range.start) {
		*offset += edit.offset();
	}
	let mut moved = *offset != 0;
	let mut mapped_start = shift_by_offset(range.start, *offset);
	if let Some(edit) = edits.next_if(|edit| edit.old_end_byte <= range.end) {
		if edit.start_byte < range.start {
			moved = true;
			mapped_start = shift_by_offset(edit.new_end_byte, *offset);
		} else {
			modified = true;
		}
		*offset += edit.offset();
		while let Some(edit) = edits.next_if(|edit| edit.old_end_byte <= range.end) {
			*offset += edit.offset();
		}
	}

	let mut mapped_end = shift_by_offset(range.end, *offset);
	if let Some(edit) = edits.peek().filter(|edit| edit.start_byte <= range.end) {
		modified = true;
		if edit.start_byte < range.start {
			mapped_start = shift_by_offset(edit.new_end_byte, *offset);
			mapped_end = mapped_start;
		}
	}

	let mut mapped_node_end = shift_by_offset(matched_node_range.end, *offset);
	if let Some(edit) = edits.peek().filter(|edit| edit.start_byte <= matched_node_range.end)
		&& edit.start_byte < matched_node_range.start
	{
		mapped_node_start = shift_by_offset(edit.new_end_byte, *offset);
		mapped_node_end = mapped_node_start;
	}

	range = mapped_start..mapped_end;
	matched_node_range = mapped_node_start..mapped_node_end;

	MappedOldInjection {
		layer,
		range,
		matched_node_range,
		language,
		pattern,
		moved,
		modified,
	}
}

fn shift_by_offset(value: u32, offset: i32) -> u32 {
	(value as i32 + offset) as u32
}

fn take_reusable_injection(
	language: Language, pattern: Pattern, new_range: &Range,
	old_injections: &mut Peekable<impl Iterator<Item = MappedOldInjection>>, retired_layers: &mut Vec<Layer>,
) -> Option<MappedOldInjection> {
	while let Some(skipped) = old_injections.next_if(|injection| injection.range.end <= new_range.start) {
		// Skipped layers no longer overlap any future candidate, so they can only retire.
		retired_layers.push(skipped.layer);
	}
	old_injections.next_if(|injection| {
		injection.range.start < new_range.end && injection.language == language && injection.pattern == Some(pattern)
	})
}

fn range_to_tree_sitter(range: Range) -> tree_sitter::Range {
	tree_sitter::Range {
		start_point: tree_sitter::Point::ZERO,
		end_point: tree_sitter::Point::ZERO,
		start_byte: range.start,
		end_byte: range.end,
	}
}

fn intersect_ranges(
	include_children: IncludedChildren, node: &Node<'_>, parent_ranges: &[tree_sitter::Range],
	push_range: impl FnMut(Range),
) {
	let range = node.byte_range();
	let i = parent_ranges.partition_point(|parent_range| parent_range.end_byte <= range.start);
	let parent_ranges = parent_ranges[i..].iter().map(|range| range.start_byte..range.end_byte);
	match include_children {
		IncludedChildren::None => intersect_ranges_impl(
			range,
			node.children().map(|node| node.byte_range()),
			parent_ranges,
			push_range,
		),
		IncludedChildren::All => intersect_ranges_impl(range, [].into_iter(), parent_ranges, push_range),
		IncludedChildren::Unnamed => intersect_ranges_impl(
			range,
			node.children()
				.filter(|node| node.is_named())
				.map(|node| node.byte_range()),
			parent_ranges,
			push_range,
		),
	}
}

fn intersect_ranges_impl(
	range: Range, excluded_ranges: impl Iterator<Item = Range>, parent_ranges: impl Iterator<Item = Range>,
	mut push_range: impl FnMut(Range),
) {
	let mut start = range.start;
	let mut excluded_ranges = excluded_ranges.filter(|range| !range.is_empty()).peekable();
	let mut parent_ranges = parent_ranges.peekable();
	loop {
		let Some(parent_range) = parent_ranges.peek() else {
			return;
		};
		let parent_end = parent_range.end;
		if let Some(excluded_range) = excluded_ranges.next_if(|range| range.start <= parent_end) {
			if excluded_range.start >= range.end {
				break;
			}
			if start != excluded_range.start {
				push_range(start..excluded_range.start)
			}
			start = excluded_range.end;
		} else {
			parent_ranges.next();
			if parent_end >= range.end {
				break;
			}
			if start != parent_end {
				push_range(start..parent_end)
			}
			let Some(next_parent_range) = parent_ranges.peek() else {
				return;
			};
			start = next_parent_range.start;
		}
	}
	if start != range.end {
		push_range(start..range.end)
	}
}

fn ranges_intersect(a: &Range, b: &Range) -> bool {
	// Adapted from <https://github.com/helix-editor/helix/blob/8df58b2e1779dcf0046fb51ae1893c1eebf01e7c/helix-core/src/selection.rs#L156-L163>
	a.start == b.start || (a.end > b.start && b.end > a.start)
}

#[cfg(test)]
#[allow(
	clippy::single_range_in_vec_init,
	reason = "planner fixtures intentionally model one emitted range"
)]
mod tests {
	use {
		super::*,
		crate::{EngineConfig, SingleLanguageLoader, StringText, tree_sitter::Grammar},
		slab::Slab,
	};

	fn test_patterns() -> (Language, Pattern, Pattern) {
		let grammar = Grammar::try_from(tree_sitter_rust::LANGUAGE).expect("rust grammar should load");
		let loader = SingleLanguageLoader::from_queries(
			grammar,
			"",
			r#"
(identifier) @injection.content
  (#set! injection.language "rust")
(block) @injection.content
  (#set! injection.language "rust")
"#,
			"",
		)
		.expect("loader should build");
		let session = crate::DocumentSession::new(
			loader.language(),
			&StringText::new("fn alpha() { beta }\n"),
			&loader,
			EngineConfig::default(),
		)
		.expect("session should parse");
		let snapshot = session.snapshot();
		let mut cursor = InactiveQueryCursor::new(0..u32::MAX, TREE_SITTER_MATCH_LIMIT).execute_query(
			&loader.config().injection_query.injection_query,
			&snapshot.root_node(),
			tree_sitter::RopeInput::new(snapshot.rope_slice()),
		);
		let mut identifier = None;
		let mut block = None;
		while let Some((query_match, node_idx)) = cursor.next_matched_node() {
			match query_match.matched_node(node_idx).node.kind() {
				"identifier" => identifier = Some(query_match.pattern()),
				"block" => block = Some(query_match.pattern()),
				_ => {}
			}
		}
		(
			loader.language(),
			identifier.expect("identifier capture should exist"),
			block.expect("block capture should exist"),
		)
	}

	fn test_syntax(language: Language, child_pattern: Option<Pattern>) -> (Syntax, Layer) {
		let mut layers = Slab::with_capacity(2);
		let root = Layer(layers.insert(LayerData {
			language,
			parse_tree: None,
			origin_pattern: None,
			ranges: vec![range_to_tree_sitter(0..u32::MAX)],
			injections: Vec::new(),
			flags: LayerUpdateFlags::default(),
			parent: None,
			locals: Locals::default(),
		}) as u32);
		let child = Layer(layers.insert(LayerData {
			language,
			parse_tree: None,
			origin_pattern: child_pattern,
			ranges: Vec::new(),
			injections: Vec::new(),
			flags: LayerUpdateFlags::default(),
			parent: Some(root),
			locals: Locals::default(),
		}) as u32);
		(Syntax { layers, root }, child)
	}

	#[test]
	fn planner_requires_matching_pattern_for_reuse() {
		let (language, identifier_pattern, block_pattern) = test_patterns();
		let (syntax, child) = test_syntax(language, Some(identifier_pattern));
		let old_injections = vec![Injection {
			range: 10..16,
			layer: child,
			matched_node_range: 10..16,
		}];
		let candidates = vec![InjectionCandidate {
			language,
			pattern: block_pattern,
			scope: None,
			last_match: true,
			matched_node_range: 10..16,
			emitted_ranges: Vec::from([10..16]),
		}];

		let plan = syntax.plan_injections(candidates, old_injections, &[]);

		assert!(matches!(plan.layer_actions[0].source, PlannedLayerSource::New));
	}

	#[test]
	fn planner_retires_unmatched_layers() {
		let (language, identifier_pattern, _) = test_patterns();
		let (syntax, child) = test_syntax(language, Some(identifier_pattern));
		let old_injections = vec![Injection {
			range: 10..16,
			layer: child,
			matched_node_range: 10..16,
		}];

		let plan = syntax.plan_injections(Vec::new(), old_injections, &[]);

		assert_eq!(plan.retired_layers, vec![child]);
	}

	#[test]
	fn planner_sorts_final_injections_by_start() {
		let (language, identifier_pattern, block_pattern) = test_patterns();
		let (syntax, _) = test_syntax(language, None);
		let candidates = vec![
			InjectionCandidate {
				language,
				pattern: identifier_pattern,
				scope: None,
				last_match: true,
				matched_node_range: 20..24,
				emitted_ranges: Vec::from([20..24]),
			},
			InjectionCandidate {
				language,
				pattern: block_pattern,
				scope: None,
				last_match: true,
				matched_node_range: 4..8,
				emitted_ranges: Vec::from([4..8]),
			},
		];

		let plan = syntax.plan_injections(candidates, Vec::new(), &[]);

		assert_eq!(
			plan.injections
				.iter()
				.map(|injection| injection.range.clone())
				.collect::<Vec<_>>(),
			vec![4..8, 20..24],
		);
	}

	#[test]
	fn mapping_insert_before_injection_is_move_only() {
		let (language, identifier_pattern, _) = test_patterns();
		let mapped = map_old_injections(
			vec![Injection {
				range: 10..16,
				layer: Layer(1),
				matched_node_range: 8..18,
			}],
			&[tree_sitter::InputEdit {
				start_byte: 0,
				old_end_byte: 0,
				new_end_byte: 3,
				start_point: tree_sitter::Point { row: 0, col: 0 },
				old_end_point: tree_sitter::Point { row: 0, col: 0 },
				new_end_point: tree_sitter::Point { row: 0, col: 3 },
			}],
			|_| (language, Some(identifier_pattern)),
		);

		assert_eq!(mapped[0].range, 13..19);
		assert_eq!(mapped[0].matched_node_range, 11..21);
		assert!(mapped[0].moved);
		assert!(!mapped[0].modified);
	}
}
