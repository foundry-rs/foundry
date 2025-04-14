use comments::Comments;

use crate::{FormatterConfig, FormatterError, InlineConfig};
use std::path::Path;

mod algorithm;
mod convenience;
mod ring;

mod comment;
mod comments;

type Result<T> = std::result::Result<T, FormatterError>;

pub fn format_file(path: &Path, config: FormatterConfig) -> Result<String> {
    let source = std::fs::read_to_string(path).map_err(FormatterError::custom)?;
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
        Ok(format_source_unit(&ast, source, Comments::new(sess.source_map(), &file), config))
    });
    sess.emitted_errors().unwrap().map_err(FormatterError::custom)?;
    Ok(res.unwrap())
}

fn format_source_unit(
    source_unit: &solar_parse::ast::SourceUnit<'_>,
    src: &str,
    comments: Comments<'_>,
    config: FormatterConfig,
) -> String {
    let mut state = State::new(config, inline_configs(&comments, src), Some(comments));
    state.source_unit(source_unit);
    state.eof()
}

struct State<'a> {
    s: algorithm::Printer,
    comments: Option<Comments<'a>>,
    config: FormatterConfig,
    inline_config: InlineConfig,
}

impl std::ops::Deref for State<'_> {
    type Target = algorithm::Printer;

    fn deref(&self) -> &Self::Target {
        &self.s
    }
}

impl std::ops::DerefMut for State<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.s
    }
}

impl<'a> State<'a> {
    fn new(
        config: FormatterConfig,
        inline_config: InlineConfig,
        comments: Option<Comments<'a>>,
    ) -> Self {
        Self { s: algorithm::Printer::new(), comments, inline_config, config }
    }

    fn once(config: FormatterConfig) {
        Self::new(config, InlineConfig::default(), None);
    }
}

fn inline_configs(comments: &Comments<'_>, src: &str) -> InlineConfig {
    comments
        .iter()
        .filter_map(|comment| {
            Some((comment, comment.lines.first()?.trim_start().strip_prefix("forgefmt:")?.trim()))
        })
        .map(|(comment, item)| {
            let loc = comment.loc;
            item.parse().map(|out| (loc, out)).map_err(|out| (loc, out))
        })
}
