// const _GRAMMAR: &'static str = include_str!("grammar.pest");

#[derive(Parser)]
#[grammar = "grammar.pest"]
pub struct HandlebarsParser;

#[cfg(test)]
mod test {
    use super::{HandlebarsParser, Rule};
    use pest::Parser;

    macro_rules! assert_rule {
        ($rule:expr, $in:expr) => {
            assert_eq!(
                HandlebarsParser::parse($rule, $in)
                    .unwrap()
                    .last()
                    .unwrap()
                    .as_span()
                    .end(),
                $in.len()
            );
        };
    }

    macro_rules! assert_not_rule {
        ($rule:expr, $in:expr) => {
            assert!(
                HandlebarsParser::parse($rule, $in).is_err()
                    || HandlebarsParser::parse($rule, $in)
                        .unwrap()
                        .last()
                        .unwrap()
                        .as_span()
                        .end()
                        != $in.len()
            );
        };
    }

    macro_rules! assert_rule_match {
        ($rule:expr, $in:expr) => {
            assert!(HandlebarsParser::parse($rule, $in).is_ok());
        };
    }

    #[test]
    fn test_raw_text() {
        let s = [
            "<h1> helloworld </h1>    ",
            r"hello\{{world}}",
            r"hello\{{#if world}}nice\{{/if}}",
            r"hello \{{{{raw}}}}hello\{{{{/raw}}}}",
        ];
        for i in &s {
            assert_rule!(Rule::raw_text, i);
        }

        let s_not_escape = [r"\\{{hello}}"];
        for i in &s_not_escape {
            assert_not_rule!(Rule::raw_text, i);
        }
    }

    #[test]
    fn test_raw_block_text() {
        let s = "<h1> {{hello}} </h1>";
        assert_rule!(Rule::raw_block_text, s);
    }

    #[test]
    fn test_reference() {
        let s = vec![
            "a",
            "abc",
            "../a",
            "a.b",
            "@abc",
            "a.[abc]",
            "aBc.[abc]",
            "abc.[0].[nice]",
            "some-name",
            "this.[0].ok",
            "this.[$id]",
            "[$id]",
            "$id",
            "this.[null]",
        ];
        for i in &s {
            assert_rule!(Rule::reference, i);
        }
    }

    #[test]
    fn test_name() {
        let s = ["if", "(abc)"];
        for i in &s {
            assert_rule!(Rule::name, i);
        }
    }

    #[test]
    fn test_param() {
        let s = ["hello", "\"json literal\"", "nullable", "truestory"];
        for i in &s {
            assert_rule!(Rule::helper_parameter, i);
        }
    }

    #[test]
    fn test_hash() {
        let s = [
            "hello=world",
            "hello=\"world\"",
            "hello=(world)",
            "hello=(world 0)",
        ];
        for i in &s {
            assert_rule!(Rule::hash, i);
        }
    }

    #[test]
    fn test_json_literal() {
        let s = [
            "\"json string\"",
            "\"quot: \\\"\"",
            "[]",
            "[\"hello\"]",
            "[1,2,3,4,true]",
            "{\"hello\": \"world\"}",
            "{}",
            "{\"a\":1, \"b\":2 }",
            "\"nullable\"",
        ];
        for i in &s {
            assert_rule!(Rule::literal, i);
        }
    }

