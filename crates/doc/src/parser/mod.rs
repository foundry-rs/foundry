//! The parser module.

use solar::parse::ast;

/// Parser error.
pub mod error;

/// Owned source types.
pub mod source;

/// Parser item.
mod item;
pub use item::{ParseItem, ParseSource};

/// Doc comment.
mod comment;
pub use comment::{Comment, CommentTag, Comments, CommentsRef};

use source::*;

/// The documentation parser.
///
/// Walks the solar AST and extracts [`ParseItem`]s with owned source data and doc comments.
#[derive(Debug)]
pub struct Parser {
    /// Parsed results.
    items: Vec<ParseItem>,
    /// The source code string of the file being parsed.
    source: String,
    /// The global byte offset of this file's first byte in solar's source map.
    ///
    /// Solar uses a global `SourceMap` where each file's `BytePos` values start at
    /// `file.start_pos` rather than 0. All span `lo`/`hi` values must be offset by this
    /// amount before indexing into `self.source`.
    file_start: usize,
    /// Tab width used to format code.
    tab_width: usize,
}

impl Parser {
    /// Create a new instance of [Parser].
    ///
    /// `file_start` is `ast_source.file.start_pos.to_usize()` — the offset of the file's
    /// first byte in solar's global source map. Pass `0` in tests where you parse directly.
    pub const fn new(source: String, file_start: usize, tab_width: usize) -> Self {
        Self { items: Vec::new(), source, file_start, tab_width }
    }

    /// Parse a solar source unit and return the parsed items.
    pub fn parse(mut self, source_unit: &ast::SourceUnit<'_>) -> Vec<ParseItem> {
        for item in source_unit.items.iter() {
            if let Some(parsed) = self.parse_item(item) {
                self.items.push(parsed);
            }
        }
        self.items
    }

    /// Parse a single solar AST item into a [ParseItem].
    fn parse_item(&self, item: &ast::Item<'_>) -> Option<ParseItem> {
        let docs = Self::parse_docs(&item.docs);
        let span = item.span;

        match &item.kind {
            ast::ItemKind::Contract(contract) => {
                let source = self.parse_contract(contract);
                let code = self.extract_code(span);
                let mut parse_item =
                    ParseItem::new(ParseSource::Contract(source)).with_comments(docs);

                // Parse children
                let mut children = Vec::new();
                for child in contract.body.iter() {
                    if let Some(parsed) = self.parse_item(child) {
                        children.push(parsed);
                    }
                }
                parse_item.children = children;
                parse_item.code = code;
                Some(parse_item)
            }
            ast::ItemKind::Function(func) => {
                let source = self.parse_function(func);
                let code = self.extract_prototype_code(func);
                Some(
                    ParseItem::new(ParseSource::Function(source))
                        .with_comments(docs)
                        .with_code(code),
                )
            }
            ast::ItemKind::Variable(var) => {
                let source = self.parse_variable(var);
                let code = self.extract_code(span);
                Some(
                    ParseItem::new(ParseSource::Variable(source))
                        .with_comments(docs)
                        .with_code(code),
                )
            }
            ast::ItemKind::Event(event) => {
                let source = self.parse_event(event);
                let code = self.extract_code(span);
                Some(ParseItem::new(ParseSource::Event(source)).with_comments(docs).with_code(code))
            }
            ast::ItemKind::Error(err) => {
                let source = self.parse_error(err);
                let code = self.extract_code(span);
                Some(ParseItem::new(ParseSource::Error(source)).with_comments(docs).with_code(code))
            }
            ast::ItemKind::Struct(strukt) => {
                let source = self.parse_struct(strukt);
                let code = self.extract_code(span);
                Some(
                    ParseItem::new(ParseSource::Struct(source)).with_comments(docs).with_code(code),
                )
            }
            ast::ItemKind::Enum(enm) => {
                let source = self.parse_enum(enm);
                let code = self.extract_code(span);
                Some(ParseItem::new(ParseSource::Enum(source)).with_comments(docs).with_code(code))
            }
            ast::ItemKind::Udvt(udvt) => {
                let source = TypeSource { name: udvt.name.to_string() };
                let code = self.extract_code(span);
                Some(ParseItem::new(ParseSource::Type(source)).with_comments(docs).with_code(code))
            }
            // Skip pragmas, imports, using directives
            _ => None,
        }
    }

    fn parse_contract(&self, contract: &ast::ItemContract<'_>) -> ContractSource {
        let kind = match contract.kind {
            ast::ContractKind::Contract => ContractKind::Contract,
            ast::ContractKind::AbstractContract => ContractKind::Abstract,
            ast::ContractKind::Interface => ContractKind::Interface,
            ast::ContractKind::Library => ContractKind::Library,
        };

        let bases = contract
            .bases
            .iter()
            .map(|base| {
                let full_name = base.name.to_string();
                let ident = base.name.last().name.to_string();
                BaseInfo { name: full_name, ident }
            })
            .collect();

        ContractSource { name: contract.name.to_string(), kind, bases }
    }

