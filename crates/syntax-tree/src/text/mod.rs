use {
	ropey::{Rope, RopeBuilder, RopeSlice},
	std::ops::Range,
};

/// Borrowed text view used by the syntax engine.
pub trait TextSlice {
	fn len_bytes(&self) -> usize;

	fn to_owned_string(&self) -> String;
}

/// Text that can return owned string content for a byte range.
pub trait ByteRangeText {
	fn byte_text(&self, range: std::ops::Range<u32>) -> String;
}

/// Text storage that can be converted into a [`Rope`].
pub trait TextStorage {
	fn to_rope(&self) -> Rope;
}

#[derive(Debug, Clone, Copy)]
pub struct DocumentText<'a> {
	slice: RopeSlice<'a>,
}

impl<'a> DocumentText<'a> {
	pub(crate) fn new(slice: RopeSlice<'a>) -> Self {
		Self { slice }
	}
}

#[derive(Debug, Clone)]
pub struct RopeText {
	rope: Rope,
}

impl RopeText {
	pub fn new(rope: Rope) -> Self {
		Self { rope }
	}

	pub fn from_slice(slice: RopeSlice<'_>) -> Self {
		let mut rope = RopeBuilder::new();
		for chunk in slice.chunks() {
			rope.append(chunk);
		}
		Self { rope: rope.finish() }
	}
}

/// UTF-8 string-backed text storage.
#[derive(Debug, Clone)]
pub struct StringText {
	text: String,
}

impl StringText {
	pub fn new(text: impl Into<String>) -> Self {
		Self { text: text.into() }
	}

	/// Returns the underlying UTF-8 string slice.
	pub fn as_str(&self) -> &str {
		&self.text
	}
}

fn clamp_byte_range(range: Range<u32>, len: u32) -> Range<usize> {
	let end = range.end.min(len);
	let start = range.start.min(end);
	start as usize..end as usize
}

fn clamp_str_byte_range_inward(text: &str, range: Range<u32>) -> Range<usize> {
	let mut range = clamp_byte_range(range, text.len() as u32);
	// Move inward so byte-oriented callers never panic on a non-boundary UTF-8 slice.
	while range.start < range.end && !text.is_char_boundary(range.start) {
		range.start += 1;
	}
	while range.end > range.start && !text.is_char_boundary(range.end) {
		range.end -= 1;
	}
	range
}

impl TextSlice for RopeSlice<'_> {
	fn len_bytes(&self) -> usize {
		RopeSlice::len_bytes(self)
	}

	fn to_owned_string(&self) -> String {
		self.to_string()
	}
}

impl TextSlice for DocumentText<'_> {
	fn len_bytes(&self) -> usize {
		self.slice.len_bytes()
	}

	fn to_owned_string(&self) -> String {
		self.slice.to_string()
	}
}

impl TextSlice for str {
	fn len_bytes(&self) -> usize {
		self.len()
	}

	fn to_owned_string(&self) -> String {
		self.to_owned()
	}
}

impl TextStorage for RopeText {
	fn to_rope(&self) -> Rope {
		self.rope.clone()
	}
}

impl TextStorage for StringText {
	fn to_rope(&self) -> Rope {
		Rope::from_str(&self.text)
	}
}

impl TextStorage for Rope {
	fn to_rope(&self) -> Rope {
		self.clone()
	}
}

impl TextStorage for String {
	fn to_rope(&self) -> Rope {
		Rope::from_str(self)
	}
}

impl TextStorage for str {
	fn to_rope(&self) -> Rope {
		Rope::from_str(self)
	}
}

impl ByteRangeText for Rope {
	fn byte_text(&self, range: std::ops::Range<u32>) -> String {
		self.byte_slice(clamp_byte_range(range, self.len_bytes() as u32))
			.to_string()
	}
}

impl ByteRangeText for DocumentText<'_> {
	fn byte_text(&self, range: std::ops::Range<u32>) -> String {
		self.slice
			.byte_slice(clamp_byte_range(range, self.slice.len_bytes() as u32))
			.to_string()
	}
}

impl ByteRangeText for RopeText {
	fn byte_text(&self, range: std::ops::Range<u32>) -> String {
		self.rope.byte_text(range)
	}
}

impl ByteRangeText for StringText {
	/// Returns the substring covered by `range`.
	///
	/// Because `range` is expressed in bytes, invalid UTF-8 boundaries are clamped
	/// inward to the nearest valid character boundary before slicing.
	fn byte_text(&self, range: std::ops::Range<u32>) -> String {
		self.text[clamp_str_byte_range_inward(&self.text, range)].to_owned()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn string_text_byte_text_clamps_to_utf8_boundaries() {
		let text = StringText::new("aé世b");

		assert_eq!(text.byte_text(1..5), "é");
		assert_eq!(text.byte_text(2..6), "世");
		assert_eq!(text.byte_text(2..5), "");
		assert_eq!(text.byte_text(0..99), "aé世b");
	}
}
