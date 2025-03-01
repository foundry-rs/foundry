use ast::Spanned;
use std::path::PathBuf;
use syn::{
    parse::{discouraged::Speculative, Parse, ParseStream},
    Attribute, Error, Ident, LitStr, Result, Token,
};

/// Parsed input for `sol!`-like macro expanders. This enum represents a `Sol` file, a JSON ABI, or
/// a Solidity type.
#[derive(Clone, Debug)]
pub enum SolInputKind {
    /// Solidity type.
    Type(ast::Type),
    /// Solidity file or snippet.
    Sol(ast::File),
    /// JSON ABI file
    #[cfg(feature = "json")]
    Json(Ident, alloy_json_abi::ContractObject),
}

impl Parse for SolInputKind {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let fork = input.fork();
        match fork.parse() {
            Ok(file) => {
                input.advance_to(&fork);
                Ok(Self::Sol(file))
            }
            Err(e) => match input.parse() {
                Ok(ast::Type::Custom(_)) | Err(_) => Err(e),

                Ok(ast::Type::Mapping(m)) => {
                    Err(Error::new(m.span(), "mapping types are not yet supported"))
                }

                Ok(ty) => Ok(Self::Type(ty)),
            },
        }
    }
}

/// Parsed input for `sol!`-like macro expanders. This struct represents a list
/// of expandable items parsed from either solidity code snippets, or from a
/// JSON abi.
#[derive(Clone, Debug)]
pub struct SolInput {
    /// Attributes attached to the input, of the form `#[...]`.
    pub attrs: Vec<Attribute>,
    /// Path to the input, if any.
    pub path: Option<PathBuf>,
    /// The input kind.
    pub kind: SolInputKind,
}

impl Parse for SolInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let attrs = Attribute::parse_inner(input)?;

        // Ignore outer attributes when peeking.
        let fork = input.fork();
        let _fork_outer = Attribute::parse_outer(&fork)?;

        if fork.peek(LitStr) || (fork.peek(Ident) && fork.peek2(Token![,]) && fork.peek3(LitStr)) {
            Self::parse_abigen(attrs, input)
        } else {
            input.parse().map(|kind| Self { attrs, path: None, kind })
        }
    }
}

impl SolInput {
    /// `abigen`-like syntax: `sol!(name, "path/to/file")`
    fn parse_abigen(mut attrs: Vec<Attribute>, input: ParseStream<'_>) -> Result<Self> {
        attrs.extend(Attribute::parse_outer(input)?);

        let name = input.parse::<Option<Ident>>()?;
        if name.is_some() {
            input.parse::<Token![,]>()?;
        }
        let lit = input.parse::<LitStr>()?;

        let _ = input.parse::<Option<Token![,]>>()?;
        if !input.is_empty() {
            let msg = "unexpected token, expected end of input";
            return Err(Error::new(input.span(), msg));
        }

        let mut value = lit.value();
        let mut path = None;
        let span = lit.span();

        let is_path = {
            let s = value.trim();
            !(s.is_empty()
                || (s.starts_with('{') && s.ends_with('}'))
                || (s.starts_with('[') && s.ends_with(']')))
        };
        if is_path {
            let mut p = PathBuf::from(value);
            if p.is_relative() {
                let dir = std::env::var_os("CARGO_MANIFEST_DIR")
                    .map(PathBuf::from)
                    .ok_or_else(|| Error::new(span, "failed to get manifest dir"))?;
                p = dir.join(p);
            }
            p = dunce::canonicalize(&p)
                .map_err(|e| Error::new(span, format!("failed to canonicalize path {p:?}: {e}")))?;
            value = std::fs::read_to_string(&p)
                .map_err(|e| Error::new(span, format!("failed to read file {p:?}: {e}")))?;
            path = Some(p);
        }

        let s = value.trim();
        if s.is_empty() {
            let msg = if is_path { "file path is empty" } else { "empty input is not allowed" };
            Err(Error::new(span, msg))
        } else if (s.starts_with('{') && s.ends_with('}'))
            || (s.starts_with('[') && s.ends_with(']'))
        {
            #[cfg(feature = "json")]
            {
                let json = serde_json::from_str(s)
                    .map_err(|e| Error::new(span, format!("invalid JSON: {e}")))?;
                let name = name.ok_or_else(|| Error::new(span, "need a name for JSON ABI"))?;
                Ok(Self { attrs, path, kind: SolInputKind::Json(name, json) })
            }
            #[cfg(not(feature = "json"))]
            {
                let msg = "JSON support must be enabled with the \"json\" feature";
                Err(Error::new(span, msg))
            }
        } else {
            if let Some(name) = name {
                let msg = "names are not allowed outside of JSON ABI, remove this name";
                return Err(Error::new(name.span(), msg));
            }
            let kind = syn::parse_str(s).map_err(|e| {
                let msg = format!("expected a valid JSON ABI string or Solidity string: {e}");
                Error::new(span, msg)
            })?;
            Ok(Self { attrs, path, kind })
        }
    }
}