    fn parse_function(&self, func: &ast::ItemFunction<'_>) -> FunctionSource {
        let name = func.header.name.map(|n| n.to_string());
        let kind = func.kind.to_string();
        let params = self.parse_var_defs(&func.header.parameters);
        let returns =
            func.header.returns.as_deref().map(|r| self.parse_var_defs(r)).unwrap_or_default();
        FunctionSource { name, kind, params, returns }
    }

    fn parse_variable(&self, var: &ast::VariableDefinition<'_>) -> VariableSource {
        let name = var.name.map(|n| n.to_string()).unwrap_or_default();

        let mut attrs = Vec::new();
        if let Some(m) = var.mutability {
            match m {
                ast::VarMut::Constant => attrs.push(VariableAttr::Constant),
                ast::VarMut::Immutable => attrs.push(VariableAttr::Immutable),
            }
        }

        VariableSource { name, attrs }
    }

    fn parse_event(&self, event: &ast::ItemEvent<'_>) -> EventSource {
        let fields = self.parse_var_defs(&event.parameters);
        EventSource { name: event.name.to_string(), fields }
    }

    fn parse_error(&self, err: &ast::ItemError<'_>) -> ErrorSource {
        let fields = self.parse_var_defs(&err.parameters);
        ErrorSource { name: err.name.to_string(), fields }
    }

    fn parse_struct(&self, strukt: &ast::ItemStruct<'_>) -> StructSource {
        let fields = self.parse_var_defs(strukt.fields);
        StructSource { name: strukt.name.to_string(), fields }
    }

