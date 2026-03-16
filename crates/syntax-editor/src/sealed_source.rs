use {
	ropey::{Rope, RopeBuilder, RopeSlice},
	std::ops::Range,
};

#[derive(Debug, Clone)]
pub struct SealedSource {
	rope: Rope,
	pub real_len_bytes: u32,
	pub suffix_len_bytes: u32,
}

impl SealedSource {
	pub fn from_window(window: RopeSlice<'_>, suffix: &str) -> Self {
		let mut rope = RopeBuilder::new();
		for chunk in window.chunks() {
			rope.append(chunk);
		}
		let real_len_bytes = window.len_bytes() as u32;
		let suffix_len_bytes = suffix.len() as u32;
		if !suffix.is_empty() {
			rope.append(suffix);
		}

		Self {
			rope: rope.finish(),
			real_len_bytes,
			suffix_len_bytes,
		}
	}

	pub fn from_byte_range(source: RopeSlice<'_>, range: Range<u32>, suffix: &str) -> Self {
		Self::from_window(
			source.byte_slice(clamp_byte_range(range, source.len_bytes() as u32)),
			suffix,
		)
	}

	pub fn from_byte_range_with_newline_padding(source: RopeSlice<'_>, range: Range<u32>) -> Self {
		let window = source.byte_slice(clamp_byte_range(range, source.len_bytes() as u32));
		let suffix = if needs_line_padding(window) { "\n" } else { "" };
		Self::from_window(window, suffix)
	}

	pub fn slice(&self) -> RopeSlice<'_> {
		self.rope.slice(..)
	}
}

fn clamp_byte_range(range: Range<u32>, len: u32) -> Range<usize> {
	let end = range.end.min(len);
	let start = range.start.min(end);
	start as usize..end as usize
}

fn needs_line_padding(window: RopeSlice<'_>) -> bool {
	window
		.chunks()
		.last()
		.is_some_and(|chunk| !chunk.ends_with('\n') && !chunk.ends_with('\r'))
}

#[cfg(test)]
mod tests {
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
}
