use super::*;

pub(crate) fn compile_error(error: Error) -> TS {
	let mut ce = TS::new();

	let compile_error = TT::Ident(I::new("compile_error", error.span));
	let exclaim = TT::Punct(Punct::new('!', Spacing::Joint));

	let mut inparens = TS::new();
	let msg = Literal::string(&error.msg);
	inparens.extend([TT::Literal(msg)]);
	let parens = TT::Group(Group::new(Delimiter::Parenthesis, inparens));

	ce.extend([compile_error, exclaim, parens]);
	ce
}

pub(crate) fn parse_literal(lit: Literal) -> MResult<String> {
	let span = lit.span();
	let tx = lit.to_string();
	let notasl = || Err(Error::new_static("not a string literal", span));
	let Some(s) = tx.strip_suffix('"') else {
		return notasl();
	};
	let error_on_escapes;
	let s = if let Some(s) = s.strip_prefix('r') {
		error_on_escapes = false;
		s
	} else {
		error_on_escapes = true;
		s
	};
	let Some(s) = s.strip_prefix('"') else {
		return notasl();
	};

	if error_on_escapes {
		for c in s.chars() {
			if c == '\\' {
				return Err(Error::new_static("escape sequences are unsupported", span));
			}
		}
	}
	Ok(s.to_owned())
}
