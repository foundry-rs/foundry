use crate::{writer::traits::ParamLike, AsDoc, CommentTag, Comments, Deployment, Markdown};
use itertools::Itertools;
use solang_parser::pt::{ErrorParameter, EventParameter, Parameter, VariableDeclaration};
use std::{
    fmt::{self, Display, Write},
    sync::LazyLock,
};

/// Solidity language name.
const SOLIDITY: &str = "solidity";

/// Headers and separator for rendering parameter table.
const PARAM_TABLE_HEADERS: &[&str] = &["Name", "Type", "Description"];
static PARAM_TABLE_SEPARATOR: LazyLock<String> =
    LazyLock::new(|| PARAM_TABLE_HEADERS.iter().map(|h| "-".repeat(h.len())).join("|"));

/// Headers and separator for rendering the deployments table.
const DEPLOYMENTS_TABLE_HEADERS: &[&str] = &["Network", "Address"];
static DEPLOYMENTS_TABLE_SEPARATOR: LazyLock<String> =
    LazyLock::new(|| DEPLOYMENTS_TABLE_HEADERS.iter().map(|h| "-".repeat(h.len())).join("|"));

/// The buffered writer.
/// Writes various display items into the internal buffer.
#[derive(Debug, Default)]
pub struct BufWriter {
    buf: String,
}

impl BufWriter {
    /// Create new instance of [BufWriter] from [ToString].
    pub fn new(content: impl ToString) -> Self {
        Self { buf: content.to_string() }
    }

    /// Returns true if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Write [AsDoc] implementation to the buffer.
    pub fn write_doc<T: AsDoc>(&mut self, doc: &T) -> fmt::Result {
        write!(self.buf, "{}", doc.as_doc()?)
    }

    /// Write [AsDoc] implementation to the buffer with newline.
    pub fn writeln_doc<T: AsDoc>(&mut self, doc: &T) -> fmt::Result {
        writeln!(self.buf, "{}", doc.as_doc()?)
    }

    /// Writes raw content to the buffer.
    pub fn write_raw<T: Display>(&mut self, content: T) -> fmt::Result {
        write!(self.buf, "{content}")
    }

    /// Writes raw content to the buffer with newline.
    pub fn writeln_raw<T: Display>(&mut self, content: T) -> fmt::Result {
        writeln!(self.buf, "{content}")
    }

    /// Writes newline to the buffer.
    pub fn writeln(&mut self) -> fmt::Result {
        writeln!(self.buf)
    }

    /// Writes a title to the buffer formatted as [Markdown::H1].
    pub fn write_title(&mut self, title: &str) -> fmt::Result {
        writeln!(self.buf, "{}", Markdown::H1(title))
    }

    /// Writes a subtitle to the bugger formatted as [Markdown::H2].
    pub fn write_subtitle(&mut self, subtitle: &str) -> fmt::Result {
        writeln!(self.buf, "{}", Markdown::H2(subtitle))
    }

    /// Writes heading to the buffer formatted as [Markdown::H3].
    pub fn write_heading(&mut self, heading: &str) -> fmt::Result {
        writeln!(self.buf, "{}", Markdown::H3(heading))
    }

    /// Writes text in italics to the buffer formatted as [Markdown::Italic].
    pub fn write_italic(&mut self, text: &str) -> fmt::Result {
        writeln!(self.buf, "{}", Markdown::Italic(text))
    }

    /// Writes bold text to the buffer formatted as [Markdown::Bold].
    pub fn write_bold(&mut self, text: &str) -> fmt::Result {
        writeln!(self.buf, "{}", Markdown::Bold(text))
    }

    /// Writes link to the buffer formatted as [Markdown::Link].
    pub fn write_link(&mut self, name: &str, path: &str) -> fmt::Result {
        writeln!(self.buf, "{}", Markdown::Link(name, path))
    }

    /// Writes a list item to the buffer indented by specified depth.
    pub fn write_list_item(&mut self, item: &str, depth: usize) -> fmt::Result {
        let indent = " ".repeat(depth * 2);
        writeln!(self.buf, "{indent}- {item}")
    }

    /// Writes a link to the buffer as a list item.
    pub fn write_link_list_item(&mut self, name: &str, path: &str, depth: usize) -> fmt::Result {
        let link = Markdown::Link(name, path);
        self.write_list_item(&link.as_doc()?, depth)
    }

    /// Writes a solidity code block block to the buffer.
    pub fn write_code(&mut self, code: &str) -> fmt::Result {
        writeln!(self.buf, "{}", Markdown::CodeBlock(SOLIDITY, code))
    }

