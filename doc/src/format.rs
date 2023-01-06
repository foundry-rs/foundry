use crate::{
    helpers::{comments_by_tag, exclude_comment_tags},
    output::DocOutput,
    parser::{DocElement, DocItem},
    writer::BufWriter,
};
use itertools::Itertools;
use solang_parser::{
    doccomment::DocCommentTag,
    pt::{
        Base, EnumDefinition, ErrorDefinition, EventDefinition, FunctionDefinition,
        StructDefinition, VariableDefinition,
    },
};

pub(crate) type DocResult = Result<String, std::fmt::Error>;

#[auto_impl::auto_impl(&)]
pub(crate) trait DocFormat {
    fn doc(&self) -> DocResult;
}

impl DocFormat for String {
    fn doc(&self) -> DocResult {
        Ok(self.to_owned())
    }
}

impl DocFormat for DocCommentTag {
    fn doc(&self) -> DocResult {
        Ok(self.value.to_owned())
    }
}

impl DocFormat for Vec<&DocCommentTag> {
    fn doc(&self) -> DocResult {
        Ok(self.iter().map(|c| DocCommentTag::doc(*c)).collect::<Result<Vec<_>, _>>()?.join("\n\n"))
    }
}

impl DocFormat for Vec<DocCommentTag> {
    fn doc(&self) -> DocResult {
        Ok(self.iter().map(DocCommentTag::doc).collect::<Result<Vec<_>, _>>()?.join("\n\n"))
    }
}

// TODO: remove?
impl DocFormat for Base {
    fn doc(&self) -> DocResult {
        Ok(self.name.identifiers.iter().map(|ident| ident.name.to_owned()).join("."))
    }
}

impl DocFormat for Vec<Base> {
    fn doc(&self) -> DocResult {
        Ok(self.iter().map(|base| base.doc()).collect::<Result<Vec<_>, _>>()?.join(", "))
    }
}

// TODO: remove
impl DocFormat for FunctionDefinition {
    fn doc(&self) -> DocResult {
        let name = self.name.as_ref().map_or(self.ty.to_string(), |n| n.name.to_owned());
        DocOutput::H3(&name).doc()
    }
}

impl DocFormat for EventDefinition {
    fn doc(&self) -> DocResult {
        DocOutput::H3(&self.name.name).doc()
    }
}

impl DocFormat for ErrorDefinition {
    fn doc(&self) -> DocResult {
        DocOutput::H3(&self.name.name).doc()
    }
}

impl DocFormat for StructDefinition {
    fn doc(&self) -> DocResult {
        DocOutput::H3(&self.name.name).doc()
    }
}

impl DocFormat for EnumDefinition {
    fn doc(&self) -> DocResult {
        DocOutput::H3(&self.name.name).doc()
    }
}

impl DocFormat for VariableDefinition {
    fn doc(&self) -> DocResult {
        DocOutput::H3(&self.name.name).doc()
    }
}

impl DocFormat for DocItem {
    fn doc(&self) -> DocResult {
        let mut writer = BufWriter::default();

        match &self.element {
            DocElement::Contract(contract) => {
                writer.write_title(&contract.name.name)?;

                if !contract.base.is_empty() {
                    // TODO:
                    // Ok(self.lookup_contract_base(docs.as_ref(), base)?.unwrap_or(base.doc()))
                    // TODO: should be a name & perform lookup
                    let bases = contract
                        .base
                        .iter()
                        .map(|base| base.doc())
                        .collect::<Result<Vec<_>, _>>()?;
                    writer.write_bold("Inherits:")?;
                    writer.write_raw(bases.join(", "))?;
                    writer.writeln()?;
                }

                writer.write_raw(self.comments.doc()?)?;

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
                            exclude_comment_tags(comments, vec!["param", "return"]).doc()?,
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
            DocElement::Variable(var) => {
                writer.write_title(&var.name.name)?;
                writer.write_section(var, &self.comments)?;
            }
            DocElement::Event(event) => {
                writer.write_title(&event.name.name)?;
                writer.write_section(event, &self.comments)?;
            }
            DocElement::Error(error) => {
                writer.write_title(&error.name.name)?;
                writer.write_section(error, &self.comments)?;
            }
            DocElement::Struct(structure) => {
                writer.write_title(&structure.name.name)?;
                writer.write_section(structure, &self.comments)?;
            }
            DocElement::Enum(enumerable) => {
                writer.write_title(&enumerable.name.name)?;
                writer.write_section(enumerable, &self.comments)?;
            }
            DocElement::Function(func) => {
                // TODO: cleanup
                // Write function name
                let func_name =
                    func.name.as_ref().map_or(func.ty.to_string(), |n| n.name.to_owned());
                writer.write_heading(&func_name)?;
                writer.writeln()?;

                // Write function docs
                writer.write_raw(
                    exclude_comment_tags(&self.comments, vec!["param", "return"]).doc()?,
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
