use forge_fmt::{DocCommentStyle, FormatterConfig};
use foundry_config::fmt::{IndentStyle, SingleLineBlockStyle};
use foundry_test_utils::init_tracing;
use snapbox::{Data, assert_data_eq, str};
use solar::{
    ast::{self, Visit},
    sema::Compiler,
};
use std::{
    fs,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::Arc,
};

#[track_caller]
fn format(source: &str, path: &Path, fmt_config: Arc<FormatterConfig>) -> String {
    let mut compiler = Compiler::new(
        solar::interface::Session::builder().with_buffer_emitter(Default::default()).build(),
    );

    match forge_fmt::format_source(source, Some(path), fmt_config, &mut compiler).into_result() {
        Ok(formatted) => formatted,
        Err(e) => panic!("failed to format {path:?}: {e}"),
    }
}

#[track_caller]
fn assert_eof(content: &str) {
    assert!(content.ends_with('\n'), "missing trailing newline");
    assert!(!content.ends_with("\n\n"), "extra trailing newline");
}

#[test]
fn binary_assignment_layout_ignores_operator_spacing() {
    for (line_length, source, expected) in [
        (
            105,
            r#"contract C {
    function f() external {
        p2pSupplyRate =
            p2pSupplyRate.mul(CompoundMath.WAD - shareOfTheDelta) +
            _params.poolRate.mul(shareOfTheDelta);
    }
}
"#,
            r#"contract C {
    function f() external {
        p2pSupplyRate = p2pSupplyRate.mul(CompoundMath.WAD - shareOfTheDelta)
            + _params.poolRate.mul(shareOfTheDelta);
    }
}
"#,
        ),
        (
            116,
            r#"contract C {
    function f() external {
        p2pSupplyGrowthFactor =
            p2pGrowthFactor -
            (_params.reserveFactor * (p2pGrowthFactor - poolSupplyGrowthFactor)) /
            MAX_BASIS_POINTS;
    }
}
"#,
            r#"contract C {
    function f() external {
        p2pSupplyGrowthFactor = p2pGrowthFactor
            - (_params.reserveFactor * (p2pGrowthFactor - poolSupplyGrowthFactor)) / MAX_BASIS_POINTS;
    }
}
"#,
        ),
        (
            120,
            r#"contract C {
    function f() external {
        uint256 poolTVL = (IERC20Detailed(address(_cpToken)).totalSupply() *
            _cpToken.getCurrentExchangeRate()) / 10**18;
    }
}
"#,
            r#"contract C {
    function f() external {
        uint256 poolTVL =
            (IERC20Detailed(address(_cpToken)).totalSupply() * _cpToken.getCurrentExchangeRate()) / 10 ** 18;
    }
}
"#,
        ),
    ] {
        let config = Arc::new(FormatterConfig { line_length, ..Default::default() });
        assert_eq!(format(source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn line_end_operators_do_not_change_layout() {
    for (source, expected) in [
        (
            r#"contract C {
    function f() internal view returns (uint256) {
        return
            super._postOpGasBudget(userOp) +
            Math.ternary(_fetchGuarantor(userOp) == address(0), 0, _guaranteedPostOpCost());
    }
}
"#,
            r#"contract C {
    function f() internal view returns (uint256) {
        return super._postOpGasBudget(userOp)
            + Math.ternary(_fetchGuarantor(userOp) == address(0), 0, _guaranteedPostOpCost());
    }
}
"#,
        ),
        (
            r#"contract C {
    function f() internal view returns (uint256) {
        return c
            ? super._postOpGasBudget(userOp) +
                Math.ternary(_fetchGuarantor(userOp) == address(0), 0, _postOpBudget())
            : 0;
    }
}
"#,
            r#"contract C {
    function f() internal view returns (uint256) {
        return c
            ? super._postOpGasBudget(userOp) + Math.ternary(_fetchGuarantor(userOp) == address(0), 0, _postOpBudget())
            : 0;
    }
}
"#,
        ),
    ] {
        let config = Arc::new(FormatterConfig::default());
        assert_eq!(format(source, Path::new("test.sol"), config.clone()), expected);
        assert_eq!(format(expected, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn for_initializer_leading_comment_is_idempotent() {
    let source = r#"contract C {
    function f() external {
        for (
            /* lead
            detail */ uint i = 0; i < 1; ++i
        ) {}
    }
}
"#;
    let expected = r#"contract C {
    function f() external {
        for (
            /* lead
            detail */
            uint256 i = 0;
            i < 1;
            ++i
        ) {}
    }
}
"#;

    assert_eq!(
        format(source, Path::new("test.sol"), Arc::new(FormatterConfig::default())),
        expected
    );
}

#[test]
fn for_initializer_comment_run_is_idempotent() {
    let source = r#"contract C {
    function f() external {
        for (uint i = 0 /* detail
        more */; // after init
        i < 1; ++i) {}
    }
}
"#;
    let expected = r#"contract C {
    function f() external {
        for (
            uint256 i = 0;

            /* detail
            more */
            // after init
            i < 1;
            ++i
        ) {}
    }
}
"#;

    assert_eq!(
        format(source, Path::new("test.sol"), Arc::new(FormatterConfig::default())),
        expected
    );
}

#[test]
fn for_keyword_comment_is_idempotent() {
    let source = r#"contract C {
    function f() external {
        for // comment
        (; ; ++i) {}
    }
}
"#;
    let expected = r#"contract C {
    function f() external {
        for // comment
            (;; ++i) {}
    }
}
"#;

    assert_eq!(
        format(source, Path::new("test.sol"), Arc::new(FormatterConfig::default())),
        expected
    );
}