    /// Write an item section to the buffer. First write comments, the item itself as code.
    pub fn write_section(&mut self, comments: &Comments, code: &str) -> fmt::Result {
        self.writeln_raw(comments.as_doc()?)?;
        self.write_code(code)?;
        self.writeln()
    }

    /// Tries to write the table to the buffer.
    /// Doesn't write anything if either params or comments are empty.
    fn try_write_table<T>(
        &mut self,
        tag: CommentTag,
        params: &[T],
        comments: &Comments,
        heading: &str,
    ) -> fmt::Result
    where
        T: ParamLike,
    {
        let comments = comments.include_tag(tag.clone());

        // There is nothing to write.
        if params.is_empty() || comments.is_empty() {
            return Ok(())
        }

        self.write_bold(heading)?;
        self.writeln()?;

        self.write_piped(&PARAM_TABLE_HEADERS.join("|"))?;
        self.write_piped(&PARAM_TABLE_SEPARATOR)?;

        for (index, param) in params.iter().enumerate() {
            let param_name = param.name();

            let mut comment = param_name.as_ref().and_then(|name| {
                comments.iter().find_map(|comment| comment.match_first_word(name))
            });

            // If it's a return tag and couldn't match by first word,
            // lookup the doc by index.
            if comment.is_none() && matches!(tag, CommentTag::Return) {
                comment = comments.get(index).map(|c| &*c.value);
            }

            let row = [
                Markdown::Code(param_name.unwrap_or("<none>")).as_doc()?,
                Markdown::Code(&param.type_name()).as_doc()?,
                comment.unwrap_or_default().replace('\n', " "),
            ];
            self.write_piped(&row.join("|"))?;
        }

        self.writeln()?;

        Ok(())
    }

    /// Tries to write the properties table to the buffer.
    /// Doesn't write anything if either params or comments are empty.
    pub fn try_write_properties_table(
        &mut self,
        params: &[VariableDeclaration],
        comments: &Comments,
    ) -> fmt::Result {
        self.try_write_table(CommentTag::Param, params, comments, "Properties")
    }

    /// Tries to write the parameters table to the buffer.
    /// Doesn't write anything if either params or comments are empty.
    pub fn try_write_events_table(
        &mut self,
        params: &[EventParameter],
        comments: &Comments,
    ) -> fmt::Result {
        self.try_write_table(CommentTag::Param, params, comments, "Parameters")
    }

    /// Tries to write the parameters table to the buffer.
    /// Doesn't write anything if either params or comments are empty.
    pub fn try_write_errors_table(
        &mut self,
        params: &[ErrorParameter],
        comments: &Comments,
    ) -> fmt::Result {
        self.try_write_table(CommentTag::Param, params, comments, "Parameters")
    }

    /// Tries to write the parameters table to the buffer.
    /// Doesn't write anything if either params or comments are empty.
    pub fn try_write_param_table(
        &mut self,
        tag: CommentTag,
        params: &[&Parameter],
        comments: &Comments,
    ) -> fmt::Result {
        let heading = match &tag {
            CommentTag::Param => "Parameters",
            CommentTag::Return => "Returns",
            _ => return Err(fmt::Error),
        };

        self.try_write_table(tag, params, comments, heading)
    }

    /// Writes the deployment table to the buffer.
    pub fn write_deployments_table(&mut self, deployments: Vec<Deployment>) -> fmt::Result {
        self.write_bold("Deployments")?;
        self.writeln()?;

        self.write_piped(&DEPLOYMENTS_TABLE_HEADERS.join("|"))?;
        self.write_piped(&DEPLOYMENTS_TABLE_SEPARATOR)?;

        for deployment in deployments {
            let mut network = deployment.network.ok_or(fmt::Error)?;
            network[0..1].make_ascii_uppercase();

            let row = [
                Markdown::Bold(&network).as_doc()?,
                Markdown::Code(&format!("{:?}", deployment.address)).as_doc()?,
            ];
            self.write_piped(&row.join("|"))?;
        }

        self.writeln()?;

        Ok(())
    }

    /// Write content to the buffer surrounded by pipes.
    pub fn write_piped(&mut self, content: &str) -> fmt::Result {
        self.write_raw("|")?;
        self.write_raw(content)?;
        self.writeln_raw("|")
    }

    /// Finish and return underlying buffer.
    pub fn finish(self) -> String {
        self.buf
    }
}