    /// Parse a list of variable definitions into [ParamInfo].
    fn parse_var_defs(&self, vars: &[ast::VariableDefinition<'_>]) -> Vec<ParamInfo> {
        vars.iter()
            .map(|v| ParamInfo { name: v.name.map(|n| n.to_string()), ty: self.type_string(&v.ty) })
            .collect()
    }

    /// Extract the type as a string from the source code.
    fn type_string(&self, ty: &ast::Type<'_>) -> String {
        let lo = ty.span.lo().to_usize().saturating_sub(self.file_start);
        let hi = ty.span.hi().to_usize().saturating_sub(self.file_start);
        if lo < self.source.len() && hi <= self.source.len() && lo < hi {
            self.source[lo..hi].to_string()
        } else {
            String::new()
        }
    }

    fn parse_enum(&self, enm: &ast::ItemEnum<'_>) -> EnumSource {
        let variants = enm.variants.iter().map(|v| v.to_string()).collect();
        EnumSource { name: enm.name.to_string(), variants }
    }

    /// Parse doc comments from solar's [ast::DocComments] into our [Comments] type.
    fn parse_docs(docs: &ast::DocComments<'_>) -> Comments {
        if docs.is_empty() {
            return Comments::default();
        }
        Comments::from_doc_lines(docs.iter().map(|d| d.symbol.as_str()))
    }

    /// Extract a code snippet from the source for the given span.
    fn extract_code(&self, span: ast::Span) -> String {
        let lo = span.lo().to_usize().saturating_sub(self.file_start);
        let hi = span.hi().to_usize().saturating_sub(self.file_start);
        if lo < self.source.len() && hi <= self.source.len() && lo < hi {
            let code = &self.source[lo..hi];
            self.dedent(code)
        } else {
            String::new()
        }
    }

    /// Extract only the function prototype (excluding the body) from the source.
    fn extract_prototype_code(&self, func: &ast::ItemFunction<'_>) -> String {
        let lo = func.header.span.lo().to_usize().saturating_sub(self.file_start);
        let hi = func.header.span.hi().to_usize().saturating_sub(self.file_start);
        if lo < self.source.len() && hi <= self.source.len() && lo < hi {
            let mut code = self.source[lo..hi].to_string();
            code.push(';');
            self.dedent(&code)
        } else {
            String::new()
        }
    }

    /// Remove one level of indentation from code.
    fn dedent(&self, code: &str) -> String {
        let prefix = &" ".repeat(self.tab_width);
        code.lines()
            .map(|line| line.strip_prefix(prefix).unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_source(src: &str) -> Vec<ParseItem> {
        use solar::parse::{
            Parser as SolarParser,
            ast::{Arena, interface},
            interface::Session,
        };

        let sess =
            Session::builder().with_silent_emitter(Some("test parse failed".to_string())).build();

        sess.enter(|| -> Vec<ParseItem> {
            let arena = Arena::new();
            let mut parser = SolarParser::from_source_code(
                &sess,
                &arena,
                interface::source_map::FileName::Custom("test".to_string()),
                src.to_string(),
            )
            .expect("failed to create parser");

            let source_unit = parser.parse_file().map_err(|e| e.emit()).expect("failed to parse");

            // file_start=0: when parsing directly, solar's BytePos values start at 0.
            let doc = Parser::new(src.to_string(), 0, 4);
            doc.parse(&source_unit)
        })
    }

    macro_rules! test_single_unit {
        ($test:ident, $src:expr, $variant:ident $identity:expr) => {
            #[test]
            fn $test() {
                let items = parse_source($src);
                assert_eq!(items.len(), 1);
                let item = items.first().unwrap();
                assert!(item.comments.is_empty());
                assert!(item.children.is_empty());
                assert_eq!(item.source.ident(), $identity);
                assert!(matches!(item.source, ParseSource::$variant(_)));
            }
        };
    }

    #[test]
    fn empty_source() {
        assert_eq!(parse_source(""), vec![]);
    }

    test_single_unit!(single_function, "function someFn() { }", Function "someFn");
    test_single_unit!(single_variable, "uint256 constant VALUE = 0;", Variable "VALUE");
    test_single_unit!(single_event, "event SomeEvent();", Event "SomeEvent");
    test_single_unit!(single_error, "error SomeError();", Error "SomeError");
    test_single_unit!(single_struct, "struct SomeStruct { }", Struct "SomeStruct");
    test_single_unit!(single_enum, "enum SomeEnum { SOME, OTHER }", Enum "SomeEnum");
    test_single_unit!(single_contract, "contract Contract { }", Contract "Contract");

    #[test]
    fn multiple_shallow_contracts() {
        let items = parse_source(
            r"
            contract A { }
            contract B { }
            contract C { }
        ",
        );
        assert_eq!(items.len(), 3);

        let first_item = items.first().unwrap();
        assert!(matches!(first_item.source, ParseSource::Contract(_)));
        assert_eq!(first_item.source.ident(), "A");

        let first_item = items.get(1).unwrap();
        assert!(matches!(first_item.source, ParseSource::Contract(_)));
        assert_eq!(first_item.source.ident(), "B");

        let first_item = items.get(2).unwrap();
        assert!(matches!(first_item.source, ParseSource::Contract(_)));
        assert_eq!(first_item.source.ident(), "C");
    }

    #[test]
    fn contract_with_children_items() {
        let items = parse_source(
            r"
            event TopLevelEvent();

            contract Contract {
                event ContractEvent();
                error ContractError();
                struct ContractStruct { }
                enum ContractEnum { }

                uint256 constant CONTRACT_CONSTANT = 0;
                bool contractVar;

                function contractFunction(uint256) external returns (uint256) {
                    bool localVar; // must be ignored
                }
            }
        ",
        );

        assert_eq!(items.len(), 2);

        let event = items.first().unwrap();
        assert!(event.comments.is_empty());
        assert!(event.children.is_empty());
        assert_eq!(event.source.ident(), "TopLevelEvent");
        assert!(matches!(event.source, ParseSource::Event(_)));

        let contract = items.get(1).unwrap();
        assert!(contract.comments.is_empty());
        assert_eq!(contract.children.len(), 7);
        assert_eq!(contract.source.ident(), "Contract");
        assert!(matches!(contract.source, ParseSource::Contract(_)));
        assert!(contract.children.iter().all(|ch| ch.children.is_empty()));
        assert!(contract.children.iter().all(|ch| ch.comments.is_empty()));
    }

    #[test]
    fn contract_with_fallback() {
        let items = parse_source(
            r"
            contract Contract {
                fallback() external payable {}
            }
        ",
        );

        assert_eq!(items.len(), 1);

        let contract = items.first().unwrap();
        assert!(contract.comments.is_empty());
        assert_eq!(contract.children.len(), 1);
        assert_eq!(contract.source.ident(), "Contract");
        assert!(matches!(contract.source, ParseSource::Contract(_)));

        let fallback = contract.children.first().unwrap();
        assert_eq!(fallback.source.ident(), "fallback");
        assert!(matches!(fallback.source, ParseSource::Function(_)));
    }

    #[test]
    fn overloaded_function_signatures() {
        let items = parse_source(
            r"
            interface IFoo {
                function process(address addr) external;
                function process(address[] calldata addrs) external;
                function process(address addr, uint256 value) external;
            }
        ",
        );
        assert_eq!(items.len(), 1);
        let contract = items.first().unwrap();
        assert_eq!(contract.children.len(), 3);
        let sigs: Vec<String> = contract.children.iter().map(|ch| ch.source.signature()).collect();
        assert_eq!(sigs[0], "process(address)", "first overload");
        assert_eq!(sigs[1], "process(address[])", "second overload (array)");
        assert_eq!(sigs[2], "process(address,uint256)", "third overload");
    }

    #[test]
    fn contract_with_doc_comments() {
        let items = parse_source(
            r"
            pragma solidity ^0.8.19;
            /// @notice    Cool contract
            /**
             * @dev line one
             *    line 2
             */
            contract Test {
                /// my function
                ///       i like whitespace
                function test() {}
            }
        ",
        );

        assert_eq!(items.len(), 1);

        let contract = items.first().unwrap();
        assert_eq!(contract.comments.len(), 2);
        assert_eq!(
            *contract.comments.first().unwrap(),
            Comment::new(CommentTag::Notice, "Cool contract".to_owned())
        );
        assert_eq!(
            *contract.comments.get(1).unwrap(),
            Comment::new(CommentTag::Dev, "line one\nline 2".to_owned())
        );

        let function = contract.children.first().unwrap();
        assert_eq!(
            *function.comments.first().unwrap(),
            Comment::new(CommentTag::Notice, "my function\ni like whitespace".to_owned())
        );
    }
}