#[test]
fn block_opening_comment_run_is_idempotent() {
    let source = r#"contract C {
    function f() external {
        if (a) {} else { // First.
            // Second.
            f();
        }
    }
}
"#;
    let expected = r#"contract C {
    function f() external {
        if (a) {} else {
            // First.
            // Second.
            f();
        }
    }
}
"#;
    for wrap_comments in [false, true] {
        let config = Arc::new(FormatterConfig { wrap_comments, ..Default::default() });
        assert_eq!(format(source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn chained_named_call_layout_ignores_source_spacing() {
    let path = Path::new("test.sol");

    for (line_length, bracket_spacing, compact, spaced) in [
        (
            40,
            false,
            "factory().foo(a,b,c).baz({value: result});",
            "factory().foo(a, b, c).baz({value: result});",
        ),
        (
            32,
            false,
            "factory().foo(a+b).baz({value: result});",
            "factory().foo(a + b).baz({value: result});",
        ),
        (
            34,
            false,
            "factory().foo([a,b]).baz({value: result});",
            "factory().foo([a, b]).baz({value: result});",
        ),
        (38, true, "factory().foo(a,b,c).baz({});", "factory().foo(a, b, c).baz({ });"),
    ] {
        let config =
            Arc::new(FormatterConfig { line_length, bracket_spacing, ..Default::default() });
        let source = |expr| format!("contract C {{ function f() external {{ {expr} }} }}");
        assert_eq!(
            format(&source(compact), path, config.clone()),
            format(&source(spaced), path, config),
        );
    }
}

#[test]
fn statement_trailing_blank_line_is_idempotent() {
    let source = r#"contract C {
    function f() external {
        uint value;
        value += 1
        /* detail. */

        ;
    }
}
"#;
    let expected = r#"contract C {
    function f() external {
        uint256 value;
        value += 1;
        /* detail. */
    }
}
"#;

    assert_eq!(
        format(source, Path::new("test.sol"), Arc::new(FormatterConfig::default())),
        expected
    );
}

#[test]
fn source_offsets_with_unicode_and_crlf() {
    let source = r#"// 😀 文
contract C {
    /// First line.
    /// Second line.
    function f() external {
        // forgefmt: disable-next-line
        uint  x =  1; // Comment.
    }
}
"#
    .replace('\n', "\r\n");
    let path = Path::new("offset.sol");
    let mut compiler = Compiler::new(
        solar::interface::Session::builder().with_buffer_emitter(Default::default()).build(),
    );
    forge_fmt::format_source(
        "// Different file with Unicode: 😀\ncontract Prefix {}",
        Some(Path::new("prefix.sol")),
        Arc::new(FormatterConfig::default()),
        &mut compiler,
    )
    .into_result()
    .unwrap();

    for config in [
        FormatterConfig::default(),
        FormatterConfig {
            wrap_comments: true,
            docs_style: DocCommentStyle::Block,
            ..Default::default()
        },
    ] {
        let config = Arc::new(config);
        let expected = format(&source, path, config.clone());
        let actual = forge_fmt::format_source(&source, Some(path), config, &mut compiler)
            .into_result()
            .unwrap();
        assert_eq!(actual, expected);
    }
}

// <https://github.com/foundry-rs/foundry/issues/3831>
#[test]
fn disable_line_uses_comment_context() {
    let source = r#"contract  C {
    function f() public {
        // forgefmt: disable-line
        assembly { sstore(   0, 0)
            sstore(1,    1)
        }

        assembly { sstore(   2, 2) } // forgefmt: disable-line
    }
}
"#;
    let expected = r#"contract C {
    function f() public {
        // forgefmt: disable-line
        assembly { sstore(   0, 0)
            sstore(1,    1)
        }
        assembly { sstore(   2, 2) } // forgefmt: disable-line
    }
}
"#;

    assert_eq!(
        format(source, Path::new("test.sol"), Arc::new(FormatterConfig::default())),
        expected
    );
}

#[test]
fn narrow_multiline_comment_is_idempotent() {
    let source = r#"contract C {
    function f() external {
        for (uint i = 0; i < 10; ++i) /* detail.
        more text. */ {}
    }
}
"#;
    let expected = "contract C {\n    function f() external {\n        for (uint256 i = 0; i < 10; ++i) \n        /* detail.\n        more text. */\n        {}\n    }\n}\n";
    let config = Arc::new(FormatterConfig { line_length: 40, ..Default::default() });

    let first = format(source, Path::new("test.sol"), config.clone());
    assert_eq!(first, expected);
    assert_eq!(format(&first, Path::new("test.sol"), config), first);
}

#[test]
fn wrapped_mixed_comment_at_line_start_is_idempotent() {
    let source = r#"contract C {
    function f() external {
        /* detail.
        more text. */ uint value;
    }
}
"#;
    let expected = r#"contract C {
    function f() external {
        /* detail.
        more text. */
        uint256 value;
    }
}
"#;
    let config = Arc::new(FormatterConfig { wrap_comments: true, ..Default::default() });

    let first = format(source, Path::new("test.sol"), config.clone());
    assert_eq!(first, expected);
    assert_eq!(format(&first, Path::new("test.sol"), config), first);
}

#[test]
fn trailing_line_comment_separates_following_comment() {
    let source = r#"contract C {
    function f() external {
        uint value;
        value // Trailing line.
        ; /* Following block.
        more text. */
    }
}
"#;
    let expected = r#"contract C {
    function f() external {
        uint256 value;
        value; // Trailing line.
        /* Following block.
        more text. */
    }
}
"#;

    assert_eq!(
        format(source, Path::new("test.sol"), Arc::new(FormatterConfig::default())),
        expected
    );
}

#[test]
fn tab_style_preserves_crlf_disabled_block_lines() {
    let source = "contract C {\n// forgefmt: disable-start\nfunction  disabled( ) external { uint   value = 1; }\n// forgefmt: disable-end\nuint value;\n}\n"
        .replace('\n', "\r\n");
    let expected = "contract C {\n\t// forgefmt: disable-start\nfunction  disabled( ) external { uint   value = 1; }\n// forgefmt: disable-end\n\tuint256 value;\n}\n";
    let config = Arc::new(FormatterConfig { style: IndentStyle::Tab, ..Default::default() });

    assert_eq!(format(&source, Path::new("test.sol"), config), expected);
}

