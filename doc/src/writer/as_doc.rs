use std::path::Path;

use crate::{
    document::read_context, parser::ParseSource, writer::BufWriter, CommentTag, Comments,
    CommentsRef, Document, Markdown, PreprocessorOutput, CONTRACT_INHERITANCE_ID,
};
use itertools::Itertools;
use solang_parser::pt::Base;

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

impl AsDoc for Comments {
    fn as_doc(&self) -> AsDocResult {
        CommentsRef::from(self).as_doc()
    }
}

impl<'a> AsDoc for CommentsRef<'a> {
    fn as_doc(&self) -> AsDocResult {
        let mut writer = BufWriter::default();

        // TODO: title

        let authors = self.include_tag(CommentTag::Author);
        if !authors.is_empty() {
            writer.write_bold(&format!("Author{}:", if authors.len() == 1 { "" } else { "s" }))?;
            writer.writeln_raw(authors.iter().map(|a| &a.value).join(", "))?;
            writer.writeln()?;
        }

        // TODO: other tags
        let docs = self.include_tags(&[CommentTag::Dev, CommentTag::Notice]);
        for doc in docs.iter() {
            writer.writeln_raw(&doc.value)?;
            writer.writeln()?;
        }

        Ok(writer.finish())
    }
}

impl AsDoc for Base {
    fn as_doc(&self) -> AsDocResult {
        Ok(self.name.identifiers.iter().map(|ident| ident.name.to_owned()).join("."))
    }
}

