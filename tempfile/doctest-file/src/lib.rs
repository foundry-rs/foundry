#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
extern crate proc_macro;

mod logic;
mod tokenmanip;
mod imports {
	pub(crate) use proc_macro::{
		Delimiter, Group, Ident as I, Literal, Punct, Spacing, Span, TokenStream as TS,
		TokenTree as TT,
	};
}

use std::{
	borrow::Cow,
	fs::File,
	io::{BufRead, BufReader},
	path::PathBuf,
};
use {imports::*, logic::*, tokenmanip::*};

type MResult<T = TS> = Result<T, Error>;

struct Error {
	msg: Cow<'static, str>,
	span: Span,
}
impl Error {
	fn new_static(msg: &'static str, span: Span) -> Self {
		Self {
			msg: Cow::Borrowed(msg),
			span,
		}
	}
	fn new_owned(msg: String, span: Span) -> Self {
		Self {
			msg: Cow::Owned(msg),
			span,
		}
	}
}

/// Includes a documentation test from a separate file, **without** inserting the surrounding
/// \`\`\` markers.
///
/// See the [crate-level documentation](crate) for more.
#[proc_macro]
pub fn include_doctest(input: TS) -> TS {
	macro_main(input).unwrap_or_else(compile_error)
}

struct Input {
	filename: PathBuf,
	filename_span: Span,
}

fn parse_input(input: TS) -> MResult<Input> {
	let mut input = input.into_iter();
	let Some(literal) = input.next() else {
		return Err(Error::new_static(
			"expected filename, found empty parameter list",
			Span::call_site(),
		));
	};
	let lspan = literal.span();
	let TT::Literal(literal) = literal else {
		return Err(Error::new_owned(
			format!("expected literal, found \"{literal}\""),
			lspan,
		));
	};

	Ok(Input {
		filename: PathBuf::from(parse_literal(literal)?),
		filename_span: lspan,
	})
}

fn macro_main(input: TS) -> MResult {
	let input = parse_input(input)?;
	// PathBuf::push() with absolute paths replaces the original value.
	let mut path = if input.filename.is_relative() {
		std::env::var_os("CARGO_MANIFEST_DIR")
			.map(PathBuf::from)
			.ok_or_else(|| {
				Error::new_static(
					"the CARGO_MANIFEST_DIR environment variable is not set",
					Span::call_site(),
				)
			})?
	} else {
		PathBuf::new()
	};
	path.push(&input.filename);

	let fln = input.filename.display();
	let ioe = |m, e| {
		Error::new_owned(
			format!("I/O error (file {fln}) {m}: {e}"),
			input.filename_span,
		)
	};
	let file = File::open(path).map_err(|e| ioe("could not open", e))?;

	let lines = BufReader::new(file)
		.lines()
		.map(|rslt| rslt.map_err(|e| ioe("read failed", e)));

	let mut pass1 = Pass1::new(lines);
	let mut lines_pass2 = Vec::with_capacity(256);
	for rslt in &mut pass1 {
		let t = rslt?;
		lines_pass2.push(t);
	}

	let mut docstring = String::with_capacity(pass1.total_length());
	let dedent = pass1.min_indent();
	for (line, visible) in lines_pass2 {
		if visible {
			// The space at the beginning is the space immediately after the /// that gets eaten by
			// Rustdoc to make doc comments look nicer.
			docstring.push(' ');
			let indent = indent_of(&line);
			for _ in 0..indent.saturating_sub(dedent) {
				docstring.push(' ');
			}
			docstring.push_str(line.trim_start());
		} else {
			docstring.push_str("# ");
			docstring.push_str(&line);
		}
		docstring.push('\n');
	}

	while docstring.ends_with('\n') {
		docstring.pop();
	}

	Ok(TT::Literal(Literal::string(&docstring)).into())
}
