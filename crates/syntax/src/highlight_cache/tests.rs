use super::*;

#[test]
fn theme_epoch_invalidates_tiles() {
	let mut cache = HighlightTiles::<u8>::with_capacity(2);
	cache.insert_tile(
		DocumentId(1),
		0,
		HighlightTile {
			key: HighlightKey {
				syntax_version: 1,
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
			tile_idx,
			HighlightTile {
				key: HighlightKey {
					syntax_version: 1,
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
		2,
		HighlightTile {
			key: HighlightKey {
				syntax_version: 1,
				theme_epoch: 0,
				tile_idx: 2,
			},
			spans: Vec::new(),
		},
	);

	assert!(cache.index.get(&DocumentId(1)).is_some_and(|doc| doc.contains_key(&0)));
	assert!(cache.index.get(&DocumentId(1)).is_some_and(|doc| !doc.contains_key(&1)));
	assert!(cache.index.get(&DocumentId(1)).is_some_and(|doc| doc.contains_key(&2)));
}