#[test]
fn array_type_comment_before_bracket_is_idempotent() {
    let source = r#"contract C {
    function f() external {
        uint256 /* first */ [
            /* second */

            3
        ] memory values;
    }
}
"#;
    let expected = r#"contract C {
    function f() external {
        uint256 /* first */ [
            /* second */

            3] memory values;
    }
}
"#;

    assert_eq!(
        format(source, Path::new("test.sol"), Arc::new(FormatterConfig::default())),
        expected
    );
}

#[test]
fn yul_assignment_comment_is_idempotent() {
    let source = r#"contract C {
    function f() external pure returns (uint256 x) {
        assembly {
            x := /* comment */ add(1, 2)
        }
    }
}
"#;
    let expected = r#"contract C {
    function f() external pure returns (uint256 x) {
        assembly {
            x := /* comment */
            add(1, 2)
        }
    }
}
"#;

    assert_eq!(
        format(source, Path::new("test.sol"), Arc::new(FormatterConfig::default())),
        expected
    );
}

#[test]
fn return_expression_comment_is_idempotent() {
    let source = r#"contract C {
    function f() external pure returns (uint256, uint256, bool) {
        return /* return values */ (1234567890, 9876543210, false);
    }
}
"#;
    let expected = r#"contract C {
    function f()
        external
        pure
        returns (uint256, uint256, bool)
    {
        return /* return values */
            (1234567890, 9876543210, false);
    }
}
"#;
    let config =
        Arc::new(FormatterConfig { line_length: 60, wrap_comments: true, ..Default::default() });

    assert_eq!(format(source, Path::new("test.sol"), config), expected);
}

