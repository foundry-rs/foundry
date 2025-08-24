#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![allow(dead_code)] // TODO(dani)

const DEBUG: bool = false || option_env!("FMT_DEBUG").is_some();
const DEBUG_INDENT: bool = false;

use foundry_common::comments::{
    Comment, Comments,
    inline_config::{InlineConfig, InlineConfigItem},
};

// TODO(dani)
// #[macro_use]
// extern crate tracing;
use tracing as _;
use tracing_subscriber as _;

mod state;

mod pp;

use solar_parse::{
    ast::{SourceUnit, Span},
    interface::{Session, diagnostics::EmittedDiagnostics, source_map::SourceFile},
};
use std::{path::Path, sync::Arc};

pub use foundry_config::fmt::*;

/// The result of the formatter.
pub type FormatterResult = DiagnosticsResult<String, EmittedDiagnostics>;

/// The result of the formatter.
#[derive(Debug)]
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
    // First pass formatting
    let first_result = format_once(config.clone(), mk_file);

    // If first pass was not successful, return the result
    if first_result.is_err() {
        return first_result;
    }
    let Some(first_formatted) = first_result.ok_ref() else { return first_result };

    // Second pass formatting
    let second_result = format_once(config, &|sess| {
        sess.source_map()
            .new_source_file(
                solar_parse::interface::source_map::FileName::Custom("format-again".to_string()),
                first_formatted,
            )
            .map_err(|e| sess.dcx.err(e.to_string()).emit())
    });

    // Check if the two passes produce the same output (idempotency)
    match (first_result.ok_ref(), second_result.ok_ref()) {
        (Some(first), Some(second)) if first != second => {
            panic!("formatter is not idempotent:\n{}", diff(first, second));
        }
        _ => {}
    }

    if first_result.is_ok() && second_result.is_err() && !DEBUG {
        panic!(
            "failed to format a second time:\nfirst_result={first_result:#?}\nsecond_result={second_result:#?}"
        );
        // second_result
    } else {
        first_result
    }
}

fn diff(first: &str, second: &str) -> impl std::fmt::Display {
    use std::fmt::Write;
    let diff = similar::TextDiff::from_lines(first, second);
    let mut s = String::new();
    for change in diff.iter_all_changes() {
        let tag = match change.tag() {
            similar::ChangeTag::Delete => "-",
            similar::ChangeTag::Insert => "+",
            similar::ChangeTag::Equal => " ",
        };
        write!(s, "{tag}{change}").unwrap();
    }
    s
}

fn format_once(
    config: FormatterConfig,
    mk_file: &dyn Fn(&Session) -> solar_parse::interface::Result<Arc<SourceFile>>,
) -> FormatterResult {
    let sess =
        solar_parse::interface::Session::builder().with_buffer_emitter(Default::default()).build();
    let res = sess.enter(|| -> solar_parse::interface::Result<_> {
        let file = mk_file(&sess)?;
        let arena = solar_parse::ast::Arena::new();
        let mut parser = solar_parse::Parser::from_source_file(&sess, &arena, &file);
        let comments = Comments::new(
            &file,
            sess.source_map(),
            true,
            config.wrap_comments,
            if matches!(config.style, IndentStyle::Tab) { Some(config.tab_width) } else { None },
        );
        let ast = parser.parse_file().map_err(|e| e.emit())?;
        let inline_config = parse_inline_config(&sess, &comments, &ast);

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

fn parse_inline_config<'ast>(
    sess: &Session,
    comments: &Comments,
    ast: &'ast SourceUnit<'ast>,
) -> InlineConfig<()> {
    let parse_item = |mut item: &str, cmnt: &Comment| -> Option<(Span, InlineConfigItem<()>)> {
        if let Some(prefix) = cmnt.prefix() {
            item = item.strip_prefix(prefix).unwrap_or(item);
        }
        if let Some(suffix) = cmnt.suffix() {
            item = item.strip_suffix(suffix).unwrap_or(item);
        }
        let item = item.trim_start().strip_prefix("forgefmt:")?.trim();
        match item.parse::<InlineConfigItem<()>>() {
            Ok(item) => Some((cmnt.span, item)),
            Err(e) => {
                sess.dcx.warn(e.to_string()).span(cmnt.span).emit();
                None
            }
        }
    };

    let items = comments.iter().flat_map(|cmnt| {
        let mut found_items = Vec::with_capacity(2);
        // Always process the first line.
        if let Some(line) = cmnt.lines.first()
            && let Some(item) = parse_item(line, cmnt)
        {
            found_items.push(item);
        }
        // If the comment has more than one line, process the last line.
        if cmnt.lines.len() > 1
            && let Some(line) = cmnt.lines.last()
            && let Some(item) = parse_item(line, cmnt)
        {
            found_items.push(item);
        }
        found_items
    });

    InlineConfig::from_ast(items, ast, sess.source_map())
}
