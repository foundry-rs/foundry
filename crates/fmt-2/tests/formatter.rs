use forge_fmt_2::FormatterConfig;
use itertools::Itertools;
use std::{
    fmt, fs,
    path::{Path, PathBuf},
};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[track_caller]
fn format(source: &str, path: &Path, config: FormatterConfig) -> String {
    match forge_fmt_2::format_source(source, Some(path), config) {
        Ok(formatted) => formatted,
        Err(e) => panic!("failed to format {path:?}: {e}"),
    }
}

#[track_caller]
fn assert_eof(content: &str) {
    assert!(content.ends_with('\n'), "missing trailing newline");
    assert!(!content.ends_with("\n\n"), "extra trailing newline");
}

#[derive(Eq)]
struct PrettyString<'a>(&'a str);

impl PartialEq for PrettyString<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.lines().eq(other.0.lines())
    }
}

impl fmt::Debug for PrettyString<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0, f)
    }
}

fn enable_tracing() {
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .with_test_writer()
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

fn tests_dir() -> PathBuf {
    // Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("fmt/testdata")
}

fn test_directory(base_name: &str) {
    enable_tracing();
    let dir = tests_dir().join(base_name);
    let original = fs::read_to_string(dir.join("original.sol")).unwrap();
    for res in dir.read_dir().unwrap() {
        let entry = res.unwrap();
        let path = entry.path();

        let filename = path.file_name().and_then(|name| name.to_str()).unwrap();
        if filename == "original.sol" {
            continue;
        }
        assert!(path.is_file(), "expected file: {path:?}");
        assert!(filename.ends_with("fmt.sol"), "unknown file: {path:?}");

        let source = fs::read_to_string(&path).unwrap();

        // The majority of the tests were written with the assumption that the default value for max
        // line length is `80`. Preserve that to avoid rewriting test logic.
        let default_config = FormatterConfig { line_length: 80, ..Default::default() };

        let mut config = toml::Value::try_from(default_config).unwrap();
        let config_table = config.as_table_mut().unwrap();
        let mut lines = source.split('\n').peekable();
        let mut line_num = 1;
        while let Some(&line) = lines.peek() {
            let Some(entry) = line
                .strip_prefix("//")
                .and_then(|line| line.trim().strip_prefix("config:"))
                .map(str::trim)
            else {
                break;
            };

            let values = match toml::from_str::<toml::Value>(entry) {
                Ok(toml::Value::Table(table)) => table,
                r => panic!("invalid fmt config item in {filename} at {line_num}: {r:?}"),
            };
            config_table.extend(values);

            line_num += 1;
            lines.next();
        }
        let config = config
            .try_into()
            .unwrap_or_else(|err| panic!("invalid test config for {filename}: {err}"));

        test_formatter(&path, filename, config, &original, &lines.join("\n"));
    }
}

fn test_formatter(
    path: &Path,
    filename: &str,
    config: FormatterConfig,
    source: &str,
    expected_source: &str,
) {
    assert_eof(expected_source);

    let source_formatted = format(source, &path.with_file_name("original.sol"), config.clone());
    assert_eof(&source_formatted);
    similar_asserts::assert_eq!(
        PrettyString(&source_formatted),
        PrettyString(expected_source),
        "{filename}: formatted source does not match expected source"
    );

    let expected_formatted = format(expected_source, path, config);
    similar_asserts::assert_eq!(
        PrettyString(&expected_formatted),
        PrettyString(expected_source),
        "{filename}: expected source is not formatted"
    );
}

fn test_all_dirs_are_declared(dirs: &[&str]) {
    let mut undeclared = vec![];
    for actual_dir in tests_dir().read_dir().unwrap().filter_map(Result::ok) {
        let path = actual_dir.path();
        assert!(path.is_dir(), "expected directory: {path:?}");
        let actual_dir_name = path.file_name().unwrap().to_str().unwrap();
        if !dirs.contains(&actual_dir_name) {
            undeclared.push(actual_dir_name.to_string());
        }
    }
    if !undeclared.is_empty() {
        panic!("the following test directories are not declared in the test suite macro call: {undeclared:#?}");
    }
}

macro_rules! fmt_tests {
    ($($dir:ident),+ $(,)?) => {
        #[test]
        fn all_dirs_are_declared() {
            test_all_dirs_are_declared(&[$(stringify!($dir)),*]);
        }

        $(
            #[allow(non_snake_case)]
            #[test]
            fn $dir() {
                test_directory(stringify!($dir));
            }
        )+
    };
}

fmt_tests! {
    Annotation,
    ArrayExpressions,
    BlockComments,
    BlockCommentsFunction,
    ConditionalOperatorExpression,
    ConstructorDefinition,
    ConstructorModifierStyle,
    ContractDefinition,
    DocComments,
    DoWhileStatement,
    EmitStatement,
    EnumDefinition,
    EnumVariants,
    ErrorDefinition,
    EventDefinition,
    ForStatement,
    FunctionCall,
    FunctionCallArgsStatement,
    FunctionDefinition,
    FunctionDefinitionWithFunctionReturns,
    FunctionType,
    HexUnderscore,
    IfStatement,
    IfStatement2,
    ImportDirective,
    InlineDisable,
    IntTypes,
    LiteralExpression,
    MappingType,
    ModifierDefinition,
    NamedFunctionCallExpression,
    NumberLiteralUnderscore,
    OperatorExpressions,
    PragmaDirective,
    Repros,
    ReturnStatement,
    RevertNamedArgsStatement,
    RevertStatement,
    SimpleComments,
    SortedImports,
    StatementBlock,
    StructDefinition,
    ThisExpression,
    TrailingComma,
    TryStatement,
    TypeDefinition,
    UnitExpression,
    UsingDirective,
    VariableAssignment,
    VariableDefinition,
    WhileStatement,
    Yul,
    YulStrings,
}