#[test]
fn unmatched_disable_end_preserves_comments_and_nested_blocks() {
    let source = r#"// forgefmt: disable-end
// forgefmt: disable-end
/**@notice Before disabled blocks.*/
contract  Before {}
// forgefmt: disable-start
contract  OuterDisabled {}
// forgefmt: disable-start
contract  InnerDisabled {}
// forgefmt: disable-end
contract  StillDisabled {}
// forgefmt: disable-end
// forgefmt: disable-end
/* Another comment. */
/**
 * @notice After disabled blocks.
 */
contract  After {}
"#;

    for wrap_comments in [false, true] {
        let config = Arc::new(FormatterConfig { wrap_comments, ..Default::default() });
        assert_data_eq!(
            format(source, Path::new("test.sol"), config),
            str![[r#"
// forgefmt: disable-end
// forgefmt: disable-end
/**
 * @notice Before disabled blocks.
 */
contract Before {}

// forgefmt: disable-start
contract  OuterDisabled {}
// forgefmt: disable-start
contract  InnerDisabled {}
// forgefmt: disable-end
contract  StillDisabled {}
// forgefmt: disable-end
// forgefmt: disable-end
/* Another comment. */
/**
 * @notice After disabled blocks.
 */
contract After {}

"#]]
        );
    }
}

#[test]
fn disable_directive_whitespace_preserves_doc_comments() {
    for start_space in ["", " ", "  ", "\t"] {
        for end_space in ["", " ", "  ", "\t"] {
            let source = format!(
                "// forgefmt:{start_space}disable-start\ncontract  A {{}}\n// forgefmt:{end_space}disable-end\n/**@notice format me*/\ncontract  B {{}}\n"
            );
            let expected = format!(
                "// forgefmt:{start_space}disable-start\ncontract  A {{}}\n// forgefmt:{end_space}disable-end\n/**\n * @notice format me\n */\ncontract B {{}}\n"
            );

            for wrap_comments in [false, true] {
                let config = Arc::new(FormatterConfig { wrap_comments, ..Default::default() });
                assert_data_eq!(format(&source, Path::new("test.sol"), config), expected.clone());
            }
        }
    }
}

#[test]
fn block_disable_directives_preserve_doc_comments() {
    for disabled in [
        "// forgefmt: disable-start\ncontract  A {}\n/**forgefmt:disable-end*/\n",
        "/**forgefmt:disable-start*/\ncontract  A {}\n/**forgefmt:disable-end*/\n",
        "/*forgefmt:disable-start*/\ncontract  A {}\n/*forgefmt:disable-end*/\n",
        "/**forgefmt:disable-start\nforgefmt:disable-end*/\n",
        "/*forgefmt:disable-start\nforgefmt:disable-end*/\n",
    ] {
        let source = format!("{disabled}/**@notice format me*/\ncontract  B {{}}\n");
        let expected = format!("{disabled}/**\n * @notice format me\n */\ncontract B {{}}\n");
        for wrap_comments in [false, true] {
            let config = Arc::new(FormatterConfig { wrap_comments, ..Default::default() });
            assert_data_eq!(format(&source, Path::new("test.sol"), config), expected.clone());
        }
    }
}

#[test]
fn disable_directives_between_comments_preserve_doc_comments() {
    let disabled = "// before\n// forgefmt:disable-start\n// inside\ncontract  A {}\n// inside\n// forgefmt:disable-end\n// after\n";
    let source = format!("{disabled}/**@notice format me*/\ncontract  B {{}}\n");
    let expected = format!("{disabled}/**\n * @notice format me\n */\ncontract B {{}}\n");
    for wrap_comments in [false, true] {
        let config = Arc::new(FormatterConfig { wrap_comments, ..Default::default() });
        assert_data_eq!(format(&source, Path::new("test.sol"), config), expected.clone());
    }
}

#[test]
fn directive_like_text_preserves_doc_comments() {
    for prefix in [
        "// mention forgefmt: disable-start\n",
        "// forgefmt: disable-start-invalid\n",
        "/* mention forgefmt: disable-start */\n",
        "contract A {\n    string constant TEXT = \"forgefmt: disable-start\";\n}\n\n",
    ] {
        let source = format!("{prefix}/**@notice format me*/\ncontract  B {{}}\n");
        let expected = format!("{prefix}/**\n * @notice format me\n */\ncontract B {{}}\n");
        for wrap_comments in [false, true] {
            let config = Arc::new(FormatterConfig { wrap_comments, ..Default::default() });
            assert_data_eq!(format(&source, Path::new("test.sol"), config), expected.clone());
        }
    }
}

#[test]
fn doc_style_conversion_preserves_disable_directives() {
    let source = "/// @notice Before\n/// forgefmt: disable-start\ncontract  A {}\n/// forgefmt: disable-end\n/// @notice After\ncontract  B {}\n";
    let expected = "/// @notice Before\n/// forgefmt: disable-start\ncontract  A {}\n/// forgefmt: disable-end\n/// @notice After\ncontract B {}\n";
    for wrap_comments in [true, false] {
        let config = Arc::new(FormatterConfig {
            wrap_comments,
            docs_style: DocCommentStyle::Block,
            ..Default::default()
        });
        assert_data_eq!(format(source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn doc_style_conversion_preserves_directive_doc_groups() {
    let source = "/// @notice Before\n/// forgefmt:disable-end\n/// @dev After\ncontract  A {}\n";
    let expected = "/// @notice Before\n/// forgefmt:disable-end\n/// @dev After\ncontract A {}\n";
    for wrap_comments in [false, true] {
        let config = Arc::new(FormatterConfig {
            wrap_comments,
            docs_style: DocCommentStyle::Block,
            ..Default::default()
        });
        assert_data_eq!(format(source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn doc_style_conversion_preserves_single_item_directives() {
    for directive in ["disable-next-item", "disable-line"] {
        let source = format!(
            "/// @notice Before\n/// forgefmt:                     {directive}\n/// @dev After\ncontract  A {{}}\n\n/// @notice Unrelated\ncontract  B {{}}\n"
        );
        for docs_style in [DocCommentStyle::Preserve, DocCommentStyle::Line, DocCommentStyle::Block]
        {
            let unrelated = if docs_style == DocCommentStyle::Block {
                "/**\n * @notice Unrelated\n */"
            } else {
                "/// @notice Unrelated"
            };
            let expected = format!(
                "/// @notice Before\n/// forgefmt:                     {directive}\n/// @dev After\ncontract  A {{}}\n\n{unrelated}\ncontract B {{}}\n"
            );
            for wrap_comments in [false, true] {
                let config = Arc::new(FormatterConfig {
                    wrap_comments,
                    docs_style,
                    line_length: 40,
                    ..Default::default()
                });
                assert_data_eq!(format(&source, Path::new("test.sol"), config), expected.clone());
            }
        }
    }
}

#[test]
fn doc_style_conversion_preserves_next_line_directive_target() {
    let source =
        "/// @notice Before\n/// forgefmt: disable-next-line\n/// @dev After\ncontract  A {}\n";
    let expected =
        "/// @notice Before\n/// forgefmt: disable-next-line\n/// @dev After\ncontract A {}\n";
    for wrap_comments in [false, true] {
        let config = Arc::new(FormatterConfig {
            wrap_comments,
            docs_style: DocCommentStyle::Block,
            line_length: 20,
            ..Default::default()
        });
        assert_data_eq!(format(source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn doc_style_conversion_preserves_trailing_directive_doc_groups() {
    let source =
        "contract A {} /// forgefmt: disable-next-item\n/// @notice Before\ncontract  B {}\n";
    for wrap_comments in [false, true] {
        let config = Arc::new(FormatterConfig {
            wrap_comments,
            docs_style: DocCommentStyle::Block,
            ..Default::default()
        });
        assert_data_eq!(format(source, Path::new("test.sol"), config), source);
    }
}

#[test]
fn directive_doc_groups_are_separate_from_regular_comments() {
    let source = "// A long regular comment with enough words to exceed the configured width.\n/// @notice Before\n/// forgefmt:disable-end\n/// @dev After\ncontract  A {}\n";
    for wrap_comments in [false, true] {
        let regular = if wrap_comments {
            "// A long regular comment with enough\n// words to exceed the configured width."
        } else {
            "// A long regular comment with enough words to exceed the configured width."
        };
        let expected = format!(
            "{regular}\n/// @notice Before\n/// forgefmt:disable-end\n/// @dev After\ncontract A {{}}\n"
        );
        let config = Arc::new(FormatterConfig {
            wrap_comments,
            docs_style: DocCommentStyle::Block,
            line_length: 40,
            ..Default::default()
        });
        assert_data_eq!(format(source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn doc_style_conversion_keeps_adjacent_regular_comments() {
    let source = "contract A {\n    /// @notice a\n    // regular note\n    function f() external {}\n\n    // regular note\n    /// @notice b\n    function g() external {}\n\n    /// @notice c\n    //\n    function h() external {}\n}\n";
    let expected = "contract A {\n    /**\n     * @notice a\n     */\n    // regular note\n    function f() external {}\n\n    // regular note\n    /**\n     * @notice b\n     */\n    function g() external {}\n\n    /**\n     * @notice c\n     */\n    //\n    function h() external {}\n}\n";
    for wrap_comments in [false, true] {
        let config = Arc::new(FormatterConfig {
            wrap_comments,
            docs_style: DocCommentStyle::Block,
            ..Default::default()
        });
        assert_data_eq!(format(source, Path::new("test.sol"), config), expected);
    }
}

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn test_directory(base_name: &str) {
    init_tracing();
    let dir = tests_dir().join(base_name);
    let mut original = fs::read_to_string(dir.join("original.sol")).unwrap();
    if cfg!(windows) {
        original = original.replace("\r\n", "\n");
    }
    let mut handles = vec![];
    for res in dir.read_dir().unwrap() {
        let entry = res.unwrap();
        let path = entry.path();

        let filename = path.file_name().and_then(|name| name.to_str()).unwrap();
        if filename == "original.sol" {
            continue;
        }
        assert!(path.is_file(), "expected file: {path:?}");
        assert!(filename.ends_with("fmt.sol"), "unknown file: {path:?}");

        let mut expected = fs::read_to_string(&path).unwrap();
        if cfg!(windows) {
            expected = expected
                .replace("\r\n", "\n")
                .replace(r"\'", r"/'")
                .replace(r#"\""#, r#"/""#)
                .replace("\\\n", "/\n");
        }

        // The majority of the tests were written with the assumption that the default value for max
        // line length is `80`. Preserve that to avoid rewriting test logic.
        let default_config = FormatterConfig { line_length: 80, ..Default::default() };

        let mut config = toml::Value::try_from(default_config).unwrap();
        let config_table = config.as_table_mut().unwrap();
        let mut comments_end = 0;
        for (i, line) in expected.lines().enumerate() {
            let line_num = i + 1;
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

            comments_end += line.len() + 1;
        }
        let config = Arc::new(
            config
                .try_into::<FormatterConfig>()
                .unwrap_or_else(|err| panic!("invalid test config for {filename}: {err}")),
        );

        let original = original.clone();
        let tname = format!("{base_name}/{filename}");
        let spawn = move || {
            test_formatter(&path, config.clone(), &original, &expected, comments_end);
        };
        handles.push(std::thread::Builder::new().name(tname).spawn(spawn).unwrap());
    }
    let results = handles.into_iter().map(|h| h.join()).collect::<Vec<_>>();
    for result in results {
        result.unwrap();
    }
}

fn test_formatter(
    expected_path: &Path,
    config: Arc<FormatterConfig>,
    source: &str,
    expected_source: &str,
    comments_end: usize,
) {
    let path = &*expected_path.with_file_name("original.sol");
    let expected_data = || Data::read_from(expected_path, None).raw();

    let mut source_formatted = format(source, path, config.clone());
    // Inject `expected`'s comments, if any, so we can use the expected file as a snapshot.
    source_formatted.insert_str(0, &expected_source[..comments_end]);
    assert_data_eq!(&source_formatted, expected_data());
    assert_eof(&source_formatted);

    let mut expected_content = std::fs::read_to_string(expected_path).unwrap();
    if cfg!(windows) {
        expected_content = expected_content.replace("\r\n", "\n");
    }
    let expected_formatted = format(&expected_content, expected_path, config);
    assert_data_eq!(&expected_formatted, expected_data());
    assert_eof(expected_source);
    assert_eof(&expected_formatted);
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
    assert!(
        undeclared.is_empty(),
        "the following test directories are not declared in the test suite macro call: {undeclared:#?}"
    )
}

macro_rules! fmt_tests {
    ($($(#[$attr:meta])* $dir:ident),+ $(,)?) => {
        #[test]
        fn all_dirs_are_declared() {
            test_all_dirs_are_declared(&[$(stringify!($dir)),*]);
        }

        $(
            #[allow(non_snake_case)]
            #[test]
            $(#[$attr])*
            fn $dir() {
                test_directory(stringify!($dir));
            }
        )+
    };
}

fmt_tests! {
    #[ignore = "annotations are not valid Solidity"]
    Annotation,
    ArrayExpressions,
    AssignmentMemberChain,
    BlockComments,
    BlockCommentsFunction,
    CallOptionsAssign,
    CommentEmptyLine,
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
    FnAttributeComment,
    ForStatement,
    FunctionCall,
    FunctionCallArgsStatement,
    FunctionDefinition,
    FunctionDefinitionWithFunctionReturns,
    FunctionType,
    HexUnderscore,
    IfStatement,
    IfStatement2,
    IfStatement3,
    IfStatementLongCondition,
    IfStatementMultilineCall,
    ImportDirective,
    IndexedAssignment,
    InlineDisable,
    IntTypes,
    LineComments,
    LiteralExpression,
    MappingNamedParams,
    MappingType,
    MemberChainIndent,
    MethodChain,
    MethodChainCallOptions,
    MixedBlockComments,
    ModifierDefinition,
    NamedCallArgsInChain,
    NestedNamedCallArgumentChain,
    NamedFunctionCallExpression,
    NonKeywords,
    NumberLiteralUnderscore,
    OperatorExpressions,
    PragmaDirective,
    Repros,
    ReprosCalls,
    ReprosFunctionDefs,
    ReturnStatement,
    RevertNamedArgsStatement,
    RevertStatement,
    SimpleComments,
    SortedImports,
    StatementBlock,
    StatementBlockBodyComments,
    StatementBlockModes,
    StatementBlockSafety,
    StructDefinition,
    StructFieldAccess,
    ThisExpression,
    #[ignore = "Solar errors when parsing inputs with trailing commas"]
    TrailingComma,
    TryStatement,
    TypeDefinition,
    UnitExpression,
    UsingDirective,
    VariableAssignment,
    VariableDefinition,
    WhileStatement,
    Yul,
    YulInlineBlock,
    YulStrings,
}

#[test]
fn test_override_state_variable_without_initializer_does_not_leak_indent() {
    init_tracing();

    let cases = [
        (
            "top-level items after override variable",
            r#"pragma solidity ^0.8.28;

contract BaseStorage {
  uint256 public total;
}

contract ChildStorage is BaseStorage {
  uint256 public override total;
}

struct Info {
  uint256 a;
}

function topLevel(uint256 value) pure returns (uint256) {
  return value;
}
"#,
            r#"pragma solidity ^0.8.28;

contract BaseStorage {
    uint256 public total;
}

contract ChildStorage is BaseStorage {
    uint256 public override total;
}

struct Info {
    uint256 a;
}

function topLevel(uint256 value) pure returns (uint256) {
    return value;
}
"#,
        ),
        (
            "contract member after override variable",
            r#"pragma solidity ^0.8.28;

contract BaseStorage {
  uint256 public total;
}

contract ChildStorage is BaseStorage {
  uint256 public override total;
  uint256 public next;
}
"#,
            r#"pragma solidity ^0.8.28;

contract BaseStorage {
    uint256 public total;
}

contract ChildStorage is BaseStorage {
    uint256 public override total;
    uint256 public next;
}
"#,
        ),
        (
            "override path list without initializer",
            r#"pragma solidity ^0.8.28;

contract BaseA {
  uint256 public total;
}

contract BaseB {
  uint256 public total;
}

contract ChildStorage is BaseA, BaseB {
  uint256 public override(BaseA, BaseB) total;
}

error AfterOverride(uint256 value);
"#,
            r#"pragma solidity ^0.8.28;

contract BaseA {
    uint256 public total;
}

contract BaseB {
    uint256 public total;
}

contract ChildStorage is BaseA, BaseB {
    uint256 public override(BaseA, BaseB) total;
}

error AfterOverride(uint256 value);
"#,
        ),
        (
            "override variable with initializer",
            r#"pragma solidity ^0.8.28;

contract BaseStorage {
  uint256 public total;
}

contract ChildStorage is BaseStorage {
  uint256 public override total = 0;
}

struct AfterInitializer {
  uint256 a;
}
"#,
            r#"pragma solidity ^0.8.28;

contract BaseStorage {
    uint256 public total;
}

contract ChildStorage is BaseStorage {
    uint256 public override total = 0;
}

struct AfterInitializer {
    uint256 a;
}
"#,
        ),
    ];

    let fmt_config = Arc::new(FormatterConfig::default());
    let path = Path::new("override-indent.sol");

    for (case, source, expected) in cases {
        let formatted = format(source, path, fmt_config.clone());
        assert_eq!(formatted, expected, "{case}");
    }
}

#[test]
fn size_estimate_ignores_literal_contents() {
    for (line_length, bracket_spacing, literal, control) in [
        (55, true, "{a}{b}{c}{d}", "abcdefghijkl"),
        (59, false, "uint uint uint uint", "word word word word"),
    ] {
        let config =
            Arc::new(FormatterConfig { line_length, bracket_spacing, ..Default::default() });
        let source = format!(
            "contract C {{ function f(uint a) external pure returns (bytes memory) {{ bytes memory encoded = abi.encode(\"{literal}\", a, a, a); return encoded; }} }}\n"
        );
        let control_source = source.replace(literal, control);
        let expected = format(&control_source, Path::new("test.sol"), config.clone())
            .replace(control, literal);
        assert_eq!(format(&source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn brace_spacing_size_estimate_handles_tabs() {
    let config =
        Arc::new(FormatterConfig { line_length: 120, bracket_spacing: true, ..Default::default() });
    let source = r#"contract C {
    function f() external {
        executions = factory({	a: assetAddress, b: receiver, c: amountToSend, d: currentNonce, e: expiryTime, f: requiredFee	});
    }
}
"#;
    let expected = r#"contract C {
    function f() external {
        executions =
            factory({ a: assetAddress, b: receiver, c: amountToSend, d: currentNonce, e: expiryTime, f: requiredFee });
    }
}
"#;

    assert_eq!(format(source, Path::new("test.sol"), config), expected);
}

#[test]
fn concatenated_string_trailing_comment_stays_after_last_literal() {
    let source = r#"contract C {
    function f() public pure returns (bytes memory) {
        return bytes.concat(
            "abc"
            "123456789012345678901234567890123456789012345678901234567890" // Longer than 32 bytes
        );
    }
}
"#;
    let expected = r#"contract C {
    function f() public pure returns (bytes memory) {
        return
            bytes.concat(
                "abc" "123456789012345678901234567890123456789012345678901234567890" // Longer than 32 bytes
            );
    }
}
"#;

    assert_eq!(
        format(source, Path::new("concatenated-string.sol"), Arc::new(FormatterConfig::default())),
        expected
    );
}

#[test]
fn for_body_normalization_is_stable() {
    let source =
        "contract C { function f() external { uint256 n; for(uint i=0;i<n;i++) n += uint(n); } }";
    for style in [SingleLineBlockStyle::Preserve, SingleLineBlockStyle::Single] {
        for line_length in [56, 57] {
            let body = if line_length == 56 {
                "{\n            n += uint256(n);\n        }"
            } else {
                "n += uint256(n);"
            };
            let expected = format!(
                "contract C {{\n    function f() external {{\n        uint256 n;\n        for (uint256 i = 0; i < n; i++) {body}\n    }}\n}}\n"
            );
            let config = Arc::new(FormatterConfig {
                line_length,
                single_line_statement_blocks: style,
                ..Default::default()
            });
            assert_eq!(format(source, Path::new("test.sol"), config), expected);
        }
    }
}

#[test]
fn for_header_normalization_is_stable() {
    for header in
        ["for (uint i = 0; i < n; i++)", "for(uint i=0;i<n;i++)", "for (uint256 i = 0; i < n; i++)"]
    {
        for style in [SingleLineBlockStyle::Preserve, SingleLineBlockStyle::Single] {
            for line_length in [44, 45] {
                let source = format!(
                    "contract C {{ function f() external {{ uint256 n; {header} n++; }} }}"
                );
                let body =
                    if line_length == 44 { "{\n            n++;\n        }" } else { "n++;" };
                let expected = format!(
                    "contract C {{\n    function f() external {{\n        uint256 n;\n        for (uint256 i = 0; i < n; i++) {body}\n    }}\n}}\n"
                );
                let config = Arc::new(FormatterConfig {
                    line_length,
                    single_line_statement_blocks: style,
                    ..Default::default()
                });
                assert_eq!(format(&source, Path::new("test.sol"), config), expected);
            }
        }
    }
}

#[test]
fn existing_condition_layouts_are_preserved() {
    for source in [
        r#"contract C {
    function f() external {
        if (_sel == this.superchainConfigProxy.selector) _superchainConfigProxy = _addr;
        else if (_sel == this.delayedWethPermissionlessGameProxy.selector) _delayedWethPermissionlessGameProxy = _addr;
        else revert("unknown address selector");
    }
}
"#,
        r#"contract C {
    function f() external {
        for (uint256 i; i < logs.length; ++i) {
            Vm.Log memory entry = logs[i];
            if (entry.emitter != address(hook) || entry.topics.length == 0 || entry.topics[0] != NATIVE_FEE_PAID_TOPIC) continue;
            ++count;
        }
    }
}
"#,
    ] {
        let config = Arc::new(FormatterConfig { line_length: 120, ..Default::default() });
        assert_eq!(format(source, Path::new("test.sol"), config), source);
    }
}

/// Records statement nesting while ignoring braces around a single non-declaration statement.
fn statement_structure(source: &str) -> Vec<String> {
    let mut compiler = Compiler::new(
        solar::interface::Session::builder().with_buffer_emitter(Default::default()).build(),
    );
    compiler.enter_mut(|c| {
        let file = c
            .sess()
            .source_map()
            .new_source_file(solar::interface::source_map::FileName::Stdin, source)
            .unwrap();
        let mut parser = c.parse();
        parser.set_resolve_imports(false);
        parser.add_file(file.clone());
        parser.parse();
        c.dcx().has_errors().unwrap();
        let gcx = c.gcx();
        let (_, source) = gcx.get_ast_source(&file.name).unwrap();
        let mut visitor = StatementStructure::default();
        let _ = visitor.visit_source_unit(source.ast.as_ref().unwrap());
        visitor.0
    })
}

#[derive(Default)]
struct StatementStructure(Vec<String>);

impl<'ast> Visit<'ast> for StatementStructure {
    type BreakValue = ();

    fn visit_stmt(&mut self, stmt: &'ast ast::Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        if let ast::StmtKind::Block(block) = &stmt.kind
            && let [child] = block.stmts.as_slice()
            && !matches!(child.kind, ast::StmtKind::DeclSingle(_) | ast::StmtKind::DeclMulti(..))
        {
            return self.visit_stmt(child);
        }
        self.0.push(format!("{:?}", std::mem::discriminant(&stmt.kind)));
        self.walk_stmt(stmt)?;
        self.0.push(")".into());
        ControlFlow::Continue(())
    }
}

#[test]
fn inlined_loops_preserve_else_binding() {
    for then in [
        "for (;;) { if (b) x(); }",
        "{ for (;;) { if (b) x(); } }",
        "for (;;) while (b) { if (c) x(); }",
        "while (b) for (;;) { if (c) x(); }",
        "for (;;) { if (b) x(); else y(); }",
        "for (;;) { if (b) { if (c) x(); } else y(); }",
        "for (;;) { uint256 x; }",
        "for (;;) { (uint256 x, uint256 y) = g(); }",
    ] {
        let source =
            format!("contract C {{ function f() external {{ if (a) {then} else z(); }} }}");
        let expected = statement_structure(&source);
        for style in [SingleLineBlockStyle::Preserve, SingleLineBlockStyle::Single] {
            let config = Arc::new(FormatterConfig {
                line_length: 120,
                single_line_statement_blocks: style,
                ..Default::default()
            });
            let actual = format(&source, Path::new("test.sol"), config);
            assert_eq!(expected, statement_structure(&actual), "{style:?}\n{source}\n{actual}");
        }
    }
}

#[test]
fn for_header_at_line_limit_keeps_nested_body_stable() {
    let source = "contract C { function f() external { for (uint i=0;i<n;++i) { while (b) { if (c) x(); } } y(); } }";
    let expected = "contract C {\n    function f() external {\n        for (uint256 i = 0; i < n; ++i) {\n            while (b) if (c) x();\n        }\n        y();\n    }\n}\n";
    for style in [SingleLineBlockStyle::Preserve, SingleLineBlockStyle::Single] {
        let config = Arc::new(FormatterConfig {
            line_length: 40,
            single_line_statement_blocks: style,
            ..Default::default()
        });
        assert_eq!(format(source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn wrapped_body_trailing_comments_keep_braces() {
    for style in [SingleLineBlockStyle::Preserve, SingleLineBlockStyle::Single] {
        let expected = "contract C {\n    function f() external {\n        for (;;) {\n            break; /* aaaaaaaaa bbbbbbbbb */\n        }\n    }\n}\n";
        let config = Arc::new(FormatterConfig {
            line_length: 40,
            wrap_comments: true,
            single_line_statement_blocks: style,
            ..Default::default()
        });
        for spacing in ["", " "] {
            let source = format!(
                "contract C {{ function f() external {{ for (;;) {{ break;{spacing}/* aaaaaaaaa bbbbbbbbb */ }} }} }}"
            );
            assert_eq!(format(&source, Path::new("test.sol"), config.clone()), expected);
        }
    }
}

#[test]
fn for_body_assignment_spacing_is_stable() {
    for style in [SingleLineBlockStyle::Preserve, SingleLineBlockStyle::Single] {
        for line_length in [36, 37] {
            let body = if line_length == 36 {
                "for (;;) {\n            variable = g() + b;\n        }"
            } else {
                "for (;;) variable = g() + b;"
            };
            let expected = format!(
                "contract C {{\n    function f(uint256 b) external {{\n        uint256 variable;\n        {body}\n    }}\n}}\n"
            );
            let config = Arc::new(FormatterConfig {
                line_length,
                single_line_statement_blocks: style,
                ..Default::default()
            });
            for spacing in ["", "                    "] {
                let source = format!(
                    "contract C {{ function f(uint256 b) external {{ uint256 variable; for (;;) variable = g({spacing}) + b; }} }}"
                );
                assert_eq!(format(&source, Path::new("test.sol"), config.clone()), expected);
            }
        }
    }

    for style in [SingleLineBlockStyle::Preserve, SingleLineBlockStyle::Single] {
        let config = Arc::new(FormatterConfig {
            line_length: 40,
            single_line_statement_blocks: style,
            ..Default::default()
        });
        for nested in [false, true] {
            let body = if nested { "g(variable = h() + b);" } else { "variable = g() + b;" };
            let expected = format!(
                "contract C {{\n    function f(uint256 b) external {{\n        uint256 variable;\n        for (;;) {body}\n    }}\n}}\n"
            );
            for padding in [0, 20, 65_516] {
                let spacing = " ".repeat(padding);
                let body = if nested {
                    format!("g(variable = h({spacing}) + b);")
                } else {
                    format!("variable = g({spacing}) + b;")
                };
                let source = format!(
                    "contract C {{ function f(uint256 b) external {{ uint256 variable; for (;;) {body} }} }}"
                );
                assert_eq!(format(&source, Path::new("test.sol"), config.clone()), expected);
            }
        }
    }
}

#[test]
fn for_nested_body_tail_comments_keep_braces() {
    for header in ["while (a)", "if (a)"] {
        let source = format!(
            r#"contract C {{
    function f(bool a) external {{
        for (;;) {{
            {header} {{
                break; // tail
            }}
        }}
    }}
}}
"#
        );
        let expected = format!(
            r#"contract C {{
    function f(bool a) external {{
        for (;;) {{
            {header} break; // tail
        }}
    }}
}}
"#
        );
        let config = Arc::new(FormatterConfig {
            single_line_statement_blocks: SingleLineBlockStyle::Single,
            ..Default::default()
        });
        assert_data_eq!(format(&source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn fractional_and_wide_number_literals() {
    let source = "contract C { function f() external { 1.25e-2; 1e100; 1.2e3; } }";
    let expected = "contract C {\n    function f() external {\n        1.25e-2;\n        1e100;\n        1.2e3;\n    }\n}\n";
    assert_eq!(format(source, Path::new("test.sol"), Arc::default()), expected);
}

#[test]
fn wrapped_for_header_keeps_body_braces() {
    let source = "contract C { function f() external { for (uint i = f() + g(); i < n; i += 2) x = g() + b; y(); } }";
    let expected = r#"contract C {
    function f() external {
        for (
            uint256 i = f() + g(); i < n; i += 2) {
            x = g() + b;
        }
        y();
    }
}
"#;
    for style in [SingleLineBlockStyle::Preserve, SingleLineBlockStyle::Single] {
        let config = Arc::new(FormatterConfig {
            line_length: 50,
            single_line_statement_blocks: style,
            ..Default::default()
        });
        assert_eq!(format(source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn body_tail_comments_keep_braces() {
    for header in ["for (;;)", "if (a)", "while (a)"] {
        let source = format!(
            "contract C {{ function f() external {{ {header} {{ x = g() /* tail */; }} y(); }} }}"
        );
        let expected = format!(
            "contract C {{\n    function f() external {{\n        {header} {{\n            x = g(); /* tail */\n        }}\n        y();\n    }}\n}}\n"
        );
        let config = Arc::new(FormatterConfig {
            single_line_statement_blocks: SingleLineBlockStyle::Single,
            ..Default::default()
        });
        assert_eq!(format(&source, Path::new("test.sol"), config), expected);
    }
}

#[test]
fn nested_for_headers_keep_parent_braces() {
    for (prefix, line_length, expected_body) in [
        ("if(a)", 50, "if (a) {\n            for (uint256 i = 0; i < n; ++i) x();\n        }"),
        (
            "while(a)",
            53,
            "while (a) {\n            for (uint256 i = 0; i < n; ++i) x();\n        }",
        ),
        (
            "if(a)while(b)",
            61,
            "if (a) {\n            while (b) {\n                for (uint256 i = 0; i < n; ++i) x();\n            }\n        }",
        ),
    ] {
        let source = format!(
            "contract C {{ function f() external {{ {prefix}for(uint i=0;i<n;++i)x(); y(); }} }}"
        );
        let expected = format!(
            "contract C {{\n    function f() external {{\n        {expected_body}\n        y();\n    }}\n}}\n"
        );
        for style in [SingleLineBlockStyle::Preserve, SingleLineBlockStyle::Single] {
            let config = Arc::new(FormatterConfig {
                line_length,
                single_line_statement_blocks: style,
                ..Default::default()
            });
            assert_eq!(format(&source, Path::new("test.sol"), config), expected);
        }
    }
}

#[test]
fn assignment_rhs_leading_comment_keeps_member_chain_layout() {
    let source = "contract C { function f() external { value = /* lead */ factory.first(\na, b\n).second(); } }";
    let expected = r#"contract C {
    function f() external {
        value = /* lead */
            factory.first(a, b).second();
    }
}
"#;
    let config =
        Arc::new(FormatterConfig { line_length: 46, bracket_spacing: true, ..Default::default() });
    assert_data_eq!(format(source, Path::new("test.sol"), config), expected);
}

#[test]
fn preserve_statement_body_line_breaks() {
    for header in ["for (;;)", "if (a)", "while (a)"] {
        for (spacing, body) in [
            (" ", " x();"),
            ("\n", " {\n            x();\n        }"),
            ("\r\n", " {\n            x();\n        }"),
            ("\n    ", " {\n            x();\n        }"),
            ("\r\n    ", " {\n            x();\n        }"),
        ] {
            let source =
                format!("contract C {{ function f() external {{ {header}{spacing}x(); }} }}");
            let expected = format!(
                "contract C {{\n    function f() external {{\n        {header}{body}\n    }}\n}}\n"
            );
            assert_data_eq!(format(&source, Path::new("test.sol"), Arc::default()), expected);
        }
    }
}
