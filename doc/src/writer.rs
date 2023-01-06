use itertools::Itertools;
use solang_parser::{doccomment::DocCommentTag, pt::Parameter};
use std::fmt::{self, Display, Write};

use crate::{
    format::{AsCode, AsDoc},
    output::DocOutput,
};

/// TODO: comments
#[derive(Default)]
pub(crate) struct BufWriter {
    buf: String,
}

impl BufWriter {
    pub(crate) fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub(crate) fn write_raw<T: Display>(&mut self, content: T) -> fmt::Result {
        write!(self.buf, "{content}")
    }

    pub(crate) fn writeln(&mut self) -> fmt::Result {
        writeln!(self.buf)
    }

    pub(crate) fn write_title(&mut self, title: &str) -> fmt::Result {
        writeln!(self.buf, "{}", DocOutput::H1(title))
    }

    pub(crate) fn write_subtitle(&mut self, subtitle: &str) -> fmt::Result {
        writeln!(self.buf, "{}", DocOutput::H2(subtitle))
    }

    pub(crate) fn write_heading(&mut self, subtitle: &str) -> fmt::Result {
        writeln!(self.buf, "{}", DocOutput::H3(subtitle))
    }

    pub(crate) fn write_bold(&mut self, text: &str) -> fmt::Result {
        writeln!(self.buf, "{}", DocOutput::Bold(text))
    }

    pub(crate) fn write_list_item(&mut self, item: &str, depth: usize) -> fmt::Result {
        let indent = " ".repeat(depth * 2);
        writeln!(self.buf, "{indent}- {item}")
    }

    pub(crate) fn write_link_list_item(
        &mut self,
        name: &str,
        path: &str,
        depth: usize,
    ) -> fmt::Result {
        let link = DocOutput::Link(name, path);
        self.write_list_item(&link.as_doc()?, depth)
    }

    pub(crate) fn write_code<T: AsCode>(&mut self, item: T) -> fmt::Result {
        let code = item.as_code();
        let block = DocOutput::CodeBlock("solidity", &code);
        writeln!(self.buf, "{block}")
    }

    // TODO: revise
    pub(crate) fn write_section<T: AsCode>(
        &mut self,
        item: T,
        comments: &Vec<DocCommentTag>,
    ) -> fmt::Result {
        self.write_raw(&comments.as_doc()?)?;
        self.writeln()?;
        self.write_code(item)?;
        self.writeln()?;
        Ok(())
    }

    pub(crate) fn write_param_table(
        &mut self,
        headers: &[&str],
        params: &[&Parameter],
        comments: &[&DocCommentTag],
    ) -> fmt::Result {
        self.write_piped(&headers.join("|"))?;

        let separator = headers.iter().map(|h| "-".repeat(h.len())).join("|");
        self.write_piped(&separator)?;

        for param in params {
            let param_name = param.name.as_ref().map(|n| n.name.to_owned());
            let description = param_name
                .as_ref()
                .and_then(|name| {
                    comments.iter().find_map(|comment| {
                        match comment.value.trim_start().split_once(' ') {
                            Some((tag_name, description)) if tag_name.trim().eq(name.as_str()) => {
                                Some(description.replace('\n', " "))
                            }
                            _ => None,
                        }
                    })
                })
                .unwrap_or_default();
            let row = [
                param_name.unwrap_or_else(|| "<none>".to_owned()),
                param.ty.as_code(),
                description,
            ];
            self.write_piped(&row.join("|"))?;
        }

        Ok(())
    }

    pub(crate) fn write_piped(&mut self, content: &str) -> fmt::Result {
        self.write_raw("|")?;
        self.write_raw(content)?;
        self.write_raw("|")
    }

    pub(crate) fn finish(self) -> String {
        self.buf
    }
}
