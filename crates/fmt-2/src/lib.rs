#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![allow(dead_code)] // TODO(dani)

// TODO(dani)
// #[macro_use]
// extern crate tracing;
use tracing as _;
use tracing_subscriber as _;

pub mod inline_config;
pub use inline_config::InlineConfig;

mod comment;

mod comments;
pub use comments::Comments;

mod state;

pub(crate) mod iter;

mod pp;

use solar_parse::interface::{diagnostics::EmittedDiagnostics, source_map::SourceFile, Session};
use std::{path::Path, sync::Arc};

pub use foundry_config::fmt::*;

/// The result of the formatter.
pub type FormatterResult = DiagnosticsResult<String, EmittedDiagnostics>;

/// The result of the formatter.
pub enum DiagnosticsResult<T, E> {
    /// Everything went well.
    Ok(T),
    /// No errors encountered, but warnings or other non-error diagnostics were emitted.
    OkWithDiagnostics(T, E),
    /// Errors encountered, but a result was produced anyway.
    ErrRecovered(T, E),
    /// Fatal errors encountered.
    Err(E),
}

impl<T, E> DiagnosticsResult<T, E> {
    /// Converts the formatter result into a standard result.
    ///
    /// This ignores any non-error diagnostics if `Ok`, and any valid result if `Err`.
    pub fn into_result(self) -> Result<T, E> {
        match self {
            Self::Ok(s) | Self::OkWithDiagnostics(s, _) => Ok(s),
            Self::ErrRecovered(_, d) | Self::Err(d) => Err(d),
        }
    }

    /// Returns the result, even if it was produced with errors.
    pub fn into_ok(self) -> Result<T, E> {
        match self {
            Self::Ok(s) | Self::OkWithDiagnostics(s, _) | Self::ErrRecovered(s, _) => Ok(s),
            Self::Err(e) => Err(e),
        }
    }

    /// Returns any result produced.
    pub fn ok_ref(&self) -> Option<&T> {
        match self {
            Self::Ok(s) | Self::OkWithDiagnostics(s, _) | Self::ErrRecovered(s, _) => Some(s),
            Self::Err(_) => None,
        }
    }

    /// Returns any diagnostics emitted.
    pub fn err_ref(&self) -> Option<&E> {
        match self {
            Self::Ok(_) => None,
            Self::OkWithDiagnostics(_, d) | Self::ErrRecovered(_, d) | Self::Err(d) => Some(d),
        }
    }

    /// Returns `true` if the result is `Ok`.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_) | Self::OkWithDiagnostics(_, _))
    }

    /// Returns `true` if the result is `Err`.
    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }
}

pub fn format_file(path: &Path, config: FormatterConfig) -> FormatterResult {
    format_inner(config, &|sess| {
        sess.source_map().load_file(path).map_err(|e| sess.dcx.err(e.to_string()).emit())
    })
}

pub fn format_source(
    source: &str,
    path: Option<&Path>,
    config: FormatterConfig,
) -> FormatterResult {
    format_inner(config, &|sess| {
        let name = match path {
            Some(path) => solar_parse::interface::source_map::FileName::Real(path.to_path_buf()),
            None => solar_parse::interface::source_map::FileName::Stdin,
        };
        sess.source_map()
            .new_source_file(name, source)
            .map_err(|e| sess.dcx.err(e.to_string()).emit())
    })
}

fn format_inner(
    config: FormatterConfig,
    mk_file: &dyn Fn(&Session) -> solar_parse::interface::Result<Arc<SourceFile>>,
) -> FormatterResult {
    let sess =
        solar_parse::interface::Session::builder().with_buffer_emitter(Default::default()).build();
    let res = sess.enter(|| -> solar_parse::interface::Result<_> {
        let file = mk_file(&sess)?;
        let source = file.src.as_str();
        let arena = solar_parse::ast::Arena::new();
        let mut parser = solar_parse::Parser::from_source_file(&sess, &arena, &file);
        let comments = Comments::new(&file);
        let ast = parser.parse_file().map_err(|e| e.emit())?;
        let inline_config = parse_inline_config(&sess, &comments, source);

        let mut state = state::State::new(sess.source_map(), config, inline_config, comments);
        state.print_source_unit(&ast);
        Ok(state.s.eof())
    });
    let diagnostics = sess.emitted_diagnostics().unwrap();
    match (res, sess.dcx.has_errors()) {
        (Ok(s), Ok(())) if diagnostics.is_empty() => FormatterResult::Ok(s),
        (Ok(s), Ok(())) => FormatterResult::OkWithDiagnostics(s, diagnostics),
        (Ok(s), Err(_)) => FormatterResult::ErrRecovered(s, diagnostics),
        (Err(_), Ok(_)) => unreachable!(),
        (Err(_), Err(_)) => FormatterResult::Err(diagnostics),
    }
}

fn parse_inline_config(sess: &Session, comments: &Comments, src: &str) -> InlineConfig {
    let items = comments.iter().filter_map(|comment| {
        let mut item = comment.lines.first()?.as_str();
        if let Some(prefix) = comment.prefix() {
            item = item.strip_prefix(prefix).unwrap_or(item);
        }
        if let Some(suffix) = comment.suffix() {
            item = item.strip_suffix(suffix).unwrap_or(item);
        }
        let item = item.trim_start().strip_prefix("forgefmt:")?.trim();
        let span = comment.span;
        match item.parse::<inline_config::InlineConfigItem>() {
            Ok(item) => Some((span, item)),
            Err(e) => {
                sess.dcx.warn(e.to_string()).span(span).emit();
                None
            }
        }
    });
    InlineConfig::new(items, src)
}
