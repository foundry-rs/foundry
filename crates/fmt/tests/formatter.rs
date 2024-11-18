use forge_fmt::{format_to, parse, solang_ext::AstEq, FormatterConfig};
use itertools::Itertools;
use std::{fs, path::PathBuf};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

fn tracing() {
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .with_test_writer()
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

fn test_directory(base_name: &str, test_config: TestConfig) {
    tracing();
    let mut original = None;

    let tests =
        fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join(base_name))
            .unwrap()
            .filter_map(|path| {
                let path = path.unwrap().path();
                let source = fs::read_to_string(&path).unwrap();

                if let Some(filename) = path.file_name().and_then(|name| name.to_str()) {
                    if filename == "original.sol" {
                        original = Some(source);
                    } else if filename
                        .strip_suffix("fmt.sol")
                        .map(|filename| filename.strip_suffix('.'))
                        .is_some()
                    {
                        // The majority of the tests were written with the assumption
                        // that the default value for max line length is `80`.
                        // Preserve that to avoid rewriting test logic.
                        let default_config =
                            FormatterConfig { line_length: 80, ..Default::default() };

                        let mut config = toml::Value::try_from(default_config).unwrap();
                        let config_table = config.as_table_mut().unwrap();
                        let mut lines = source.split('\n').peekable();
                        let mut line_num = 1;
                        while let Some(line) = lines.peek() {
                            let entry = line
                                .strip_prefix("//")
                                .and_then(|line| line.trim().strip_prefix("config:"))
                                .map(str::trim);
                            let entry = if let Some(entry) = entry { entry } else { break };

                            let values = match toml::from_str::<toml::Value>(entry) {
                                Ok(toml::Value::Table(table)) => table,
                                _ => panic!("Invalid config item in {filename} at {line_num}"),
                            };
                            config_table.extend(values);

                            line_num += 1;
                            lines.next();
                        }
                        let config = config
                            .try_into()
                            .unwrap_or_else(|err| panic!("Invalid config for {filename}: {err}"));

                        return Some((filename.to_string(), config, lines.join("\n")))
                    }
                }

                None
            })
            .collect::<Vec<_>>();

    for (filename, config, formatted) in tests {
        test_formatter(
            &filename,
            config,
            original.as_ref().expect("original.sol not found"),
            &formatted,
            test_config,
        );
    }
}

fn assert_eof(content: &str) {
    assert!(content.ends_with('\n') && !content.ends_with("\n\n"));
}

fn test_formatter(
    filename: &str,
    config: FormatterConfig,
    source: &str,
    expected_source: &str,
    test_config: TestConfig,
) {
    #[derive(Eq)]
    struct PrettyString(String);

    impl PartialEq for PrettyString {
        fn eq(&self, other: &Self) -> bool {
            self.0.lines().eq(other.0.lines())
        }
    }

    impl std::fmt::Debug for PrettyString {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    assert_eof(expected_source);

    let source_parsed = parse(source).unwrap();
    let expected_parsed = parse(expected_source).unwrap();

    if !test_config.skip_compare_ast_eq && !source_parsed.pt.ast_eq(&expected_parsed.pt) {
        similar_asserts::assert_eq!(
            source_parsed.pt,
            expected_parsed.pt,
            "(formatted Parse Tree == expected Parse Tree) in {}",
            filename
        );
    }

    let expected = PrettyString(expected_source.to_string());

    let mut source_formatted = String::new();
    format_to(&mut source_formatted, source_parsed, config.clone()).unwrap();
    assert_eof(&source_formatted);

    let source_formatted = PrettyString(source_formatted);

    similar_asserts::assert_eq!(
        source_formatted,
        expected,
        "(formatted == expected) in {}",
        filename
    );

    let mut expected_formatted = String::new();
    format_to(&mut expected_formatted, expected_parsed, config).unwrap();
    assert_eof(&expected_formatted);

    let expected_formatted = PrettyString(expected_formatted);

    similar_asserts::assert_eq!(
        expected_formatted,
        expected,
        "(formatted == expected) in {}",
        filename
    );
}

#[derive(Clone, Copy, Default)]
struct TestConfig {
    /// Whether to compare the formatted source code AST with the original AST
    skip_compare_ast_eq: bool,
}

impl TestConfig {
    fn skip_compare_ast_eq() -> Self {
        Self { skip_compare_ast_eq: true }
    }
}

macro_rules! test_dir {
    ($dir:ident $(,)?) => {
        test_dir!($dir, Default::default());
    };
    ($dir:ident, $config:expr $(,)?) => {
        #[allow(non_snake_case)]
        #[test]
        fn $dir() {
            test_directory(stringify!($dir), $config);
        }
    };
}

macro_rules! test_directories {
    ($($dir:ident),+ $(,)?) => {$(
        test_dir!($dir);
    )+};
}

test_directories! {
    ConstructorDefinition,
    ConstructorModifierStyle,
    ContractDefinition,
    DocComments,
    EnumDefinition,
    ErrorDefinition,
    EventDefinition,
    FunctionDefinition,
    FunctionDefinitionWithFunctionReturns,
    FunctionType,
    ImportDirective,
    ModifierDefinition,
    StatementBlock,
    StructDefinition,
    TypeDefinition,
    UsingDirective,
    VariableDefinition,
    OperatorExpressions,
    WhileStatement,
    DoWhileStatement,
    ForStatement,
    IfStatement,
    IfStatement2,
    VariableAssignment,
    FunctionCallArgsStatement,
    RevertStatement,
    RevertNamedArgsStatement,
    ReturnStatement,
    TryStatement,
    ConditionalOperatorExpression,
    NamedFunctionCallExpression,
    ArrayExpressions,
    UnitExpression,
    ThisExpression,
    SimpleComments,
    LiteralExpression,
    Yul,
    YulStrings,
    IntTypes,
    InlineDisable,
    NumberLiteralUnderscore,
    HexUnderscore,
    FunctionCall,
    TrailingComma,
    PragmaDirective,
    Annotation,
    MappingType,
    EmitStatement,
    Repros,
    BlockComments,
    BlockCommentsFunction,
    EnumVariants,
}

test_dir!(SortedImports, TestConfig::skip_compare_ast_eq());
