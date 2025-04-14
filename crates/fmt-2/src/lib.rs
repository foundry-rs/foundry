#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

pub mod inline_config;
pub use inline_config::InlineConfig;

mod comment;

mod comments;
pub use comments::Comments;

mod state;

mod pp;

use solar_parse::interface::Session;
use std::path::Path;

pub use foundry_config::fmt::*;

type Result<T> = std::result::Result<T, FormatterError>;

#[derive(Debug, thiserror::Error)]
pub enum FormatterError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
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
        let comments = Comments::new(sess.source_map(), &file);
        let inline_config = parse_inline_config(&sess, &comments, source);
        Ok(format_source_unit(&ast, config, inline_config, comments))
    });
    sess.emitted_errors().unwrap()?;
    Ok(res.unwrap())
}

fn format_source_unit(
    source_unit: &solar_parse::ast::SourceUnit<'_>,
    config: FormatterConfig,
    inline_config: InlineConfig,
    comments: Comments<'_>,
) -> String {
    let mut state = state::State::new(config, inline_config, Some(comments));
    // state.source_unit(source_unit);
    // state.eof()
    // TODO(dani)
    let _ = source_unit;
    let _ = &mut state;
    todo!()
}

fn parse_inline_config(sess: &Session, comments: &Comments<'_>, src: &str) -> InlineConfig {
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
