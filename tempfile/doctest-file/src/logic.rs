use super::*;
use std::cmp::min;

// A line in pass 1 can be in one of the following states:
// - Visible: line does not get hidden.
// - Hidden line: line gets hidden because it has an empty end-of-line comment, but it is not part
//   of a hidden block of lines.
// - In-boundary: line does not get hidden, but its end-of-line comment starts a hidden block.
// - Hidden block: line is part of a hidden block, either because the hidden block was started at a
//   previous line or because this line is an `InBoundary` that has nothing besides an end-of-line
//   comment and whitespace.
// - Out-boundary: line gets hidden, but its end-of-line comment ends a hidden block.
pub(crate) struct Pass1<I> {
	hidden_block: bool,
	total_length: usize,
	min_indent: Option<usize>,
	lines: I,
}
impl<I> Pass1<I> {
	pub fn new(lines: I) -> Self {
		Pass1 {
			hidden_block: false,
			total_length: 0,
			min_indent: None,
			lines,
		}
	}
	pub fn total_length(&self) -> usize {
		self.total_length
	}
	pub fn min_indent(&self) -> usize {
		self.min_indent.unwrap_or(0)
	}
}
impl<I: Iterator<Item = MResult<String>>> Iterator for Pass1<I> {
	type Item = MResult<(String, bool)>;
	fn next(&mut self) -> Option<Self::Item> {
		let mut linebuf = match self.lines.next()? {
			Ok(s) => s,
			Err(e) => return Some(Err(e)),
		};
		let line = linebuf.trim_end();

		let visible;
		if self.hidden_block {
			visible = false;
			if let Some(line) = line.strip_suffix("//}") {
				linebuf.truncate(line.len());
				self.hidden_block = false;
			}
		} else if let Some(line) = line.strip_suffix("//{") {
			visible = !line.trim_start().is_empty();
			linebuf.truncate(line.len());
			self.hidden_block = true;
		} else if let Some(line) = line.strip_suffix("//") {
			visible = false;
			linebuf.truncate(line.len());
		} else {
			visible = true;
		}

		// Empty lines have indeterminate indentation, meaning that a dedent that appears as a blank
		// line is not a dedent.
		if visible && !linebuf.trim().is_empty() {
			self.min_indent = Some(min(
				self.min_indent.unwrap_or(usize::MAX),
				indent_of(&linebuf),
			));
		}
		self.total_length += linebuf.len();
		if !visible {
			self.total_length += 2;
		}

		Some(Ok((linebuf, visible)))
	}
}

pub(crate) fn indent_of(s: &str) -> usize {
	let mut cnt = 0;
	for c in s.chars() {
		if c == ' ' {
			cnt += 1;
		} else if c == '\t' {
			// Rustdoc always uses a tapstop value of 4, as of Rust 1.80.
			cnt += 4;
		} else {
			break;
		}
	}
	cnt
}
