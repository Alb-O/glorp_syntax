use super::*;

#[test]
fn byte_range_is_clamped_to_source() {
	let rope = Rope::from_str("alpha\nbeta\n");
	let sealed = SealedSource::from_byte_range(rope.slice(..), 2..99, "");

	assert_eq!(sealed.real_len_bytes, 9);
	assert_eq!(sealed.suffix_len_bytes, 0);
	assert_eq!(sealed.slice().to_string(), "pha\nbeta\n");
}

#[test]
fn newline_padding_is_added_only_when_missing() {
	let rope = Rope::from_str("alpha\nbeta");
	let sealed = SealedSource::from_byte_range_with_newline_padding(rope.slice(..), 6..10);
	assert_eq!(sealed.real_len_bytes, 4);
	assert_eq!(sealed.suffix_len_bytes, 1);
	assert_eq!(sealed.slice().to_string(), "beta\n");

	let rope = Rope::from_str("alpha\nbeta\n");
	let sealed = SealedSource::from_byte_range_with_newline_padding(rope.slice(..), 6..11);
	assert_eq!(sealed.real_len_bytes, 5);
	assert_eq!(sealed.suffix_len_bytes, 0);
	assert_eq!(sealed.slice().to_string(), "beta\n");
}
