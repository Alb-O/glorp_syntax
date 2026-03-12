use ropey::{Rope, RopeSlice};

pub trait TextSlice {
	fn len_bytes(&self) -> usize;

	fn to_owned_string(&self) -> String;
}

pub trait ByteRangeText {
	fn byte_text(&self, range: std::ops::Range<u32>) -> String;
}

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
		let mut rope = Rope::new();
		for chunk in slice.chunks() {
			rope.append(Rope::from(chunk));
		}
		Self { rope }
	}
}

#[derive(Debug, Clone)]
pub struct StringText {
	text: String,
}

impl StringText {
	pub fn new(text: impl Into<String>) -> Self {
		Self { text: text.into() }
	}

	pub fn as_str(&self) -> &str {
		&self.text
	}
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
		let end = range.end.min(self.len_bytes() as u32);
		let start = range.start.min(end);
		self.byte_slice(start as usize..end as usize).to_string()
	}
}

impl ByteRangeText for DocumentText<'_> {
	fn byte_text(&self, range: std::ops::Range<u32>) -> String {
		let end = range.end.min(self.slice.len_bytes() as u32);
		let start = range.start.min(end);
		self.slice.byte_slice(start as usize..end as usize).to_string()
	}
}

impl ByteRangeText for RopeText {
	fn byte_text(&self, range: std::ops::Range<u32>) -> String {
		self.rope.byte_text(range)
	}
}

impl ByteRangeText for StringText {
	fn byte_text(&self, range: std::ops::Range<u32>) -> String {
		let end = range.end.min(self.text.len() as u32) as usize;
		let start = range.start.min(range.end).min(end as u32) as usize;
		self.text[start..end].to_owned()
	}
}
