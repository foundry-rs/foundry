#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![allow(dead_code)] // TODO(dani)

// TODO(dani)
// #[macro_use]
// extern crate tracing;
use tracing as _;

pub mod inline_config;
pub use inline_config::InlineConfig;

mod comment;

mod comments;
pub use comments::Comments;

mod state;

pub(crate) mod iter;

mod pp;

use solar_parse::interface::Session;
use std::path::Path;

pub use foundry_config::fmt::*;

type Result<T> = std::result::Result<T, FormatterError>;

#[derive(Debug, thiserror::Error)]
pub enum FormatterError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("\n{0}")]
    Parse(#[from] solar_parse::interface::diagnostics::EmittedDiagnostics),
}

pub fn format_file(path: &Path, config: FormatterConfig) -> Result<String> {
    let source = std::fs::read_to_string(path)?;
    format_source(&source, Some(path), config)
}

pub fn format_source(source: &str, path: Option<&Path>, config: FormatterConfig) -> Result<String> {
    let sess =
        solar_parse::interface::Session::builder().with_buffer_emitter(Default::default()).build();
    let res = sess.enter(|| -> solar_parse::interface::Result<_> {
        let name = match path {
            Some(path) => solar_parse::interface::source_map::FileName::Real(path.to_path_buf()),
            None => solar_parse::interface::source_map::FileName::Custom("fmt".to_string()),
        };
        let arena = solar_parse::ast::Arena::new();
        let file = sess
            .source_map()
            .new_source_file(name, source)
            .map_err(|e| sess.dcx.err(e.to_string()).emit())?;
        let mut parser = solar_parse::Parser::from_source_file(&sess, &arena, &file);
        let ast = parser.parse_file().map_err(|e| e.emit())?;
        let comments = Comments::new(&file);
        let inline_config = parse_inline_config(&sess, &comments, source);

        let mut state = state::State::new(sess.source_map(), config, inline_config, comments);
        state.print_source_unit(&ast);
        Ok(state.s.eof())
    });
    // TODO(dani): add a non-fatal error that returns the formatted source with the errors
    sess.emitted_errors().unwrap()?;
    Ok(res.unwrap())
}

fn parse_inline_config(sess: &Session, comments: &Comments, src: &str) -> InlineConfig {
    let items = comments.iter().filter_map(|comment| {
        let item = comment.lines.first()?.trim_start().strip_prefix("forgefmt:")?.trim();
        let span = comment.span;
        match item.parse::<inline_config::InlineConfigItem>() {
            Ok(item) => Some((span, item)),
            Err(e) => {
                let _ = sess.dcx.err(e.to_string()).span(span).emit();
                None
            }
        }
    });
    InlineConfig::new(items, src)
}
