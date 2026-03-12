use ropey::{Rope, RopeSlice};

#[derive(Debug, Clone)]
pub struct SealedSource {
	rope: Rope,
	/// Length of the original window bytes (no suffix).
	pub real_len_bytes: u32,
	/// Length of the synthetic suffix bytes.
	pub suffix_len_bytes: u32,
}

impl SealedSource {
	pub fn from_window(window: RopeSlice<'_>, suffix: &str) -> Self {
		let mut rope = Rope::new();
		for chunk in window.chunks() {
			rope.append(Rope::from(chunk));
		}
		let real_len_bytes = rope.len_bytes() as u32;
		if !suffix.is_empty() {
			rope.append(Rope::from(suffix));
		}
		let suffix_len_bytes = (rope.len_bytes() as u32) - real_len_bytes;

		Self {
			rope,
			real_len_bytes,
			suffix_len_bytes,
		}
	}

	pub fn slice(&self) -> RopeSlice<'_> {
		self.rope.slice(..)
	}
}
