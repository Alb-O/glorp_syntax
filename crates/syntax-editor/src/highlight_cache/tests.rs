use super::*;

#[test]
fn theme_epoch_invalidates_tiles() {
	let mut cache = HighlightTiles::<u8>::with_capacity(2);
	cache.insert_tile(
		DocumentId(1),
		HighlightTile {
			key: HighlightKey {
				syntax_version: 1,
				tree_id: 11,
				theme_epoch: 0,
				tile_idx: 0,
			},
			spans: Vec::new(),
		},
	);

	cache.set_theme_epoch(1);

	assert_eq!(cache.theme_epoch(), 1);
	assert!(cache.tiles.is_empty());
	assert!(cache.mru_order.is_empty());
	assert!(cache.index.is_empty());
}

#[test]
fn lru_eviction_reuses_slots() {
	let mut cache = HighlightTiles::<u8>::with_capacity(2);
	for tile_idx in 0..2 {
		cache.insert_tile(
			DocumentId(1),
			HighlightTile {
				key: HighlightKey {
					syntax_version: 1,
					tree_id: 11,
					theme_epoch: 0,
					tile_idx,
				},
				spans: Vec::new(),
			},
		);
	}

	cache.touch(0);
	cache.insert_tile(
		DocumentId(1),
		HighlightTile {
			key: HighlightKey {
				syntax_version: 1,
				tree_id: 11,
				theme_epoch: 0,
				tile_idx: 2,
			},
			spans: Vec::new(),
		},
	);

	assert!(
		cache
			.index
			.get(&DocumentId(1))
			.is_some_and(|doc| doc.contains_key(&(11, 0)))
	);
	assert!(
		cache
			.index
			.get(&DocumentId(1))
			.is_some_and(|doc| !doc.contains_key(&(11, 1)))
	);
	assert!(
		cache
			.index
			.get(&DocumentId(1))
			.is_some_and(|doc| doc.contains_key(&(11, 2)))
	);
}

#[test]
fn replacing_same_tile_reuses_slot() {
	let mut cache = HighlightTiles::<u8>::with_capacity(3);
	cache.insert_tile(
		DocumentId(1),
		HighlightTile {
			key: HighlightKey {
				syntax_version: 1,
				tree_id: 11,
				theme_epoch: 0,
				tile_idx: 0,
			},
			spans: Vec::new(),
		},
	);

	cache.insert_tile(
		DocumentId(1),
		HighlightTile {
			key: HighlightKey {
				syntax_version: 2,
				tree_id: 11,
				theme_epoch: 0,
				tile_idx: 0,
			},
			spans: Vec::new(),
		},
	);

	assert_eq!(cache.tiles.len(), 1);
	assert_eq!(cache.mru_order.len(), 1);
	assert_eq!(cache.tiles[0].key.syntax_version, 2);
	assert_eq!(
		cache.index.get(&DocumentId(1)).and_then(|doc| doc.get(&(11, 0))),
		Some(&0)
	);
}

#[test]
fn invalidate_document_reclaims_dead_tile_slots() {
	let mut cache = HighlightTiles::<u8>::with_capacity(3);
	for tile_idx in 0..2 {
		cache.insert_tile(
			DocumentId(1),
			HighlightTile {
				key: HighlightKey {
					syntax_version: 1,
					tree_id: 11,
					theme_epoch: 0,
					tile_idx,
				},
				spans: Vec::new(),
			},
		);
	}
	cache.insert_tile(
		DocumentId(2),
		HighlightTile {
			key: HighlightKey {
				syntax_version: 1,
				tree_id: 22,
				theme_epoch: 0,
				tile_idx: 0,
			},
			spans: Vec::new(),
		},
	);

	cache.invalidate_document(DocumentId(1));

	assert_eq!(cache.tiles.len(), 1);
	assert_eq!(cache.mru_order.len(), 1);
	assert!(!cache.index.contains_key(&DocumentId(1)));
	assert!(
		cache
			.index
			.get(&DocumentId(2))
			.is_some_and(|doc| doc.contains_key(&(22, 0)))
	);
}

#[test]
fn tiles_do_not_alias_across_tree_ids() {
	let mut cache = HighlightTiles::<u32>::with_capacity(4);
	cache.insert_tile(
		DocumentId(1),
		HighlightTile {
			key: HighlightKey {
				syntax_version: 7,
				tree_id: 1,
				theme_epoch: 0,
				tile_idx: 0,
			},
			spans: vec![(
				HighlightSpan {
					start: 0,
					end: 5,
					highlight: Highlight::new(1),
				},
				1,
			)],
		},
	);
	cache.insert_tile(
		DocumentId(1),
		HighlightTile {
			key: HighlightKey {
				syntax_version: 7,
				tree_id: 2,
				theme_epoch: 0,
				tile_idx: 0,
			},
			spans: vec![(
				HighlightSpan {
					start: 10,
					end: 15,
					highlight: Highlight::new(1),
				},
				2,
			)],
		},
	);

	assert_eq!(cache.tiles.len(), 2);
	assert!(
		cache
			.index
			.get(&DocumentId(1))
			.is_some_and(|doc| doc.contains_key(&(1, 0)))
	);
	assert!(
		cache
			.index
			.get(&DocumentId(1))
			.is_some_and(|doc| doc.contains_key(&(2, 0)))
	);
	assert_eq!(cache.index.get(&DocumentId(1)).map(HashMap::len), Some(2));
}
