use crate::{
    helpers::{comments_by_tag, exclude_comment_tags},
    parser::{ParseItem, ParseSource},
    writer::BufWriter,
};
use itertools::Itertools;
use solang_parser::{doccomment::DocCommentTag, pt::Base};

/// The result of [Asdoc::as_doc] method.
pub type AsDocResult = Result<String, std::fmt::Error>;

/// A trait for formatting a parse unit as documentation.
#[auto_impl::auto_impl(&)]
pub trait AsDoc {
    /// Formats a parse tree item into a doc string.
    fn as_doc(&self) -> AsDocResult;
}

impl AsDoc for String {
    fn as_doc(&self) -> AsDocResult {
        Ok(self.to_owned())
    }
}

impl AsDoc for DocCommentTag {
    fn as_doc(&self) -> AsDocResult {
        Ok(self.value.to_owned())
    }
}

impl AsDoc for Vec<&DocCommentTag> {
    fn as_doc(&self) -> AsDocResult {
        Ok(self
            .iter()
            .map(|c| DocCommentTag::as_doc(*c))
            .collect::<Result<Vec<_>, _>>()?
            .join("\n\n"))
    }
}

impl AsDoc for Vec<DocCommentTag> {
    fn as_doc(&self) -> AsDocResult {
        Ok(self.iter().map(DocCommentTag::as_doc).collect::<Result<Vec<_>, _>>()?.join("\n\n"))
    }
}

impl AsDoc for Base {
    fn as_doc(&self) -> AsDocResult {
        Ok(self.name.identifiers.iter().map(|ident| ident.name.to_owned()).join("."))
    }
}

impl AsDoc for ParseItem {
    fn as_doc(&self) -> AsDocResult {
        let mut writer = BufWriter::default();

        match &self.source {
            ParseSource::Contract(contract) => {
                writer.write_title(&contract.name.name)?;

                if !contract.base.is_empty() {
                    // TODO:
                    // Ok(self.lookup_contract_base(docs.as_ref(), base)?.unwrap_or(base.doc()))
                    // TODO: should be a name & perform lookup
                    let bases = contract
                        .base
                        .iter()
                        .map(|base| base.as_doc())
                        .collect::<Result<Vec<_>, _>>()?;
                    writer.write_bold("Inherits:")?;
                    writer.write_raw(bases.join(", "))?;
                    writer.writeln()?;
                }

                writer.write_raw(self.comments.as_doc()?)?;

                if let Some(state_vars) = self.variables() {
                    writer.write_subtitle("State Variables")?;
                    state_vars
                        .into_iter()
                        .try_for_each(|(item, comments)| writer.write_section(item, comments))?;
                }

                if let Some(funcs) = self.functions() {
                    writer.write_subtitle("Functions")?;
                    funcs.into_iter().try_for_each(|(func, comments)| {
                        // Write function name
                        let func_name =
                            func.name.as_ref().map_or(func.ty.to_string(), |n| n.name.to_owned());
                        writer.write_heading(&func_name)?;
                        writer.writeln()?;

                        // Write function docs
                        writer.write_raw(
                            exclude_comment_tags(comments, vec!["param", "return"]).as_doc()?,
                        )?;

                        // Write function header
                        writer.write_code(func)?;

                        // Write function parameter comments in a table
                        let params =
                            func.params.iter().filter_map(|p| p.1.as_ref()).collect::<Vec<_>>();
                        let param_comments = comments_by_tag(comments, "param");
                        if !params.is_empty() && !param_comments.is_empty() {
                            writer.write_heading("Parameters")?;
                            writer.writeln()?;
                            writer.write_param_table(
                                &["Name", "Type", "Description"],
                                &params,
                                &param_comments,
                            )?
                        }

                        // Write function parameter comments in a table
                        let returns =
                            func.returns.iter().filter_map(|p| p.1.as_ref()).collect::<Vec<_>>();
                        let returns_comments = comments_by_tag(comments, "return");
                        if !returns.is_empty() && !returns_comments.is_empty() {
                            writer.write_heading("Returns")?;
                            writer.writeln()?;
                            writer.write_param_table(
                                &["Name", "Type", "Description"],
                                &returns,
                                &returns_comments,
                            )?;
                        }

                        writer.writeln()?;

                        Ok::<(), std::fmt::Error>(())
                    })?;
                }

                if let Some(events) = self.events() {
                    writer.write_subtitle("Events")?;
                    events.into_iter().try_for_each(|(item, comments)| {
                        writer.write_heading(&item.name.name)?;
                        writer.write_section(item, comments)
                    })?;
                }

                if let Some(errors) = self.errors() {
                    writer.write_subtitle("Errors")?;
                    errors.into_iter().try_for_each(|(item, comments)| {
                        writer.write_heading(&item.name.name)?;
                        writer.write_section(item, comments)
                    })?;
                }

                if let Some(structs) = self.structs() {
                    writer.write_subtitle("Structs")?;
                    structs.into_iter().try_for_each(|(item, comments)| {
                        writer.write_heading(&item.name.name)?;
                        writer.write_section(item, comments)
                    })?;
                }

                if let Some(enums) = self.enums() {
                    writer.write_subtitle("Enums")?;
                    enums.into_iter().try_for_each(|(item, comments)| {
                        writer.write_heading(&item.name.name)?;
                        writer.write_section(item, comments)
                    })?;
                }
            }
            ParseSource::Variable(var) => {
                writer.write_title(&var.name.name)?;
                writer.write_section(var, &self.comments)?;
            }
            ParseSource::Event(event) => {
                writer.write_title(&event.name.name)?;
                writer.write_section(event, &self.comments)?;
            }
            ParseSource::Error(error) => {
                writer.write_title(&error.name.name)?;
                writer.write_section(error, &self.comments)?;
            }
            ParseSource::Struct(structure) => {
                writer.write_title(&structure.name.name)?;
                writer.write_section(structure, &self.comments)?;
            }
            ParseSource::Enum(enumerable) => {
                writer.write_title(&enumerable.name.name)?;
                writer.write_section(enumerable, &self.comments)?;
            }
            ParseSource::Function(func) => {
                // TODO: cleanup
                // Write function name
                let func_name =
                    func.name.as_ref().map_or(func.ty.to_string(), |n| n.name.to_owned());
                writer.write_heading(&func_name)?;
                writer.writeln()?;

                // Write function docs
                writer.write_raw(
                    exclude_comment_tags(&self.comments, vec!["param", "return"]).as_doc()?,
                )?;

                // Write function header
                writer.write_code(func)?;

                // Write function parameter comments in a table
                let params = func.params.iter().filter_map(|p| p.1.as_ref()).collect::<Vec<_>>();
                let param_comments = comments_by_tag(&self.comments, "param");
                if !params.is_empty() && !param_comments.is_empty() {
                    writer.write_heading("Parameters")?;
                    writer.writeln()?;
                    writer.write_param_table(
                        &["Name", "Type", "Description"],
                        &params,
                        &param_comments,
                    )?
                }

                // Write function parameter comments in a table
                let returns = func.returns.iter().filter_map(|p| p.1.as_ref()).collect::<Vec<_>>();
                let returns_comments = comments_by_tag(&self.comments, "return");
                if !returns.is_empty() && !returns_comments.is_empty() {
                    writer.write_heading("Returns")?;
                    writer.writeln()?;
                    writer.write_param_table(
                        &["Name", "Type", "Description"],
                        &returns,
                        &returns_comments,
                    )?;
                }

                writer.writeln()?;
            }
        };

        Ok(writer.finish())
    }
}