    #[test]
    fn test_comment() {
        let s = ["{{!-- <hello {{ a-b c-d}} {{d-c}} ok --}}",
                 "{{!--
                    <li><a href=\"{{up-dir nest-count}}{{base-url}}index.html\">{{this.title}}</a></li>
                --}}",
                     "{{!    -- good  --}}"];
        for i in &s {
            assert_rule!(Rule::hbs_comment, i);
        }
        let s2 = ["{{! hello }}", "{{! test me }}"];
        for i in &s2 {
            assert_rule!(Rule::hbs_comment_compact, i);
        }
    }

    #[test]
    fn test_subexpression() {
        let s = ["(sub)", "(sub 0)", "(sub a=1)"];
        for i in &s {
            assert_rule!(Rule::subexpression, i);
        }
    }

    #[test]
    fn test_expression() {
        let s = vec![
            "{{exp}}",
            "{{(exp)}}",
            "{{../exp}}",
            "{{exp 1}}",
            "{{exp \"literal\"}}",
            "{{exp \"literal with space\"}}",
            "{{exp 'literal with space'}}",
            r#"{{exp "literal with escape \\\\"}}"#,
            "{{exp ref}}",
            "{{exp (sub)}}",
            "{{exp (sub 123)}}",
            "{{exp []}}",
            "{{exp {}}}",
            "{{exp key=1}}",
            "{{exp key=ref}}",
            "{{exp key='literal with space'}}",
            "{{exp key=\"literal with space\"}}",
            "{{exp key=(sub)}}",
            "{{exp key=(sub 0)}}",
            "{{exp key=(sub 0 key=1)}}",
        ];
        for i in &s {
            assert_rule!(Rule::expression, i);
        }
    }

    #[test]
    fn test_identifier_with_dash() {
        let s = ["{{exp-foo}}"];
        for i in &s {
            assert_rule!(Rule::expression, i);
        }
    }

    #[test]
    fn test_html_expression() {
        let s = [
            "{{{html}}}",
            "{{{(html)}}}",
            "{{{(html)}}}",
            "{{&html}}",
            "{{{html 1}}}",
            "{{{html p=true}}}",
            "{{{~ html}}}",
            "{{{html ~}}}",
            "{{{~ html ~}}}",
            "{{~{ html }~}}",
            "{{~{ html }}}",
            "{{{ html }~}}",
        ];
        for i in &s {
            assert_rule!(Rule::html_expression, i);
        }
    }

    #[test]
    fn test_helper_start() {
        let s = [
            "{{#if hello}}",
            "{{#if (hello)}}",
            "{{#if hello=world}}",
            "{{#if hello hello=world}}",
            "{{#if []}}",
            "{{#if {}}}",
            "{{#if}}",
            "{{~#if hello~}}",
            "{{#each people as |person|}}",
            "{{#each-obj obj as |val key|}}",
            "{{#each assets}}",
        ];
        for i in &s {
            assert_rule!(Rule::helper_block_start, i);
        }
    }

    #[test]
    fn test_helper_end() {
        let s = ["{{/if}}", "{{~/if}}", "{{~/if ~}}", "{{/if   ~}}"];
        for i in &s {
            assert_rule!(Rule::helper_block_end, i);
        }
    }

    #[test]
    fn test_helper_block() {
        let s = [
            "{{#if hello}}hello{{/if}}",
            "{{#if true}}hello{{/if}}",
            "{{#if nice ok=1}}hello{{/if}}",
            "{{#if}}hello{{else}}world{{/if}}",
            "{{#if}}hello{{^}}world{{/if}}",
            "{{#if}}{{#if}}hello{{/if}}{{/if}}",
            "{{#if}}hello{{~else}}world{{/if}}",
            "{{#if}}hello{{else~}}world{{/if}}",
            "{{#if}}hello{{~^~}}world{{/if}}",
            "{{#if}}{{/if}}",
            "{{#if}}hello{{else if}}world{{else}}test{{/if}}",
        ];
        for i in &s {
            assert_rule!(Rule::helper_block, i);
        }
    }

    #[test]
    fn test_raw_block() {
        let s = [
            "{{{{if hello}}}}good {{hello}}{{{{/if}}}}",
            "{{{{if hello}}}}{{#if nice}}{{/if}}{{{{/if}}}}",
        ];
        for i in &s {
            assert_rule!(Rule::raw_block, i);
        }
    }

    #[test]
    fn test_block_param() {
        let s = ["as |person|", "as |val key|"];
        for i in &s {
            assert_rule!(Rule::block_param, i);
        }
    }

    #[test]
    fn test_path() {
        let s = vec![
            "a",
            "a.b.c.d",
            "a.[0].[1].[2]",
            "a.[abc]",
            "a/v/c.d.s",
            "a.[0]/b/c/d",
            "a.[bb c]/b/c/d",
            "a.[0].[#hello]",
            "../a/b.[0].[1]",
            "this.[0]/[1]/this/a",
            "./this_name",
            "./goo/[/bar]",
            "a.[你好]",
            "a.[10].[#comment]",
            "a.[]", // empty key
            "./[/foo]",
            "[foo]",
            "@root/a/b",
            "nullable",
        ];
        for i in &s {
            assert_rule_match!(Rule::path, i);
        }
    }

    #[test]
    fn test_decorator_expression() {
        let s = ["{{* ssh}}", "{{~* ssh}}"];
        for i in &s {
            assert_rule!(Rule::decorator_expression, i);
        }
    }

    #[test]
    fn test_decorator_block() {
        let s = [
            "{{#* inline}}something{{/inline}}",
            "{{~#* inline}}hello{{/inline}}",
            "{{#* inline \"partialname\"}}something{{/inline}}",
        ];
        for i in &s {
            assert_rule!(Rule::decorator_block, i);
        }
    }

    #[test]
    fn test_partial_expression() {
        let s = [
            "{{> hello}}",
            "{{> (hello)}}",
            "{{~> hello a}}",
            "{{> hello a=1}}",
            "{{> (hello) a=1}}",
            "{{> hello.world}}",
            "{{> [a83?f4+.3]}}",
            "{{> 'anif?.bar'}}",
        ];
        for i in &s {
            assert_rule!(Rule::partial_expression, i);
        }
    }

    #[test]
    fn test_partial_block() {
        let s = ["{{#> hello}}nice{{/hello}}"];
        for i in &s {
            assert_rule!(Rule::partial_block, i);
        }
    }
}