impl AsDoc for Document {
    fn as_doc(&self) -> AsDocResult {
        let mut writer = BufWriter::default();

        // Handle constant files
        if self.items.iter().all(|item| matches!(item.source, ParseSource::Variable(_))) {
            writer.write_title("Constants")?;

            for item in self.items.iter() {
                let var = item.as_variable().unwrap();
                writer.write_heading(&var.name.name)?;
                writer.write_section(var, &item.comments)?;
            }

            return Ok(writer.finish())
        }

        for item in self.items.iter() {
            match &item.source {
                ParseSource::Contract(contract) => {
                    writer.write_title(&contract.name.name)?;

                    if !contract.base.is_empty() {
                        writer.write_bold("Inherits:")?;

                        let mut bases = vec![];
                        let linked =
                            read_context!(self, CONTRACT_INHERITANCE_ID, ContractInheritance);
                        for base in contract.base.iter() {
                            let base_doc = base.as_doc()?;
                            let base_ident = &base.name.identifiers.last().unwrap().name;
                            bases.push(
                                linked
                                    .as_ref()
                                    .and_then(|l| {
                                        l.get(base_ident).map(|path| {
                                            let path = Path::new("/").join(
                                                // TODO: move to func
                                                path.strip_prefix("docs/src").ok().unwrap_or(path),
                                            );
                                            Markdown::Link(&base_doc, &path.display().to_string())
                                                .as_doc()
                                        })
                                    })
                                    .transpose()?
                                    .unwrap_or(base_doc),
                            )
                        }

                        writer.writeln_raw(bases.join(", "))?;
                        writer.writeln()?;
                    }

                    writer.writeln_doc(&item.comments)?;

                    if let Some(state_vars) = item.variables() {
                        writer.write_subtitle("State Variables")?;
                        state_vars.into_iter().try_for_each(|(item, comments)| {
                            writer.write_heading(&item.name.name)?;
                            writer.write_section(item, comments)?;
                            writer.writeln()
                        })?;
                    }

                    if let Some(funcs) = item.functions() {
                        writer.write_subtitle("Functions")?;
                        funcs.into_iter().try_for_each(|(func, comments)| {
                            // Write function name
                            let func_name = func
                                .name
                                .as_ref()
                                .map_or(func.ty.to_string(), |n| n.name.to_owned());
                            writer.write_heading(&func_name)?;
                            writer.writeln()?;

                            // Write function docs
                            writer.writeln_doc(
                                comments.exclude_tags(&[CommentTag::Param, CommentTag::Return]),
                            )?;

                            // Write function header
                            writer.write_code(func)?;

                            // Write function parameter comments in a table
                            let params =
                                func.params.iter().filter_map(|p| p.1.as_ref()).collect::<Vec<_>>();
                            writer.try_write_param_table(CommentTag::Param, &params, comments)?;

                            // Write function parameter comments in a table
                            let returns = func
                                .returns
                                .iter()
                                .filter_map(|p| p.1.as_ref())
                                .collect::<Vec<_>>();
                            writer.try_write_param_table(CommentTag::Return, &returns, comments)?;

                            writer.writeln()?;

                            Ok::<(), std::fmt::Error>(())
                        })?;
                    }

                    if let Some(events) = item.events() {
                        writer.write_subtitle("Events")?;
                        events.into_iter().try_for_each(|(item, comments)| {
                            writer.write_heading(&item.name.name)?;
                            writer.write_section(item, comments)
                        })?;
                    }

                    if let Some(errors) = item.errors() {
                        writer.write_subtitle("Errors")?;
                        errors.into_iter().try_for_each(|(item, comments)| {
                            writer.write_heading(&item.name.name)?;
                            writer.write_section(item, comments)
                        })?;
                    }

                    if let Some(structs) = item.structs() {
                        writer.write_subtitle("Structs")?;
                        structs.into_iter().try_for_each(|(item, comments)| {
                            writer.write_heading(&item.name.name)?;
                            writer.write_section(item, comments)
                        })?;
                    }

                    if let Some(enums) = item.enums() {
                        writer.write_subtitle("Enums")?;
                        enums.into_iter().try_for_each(|(item, comments)| {
                            writer.write_heading(&item.name.name)?;
                            writer.write_section(item, comments)
                        })?;
                    }
                }
                ParseSource::Variable(var) => {
                    writer.write_title(&var.name.name)?;
                    writer.write_section(var, &item.comments)?;
                }
                ParseSource::Event(event) => {
                    writer.write_title(&event.name.name)?;
                    writer.write_section(event, &item.comments)?;
                }
                ParseSource::Error(error) => {
                    writer.write_title(&error.name.name)?;
                    writer.write_section(error, &item.comments)?;
                }
                ParseSource::Struct(structure) => {
                    writer.write_title(&structure.name.name)?;
                    writer.write_section(structure, &item.comments)?;
                }
                ParseSource::Enum(enumerable) => {
                    writer.write_title(&enumerable.name.name)?;
                    writer.write_section(enumerable, &item.comments)?;
                }
                ParseSource::Type(ty) => {
                    writer.write_title(&ty.name.name)?;
                    writer.write_section(ty, &item.comments)?;
                }
                ParseSource::Function(func) => {
                    // TODO: cleanup
                    // Write function name
                    let func_name =
                        func.name.as_ref().map_or(func.ty.to_string(), |n| n.name.to_owned());
                    writer.write_heading(&func_name)?;
                    writer.writeln()?;

                    // Write function docs
                    writer.writeln_doc(
                        item.comments.exclude_tags(&[CommentTag::Param, CommentTag::Return]),
                    )?;

                    // Write function header
                    writer.write_code(func)?;

                    // Write function parameter comments in a table
                    let params =
                        func.params.iter().filter_map(|p| p.1.as_ref()).collect::<Vec<_>>();
                    writer.try_write_param_table(CommentTag::Param, &params, &item.comments)?;

                    // Write function parameter comments in a table
                    let returns =
                        func.returns.iter().filter_map(|p| p.1.as_ref()).collect::<Vec<_>>();
                    writer.try_write_param_table(CommentTag::Return, &returns, &item.comments)?;

                    writer.writeln()?;
                }
            };
        }

        Ok(writer.finish())
    }
}
