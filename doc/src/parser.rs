use forge_fmt::{Visitable, Visitor};
use solang_parser::{
    doccomment::{parse_doccomments, DocComment, DocCommentTag},
    pt::{
        Comment, ContractDefinition, EnumDefinition, ErrorDefinition, EventDefinition,
        FunctionDefinition, Loc, SourceUnit, SourceUnitPart, StructDefinition, VariableDefinition,
    },
};
use thiserror::Error;

/// The parser error
#[derive(Error, Debug)]
pub(crate) enum DocParserError {}

// TODO:
type Result<T, E = DocParserError> = std::result::Result<T, E>;

#[derive(Debug)]
pub(crate) struct DocParser {
    pub(crate) items: Vec<DocItem>,
    comments: Vec<Comment>,
    start_at: usize,
    context: DocContext,
}

#[derive(Debug, Default)]
struct DocContext {
    parent: Option<DocItem>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct DocItem {
    pub(crate) element: DocElement,
    pub(crate) comments: Vec<DocCommentTag>,
    pub(crate) children: Vec<DocItem>,
}

macro_rules! filter_children_fn {
    ($vis:vis fn $name:ident(&self, $variant:ident) -> $ret:ty) => {
        $vis fn $name<'a>(&'a self) -> Option<Vec<(&'a $ret, &'a Vec<DocCommentTag>)>> {
            let items = self.children.iter().filter_map(|item| match item.element {
                DocElement::$variant(ref inner) => Some((inner, &item.comments)),
                _ => None,
            });
            let items = items.collect::<Vec<_>>();
            if !items.is_empty() {
                Some(items)
            } else {
                None
            }
        }
    };
}

impl DocItem {
    filter_children_fn!(pub(crate) fn variables(&self, Variable) -> VariableDefinition);
    filter_children_fn!(pub(crate) fn functions(&self, Function) -> FunctionDefinition);
    filter_children_fn!(pub(crate) fn events(&self, Event) -> EventDefinition);
    filter_children_fn!(pub(crate) fn errors(&self, Error) -> ErrorDefinition);
    filter_children_fn!(pub(crate) fn structs(&self, Struct) -> StructDefinition);
    filter_children_fn!(pub(crate) fn enums(&self, Enum) -> EnumDefinition);

    pub(crate) fn filename(&self) -> String {
        let prefix = match self.element {
            DocElement::Contract(_) => "contract",
            DocElement::Function(_) => "function",
            DocElement::Variable(_) => "variable",
            DocElement::Event(_) => "event",
            DocElement::Error(_) => "error",
            DocElement::Struct(_) => "struct",
            DocElement::Enum(_) => "enum",
        };
        let ident = self.element.ident();
        format!("{prefix}.{ident}.md")
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum DocElement {
    Contract(Box<ContractDefinition>),
    Function(FunctionDefinition),
    Variable(VariableDefinition),
    Event(EventDefinition),
    Error(ErrorDefinition),
    Struct(StructDefinition),
    Enum(EnumDefinition),
}

impl DocElement {
    pub(crate) fn ident(&self) -> String {
        match self {
            DocElement::Contract(contract) => contract.name.name.to_owned(),
            DocElement::Variable(var) => var.name.name.to_owned(),
            DocElement::Event(event) => event.name.name.to_owned(),
            DocElement::Error(error) => error.name.name.to_owned(),
            DocElement::Struct(structure) => structure.name.name.to_owned(),
            DocElement::Enum(enumerable) => enumerable.name.name.to_owned(),
            DocElement::Function(func) => {
                func.name.as_ref().map(|name| name.name.to_owned()).unwrap_or_default() // TODO: <unknown>
            }
        }
    }
}

impl DocParser {
    pub(crate) fn new(comments: Vec<Comment>) -> Self {
        DocParser { items: vec![], comments, start_at: 0, context: Default::default() }
    }

    fn with_parent(
        &mut self,
        mut parent: DocItem,
        mut visit: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<DocItem> {
        let curr = self.context.parent.take();
        self.context.parent = Some(parent);
        visit(self)?;
        parent = self.context.parent.take().unwrap();
        self.context.parent = curr;
        Ok(parent)
    }

    fn add_element_to_parent(&mut self, element: DocElement, loc: Loc) {
        let child = DocItem { comments: self.parse_docs(loc.start()), element, children: vec![] };
        if let Some(parent) = self.context.parent.as_mut() {
            parent.children.push(child);
        } else {
            self.items.push(child);
        }
        self.start_at = loc.end();
    }

    fn parse_docs(&mut self, end: usize) -> Vec<DocCommentTag> {
        let mut res = vec![];
        for comment in parse_doccomments(&self.comments, self.start_at, end) {
            match comment {
                DocComment::Line { comment } => res.push(comment),
                DocComment::Block { comments } => res.extend(comments.into_iter()),
            }
        }
        res
    }
}

impl Visitor for DocParser {
    type Error = DocParserError;

    fn visit_source_unit(&mut self, source_unit: &mut SourceUnit) -> Result<()> {
        for source in source_unit.0.iter_mut() {
            match source {
                SourceUnitPart::ContractDefinition(def) => {
                    let contract = DocItem {
                        element: DocElement::Contract(def.clone()),
                        comments: self.parse_docs(def.loc.start()),
                        children: vec![],
                    };
                    self.start_at = def.loc.start();
                    let contract = self.with_parent(contract, |doc| {
                        def.parts.iter_mut().map(|d| d.visit(doc)).collect::<Result<Vec<_>>>()?;
                        Ok(())
                    })?;

                    self.start_at = def.loc.end();
                    self.items.push(contract);
                }
                SourceUnitPart::FunctionDefinition(func) => self.visit_function(func)?,
                SourceUnitPart::EventDefinition(event) => self.visit_event(event)?,
                SourceUnitPart::ErrorDefinition(error) => self.visit_error(error)?,
                SourceUnitPart::StructDefinition(structure) => self.visit_struct(structure)?,
                SourceUnitPart::EnumDefinition(enumerable) => self.visit_enum(enumerable)?,
                _ => {}
            };
        }

        Ok(())
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> Result<()> {
        self.add_element_to_parent(DocElement::Function(func.clone()), func.loc);
        Ok(())
    }

    fn visit_var_definition(&mut self, var: &mut VariableDefinition) -> Result<()> {
        self.add_element_to_parent(DocElement::Variable(var.clone()), var.loc);
        Ok(())
    }

    fn visit_event(&mut self, event: &mut EventDefinition) -> Result<()> {
        self.add_element_to_parent(DocElement::Event(event.clone()), event.loc);
        Ok(())
    }

    fn visit_error(&mut self, error: &mut ErrorDefinition) -> Result<()> {
        self.add_element_to_parent(DocElement::Error(error.clone()), error.loc);
        Ok(())
    }

    fn visit_struct(&mut self, structure: &mut StructDefinition) -> Result<()> {
        self.add_element_to_parent(DocElement::Struct(structure.clone()), structure.loc);
        Ok(())
    }

    fn visit_enum(&mut self, enumerable: &mut EnumDefinition) -> Result<()> {
        self.add_element_to_parent(DocElement::Enum(enumerable.clone()), enumerable.loc);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    // TODO: write tests w/ sol source code

    #[test]
    fn parse_docs() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("oz-governance")
            .join("Governor.sol");
        let target = fs::read_to_string(path).unwrap();
        let (mut source_pt, source_comments) = solang_parser::parse(&target, 1).unwrap();
        let mut d = DocParser::new(source_comments);
        assert!(source_pt.visit(&mut d).is_ok());
    }
}
