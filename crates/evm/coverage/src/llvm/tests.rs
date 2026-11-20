use super::*;
use crate::instrumented::{InstrumentedBuild, InstrumentedSource};
use foundry_compilers::artifacts::SourceCoverage;
use serde_json::{Value, json};
use std::{collections::BTreeMap, process::Command};

pub(crate) const SOURCE: &str = "\
contract C {
    uint x;
    function f(uint a, uint b) public returns (uint) {
        if (a > 1 && b > 2) {
            x = 1;
        } else {
            x = 2;
        }
        return x;
    }
    modifier m() { x += 1; _; }
    function g() public m { x = 3; }
    function g2() public m { x = 4; }
    function k() public { if (false) { x = 9; } }
    function h() public {}
}
";

/// The `(start, len)` of the first occurrence of `needle` after `after`.
fn span(after: &str, needle: &str) -> (usize, usize) {
    let from = SOURCE.find(after).unwrap();
    (from + SOURCE[from..].find(needle).unwrap(), needle.len())
}

fn region(kind: u32, file: u32, after: &str, needle: &str, rest: &[i64]) -> Value {
    let (start, len) = span(after, needle);
    let mut array = vec![i64::from(kind), i64::from(file), start as i64, len as i64];
    array.extend(rest);
    json!(array)
}

/// Tagged counters.
const fn c(id: u32) -> i64 {
    (id << 2 | 1) as i64
}
const fn sub(id: u32) -> i64 {
    (id << 2 | 2) as i64
}
const fn add(id: u32) -> i64 {
    (id << 2 | 3) as i64
}

pub(crate) fn coverage() -> SourceCoverage {
    let f_body = "function f(uint a, uint b) public returns (uint) {";
    let body = |start: &str, end: &str| {
        let from = SOURCE.find(start).unwrap();
        let to = from + SOURCE[from..].find(end).unwrap() + end.len();
        (from, to - from)
    };
    let (fs, fl) = body(f_body, "return x;\n    }");
    let (gs, gl) = body("function g()", "x = 3; }");
    let (g2s, g2l) = body("function g2()", "x = 4; }");
    let (ms, ml) = body("modifier m()", "_; }");
    let (ks, kl) = body("function k()", "x = 9; } }");
    let (hs, hl) = body("function h()", "{}");
    let code = |s: usize, l: usize, file: u32, count: i64| json!([0, file, s, l, count]);
    let m_body = |file: u32, count: i64| {
        [code(ms, ml, file, count), region(0, file, "modifier m()", "x += 1;", &[count])]
    };
    serde_json::from_value(json!({
        "version": 2,
        "functions": [
            {
                "name": "src/C.sol:C.f(uint256,uint256)",
                "hash": "0x1111111111111111",
                "counters": [1, 3],
                "files": [0],
                // 0: c1 - c2, 1: c2 - c3, 2: c1 - c3.
                "expressions": [[c(1), c(2)], [c(2), c(3)], [c(1), c(3)]],
                "regions": [
                    code(fs, fl, 0, c(1)),
                    [5, 0, span("if (a", "a > 1 && b > 2").0, 14, 0],
                    region(6, 0, "if (a", "a > 1", &[c(2), sub(0), 0, -1, 1]),
                    region(6, 0, "if (a", "b > 2", &[c(3), sub(1), 1, -1, -1]),
                    region(0, 0, "if (a", "{\n            x = 1;\n        }", &[c(3)]),
                    region(0, 0, "else", "{\n            x = 2;\n        }", &[sub(2)]),
                    region(0, 0, "return x", "return x;", &[c(1)]),
                ],
                "decisions": [{"id": 0, "conditions": 2, "vectors": ["FFF", "TFF", "TTT"]}],
            },
            {
                "name": "src/C.sol:C.m()",
                "hash": "0x2222222222222222",
                "counters": [10, 0],
                "files": [0],
                // The modifier's count is the sum of its two expansions' counters, 12 and 14.
                "expressions": [[c(12), c(14)]],
                "regions": [m_body(0, add(0))[0], m_body(0, add(0))[1]],
            },
            {
                "name": "src/C.sol:C.g()",
                "hash": "0x3333333333333333",
                "counters": [11, 2],
                "files": [0, 0],
                "regions": [
                    code(gs, gl, 0, c(11)),
                    [1, 0, span("function g()", "m {").0, 1, c(12), 1],
                    m_body(1, c(12))[0],
                    m_body(1, c(12))[1],
                    region(0, 0, "function g()", "x = 3;", &[c(12)]),
                ],
            },
            {
                "name": "src/C.sol:C.g2()",
                "hash": "0x4444444444444444",
                "counters": [13, 2],
                "files": [0, 0],
                "regions": [
                    code(g2s, g2l, 0, c(13)),
                    [1, 0, span("function g2()", "m {").0, 1, c(14), 1],
                    m_body(1, c(14))[0],
                    m_body(1, c(14))[1],
                    region(0, 0, "function g2()", "x = 4;", &[c(14)]),
                ],
            },
            {
                "name": "src/C.sol:C.k()",
                "hash": "0x5555555555555555",
                "counters": [15, 1],
                "files": [0],
                "regions": [
                    code(ks, kl, 0, c(15)),
                    region(2, 0, "function k()", "{ x = 9; }", &[]),
                ],
            },
            {
                "name": "src/C.sol:C.h()",
                "hash": "0x6666666666666666",
                "counters": [16, 1],
                "files": [0],
                "regions": [code(hs, hl, 0, c(16))],
            },
        ]
    }))
    .unwrap()
}

/// Counter hits: `f` ran three times, `g` twice, `g2` once.
pub(crate) fn build() -> InstrumentedBuild {
    let source = InstrumentedSource::new("src/C.sol".into(), SOURCE.to_string(), true, coverage());
    let mut build = InstrumentedBuild::new(BTreeMap::from([(0, source)]));
    let counters = [(1, 3), (2, 2), (3, 1), (11, 2), (12, 2), (13, 1), (14, 1), (15, 1)];
    // The decision ran its first vector, `a > 1` false, then `a > 1 && b > 2` false and true.
    build.add_hits(counters.into_iter(), [(0, 0), (0, 1), (0, 2)].into_iter(), &[]);
    build
}

fn run(program: &str, dir: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new(program).current_dir(dir).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Without LLVM's tools installed there is nothing to compare against, and the test passes.
#[test]
fn llvm_tools_read_the_coverage_mapping() {
    let build = build();
    assert_eq!(build.function_inputs().len(), 6);
    assert_llvm_agrees(SOURCE, &build);
}

/// A function that never ran has branches that no side of was taken: LCOV prints `-` for them.
#[test]
fn branches_of_code_that_never_ran_print_a_dash() {
    let source = InstrumentedSource::new("src/C.sol".into(), SOURCE.to_string(), true, coverage());
    let build = InstrumentedBuild::new(BTreeMap::from([(0, source)]));
    let lcov = export_lcov(&CoverageMapping::new(build.function_inputs()));
    assert!(lcov.contains("BRDA:4,0,0,-\nBRDA:4,0,1,-\n"), "{lcov}");
    assert_llvm_agrees(SOURCE, &build);
}

/// Asserts that `llvm-cov export` over the coverage object of the build equals the JSON of the
/// build's coverage mapping, with every installed LLVM version.
fn assert_llvm_agrees(source: &str, build: &InstrumentedBuild) {
    assert_inputs_agree(source, build.function_inputs());
}

/// Asserts the same for function records, which may combine several builds.
pub(crate) fn assert_inputs_agree(source: &str, inputs: Vec<FunctionInput>) {
    assert_inputs_agree_in("src/C.sol", source, inputs);
}

/// Asserts the same for a source at `path`, such as `src/B.sol`.
pub(crate) fn assert_inputs_agree_in(path: &str, source: &str, inputs: Vec<FunctionInput>) {
    let object = CoverageObject::new(&inputs).unwrap();
    let mapping = CoverageMapping::new(inputs);
    let ours_text = export_json(&mapping);
    let ours: Value = serde_json::from_str(&ours_text).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join(path), source).unwrap();
    std::fs::write(dir.join("coverage.proftext"), &object.proftext).unwrap();
    std::fs::write(dir.join("coverage.covmap.o"), &object.object).unwrap();

    for version in ["22", "23"] {
        let (profdata, cov) = (format!("llvm-profdata-{version}"), format!("llvm-cov-{version}"));
        if Command::new(&cov).arg("--version").output().is_err() {
            continue;
        }
        run(&profdata, dir, &["merge", "-o", "coverage.profdata", "coverage.proftext"]);
        let exported = run(
            &cov,
            dir,
            &["export", "--instr-profile", "coverage.profdata", "coverage.covmap.o"],
        );
        // LLVM 23 prints the JSON byte for byte as Foundry does. LLVM 22 sorts the members.
        if version == "23" {
            assert_eq!(exported, ours_text, "llvm-cov {version}");
        }
        let lcov = run(
            &cov,
            dir,
            &[
                "export",
                "-format=lcov",
                "--instr-profile",
                "coverage.profdata",
                "coverage.covmap.o",
            ],
        );
        assert_eq!(lcov, export_lcov(&mapping), "{cov}");
        let theirs: Value = serde_json::from_str(&exported).unwrap();
        if let Some(difference) = diff(&normalize(ours.clone()), &normalize(theirs), "") {
            panic!("llvm-cov {version}: {difference}");
        }
    }
}

const GAP_SOURCE: &str = "\
contract G {
    uint x;

    function f(uint a) public {
        if (a > 1) {
            x = 1;
        } else {
            x = 2;
        }

        // keep
        x = 3;

    }
}
";

/// A function with gap regions between its statements and skipped regions over a blank line, a
/// comment, and a lone closing brace, as Solar emits them.
fn gap_build() -> InstrumentedBuild {
    let at = |needle: &str| {
        let start = GAP_SOURCE.find(needle).unwrap();
        (start, needle.len())
    };
    let region = |kind: i64, needle: &str, rest: &[i64]| {
        let (start, len) = at(needle);
        let mut region = vec![kind, 0, start as i64, len as i64];
        region.extend(rest);
        json!(region)
    };
    // The text between the end of `from` and the start of `to`.
    let between = |kind: i64, from: &str, to: &str, rest: &[i64]| {
        let (start, len) = at(from);
        let end = GAP_SOURCE.find(to).unwrap();
        let start = start + len;
        let mut region = vec![kind, 0, start as i64, (end - start) as i64];
        region.extend(rest);
        json!(region)
    };
    let body = "function f(uint a) public {";
    let (body_start, _) = at(body);
    let body_end = GAP_SOURCE.rfind("    }\n}").unwrap() + "    }".len();
    // Counter 1 is the entry and counter 2 the `if` block, so expression 0, 1 - 2, is the `else`.
    let coverage = serde_json::from_value(json!({
        "version": 2,
        "functions": [{
            "name": "src/C.sol:G.f(uint256)", "hash": "0x7", "counters": [1, 2], "files": [0],
            "expressions": [[c(1), c(2)]],
            "regions": [
                [0, 0, body_start, body_end - body_start, c(1)],
                region(4, "a > 1", &[c(2), sub(0)]),
                region(0, "{\n            x = 1;\n        }", &[c(2)]),
                between(3, "x = 1;\n        }", "{\n            x = 2;", &[sub(0)]),
                region(0, "{\n            x = 2;\n        }", &[sub(0)]),
                between(3, "x = 2;\n        }", "x = 3;", &[c(1)]),
                // The blank line, then the comment with its line break.
                [2, 0, GAP_SOURCE.find("}\n\n").unwrap() + 2, 1],
                region(2, "        // keep\n", &[]),
                region(0, "x = 3;", &[c(1)]),
                // A blank line after a statement.
                [2, 0, GAP_SOURCE.find("x = 3;\n\n").unwrap() + "x = 3;\n".len(), 1],
            ],
        }]
    }))
    .unwrap();
    let source =
        InstrumentedSource::new("src/C.sol".into(), GAP_SOURCE.to_string(), true, coverage);
    let mut build = InstrumentedBuild::new(BTreeMap::from([(0, source)]));
    build.add_hits([(1, 3), (2, 1)].into_iter(), std::iter::empty(), &[]);
    build
}

#[test]
fn gap_and_skipped_regions_follow_llvm_cov() {
    let build = gap_build();
    assert_llvm_agrees(GAP_SOURCE, &build);

    // `llvm-cov export` agrees with these lines above. A gap never starts a line's count, and a
    // line that starts a skipped region, like the blank line and the comment, is not mapped.
    let mapping = CoverageMapping::new(build.function_inputs());
    let lines = mapping
        .coverage_for_file("src/C.sol")
        .line_stats()
        .into_iter()
        .filter(|stats| stats.mapped)
        .map(|stats| (stats.line, stats.execution_count))
        .collect::<Vec<_>>();
    assert_eq!(lines, [(4, 3), (5, 3), (6, 1), (7, 2), (8, 2), (9, 2), (12, 3), (14, 3)]);
}

const MODIFIER_SOURCE: &str = "\
contract M {
    uint x;
    modifier m(uint a, uint b) { require(a > 1 && b > 2); _; }
    function f(uint a, uint b) public m(a, b) { x = 1; }
    function g(uint a, uint b) public m(a, b) { x = 2; }
}
";

/// Two functions that run one modifier with a decision, and the modifier's own record, which sums
/// their counters: its header counts the entries, and its branch and MC/DC regions hold the sums.
fn modifier_build() -> InstrumentedBuild {
    let at = |needle: &str| {
        let start = MODIFIER_SOURCE.find(needle).unwrap();
        (start as i64, needle.len() as i64)
    };
    let region = |kind: i64, file: i64, needle: &str, rest: &[i64]| {
        let (start, len) = at(needle);
        let mut region = vec![kind, file, start, len];
        region.extend(rest);
        json!(region)
    };
    let mcdc_branches = |file: i64, (true_a, false_a, true_b, false_b): (i64, i64, i64, i64)| {
        [
            region(6, file, "a > 1", &[true_a, false_a, 0, -1, 1]),
            region(6, file, "b > 2", &[true_b, false_b, 1, -1, -1]),
        ]
    };
    let user = |name: &str, hash: &str, first: u32, call: &str, body: &str| {
        let c = |i: u32| c(first + i);
        // Counters: the entry, the modifier body, `a > 1` and `b > 2` held, and `_;`. Expressions
        // 0 and 1 are the false sides of the two conditions.
        let [a, b] = mcdc_branches(1, (c(2), sub(0), c(3), sub(1)));
        json!({
            "name": name, "hash": hash, "counters": [first, 5], "files": [0, 0],
            "expressions": [[c(1), c(2)], [c(2), c(3)]],
            "regions": [
                region(0, 0, body, &[c(0)]),
                [1, 0, at(call).0, at(call).1, c(1), 1],
                region(0, 0, &format!("{{ x = {}; }}", if first == 1 { 1 } else { 2 }), &[c(4)]),
                region(0, 1, "{ require(a > 1 && b > 2); _; }", &[c(1)]),
                region(5, 1, "a > 1 && b > 2", &[0]),
                a, b,
                region(0, 1, "_;", &[c(4)]),
            ],
            "decisions": [{"id": 10 + first / 6, "conditions": 2, "vectors": ["F-F", "TFF", "TTT"]}],
        })
    };
    let own_b = mcdc_branches(0, (add(1), sub(3), add(2), sub(4)));
    // Counters 2 and 7 are the modifier bodies, 3 and 8 `a > 1`, and 4 and 9 `b > 2`.
    let coverage = serde_json::from_value(json!({
        "version": 2,
        "functions": [
            user(
                "src/C.sol:M.f(uint256,uint256)", "0x1", 1, "m(a, b) {",
                "function f(uint a, uint b) public m(a, b) { x = 1; }",
            ),
            user(
                "src/C.sol:M.g(uint256,uint256)", "0x2", 6, "m(a, b) {",
                "function g(uint a, uint b) public m(a, b) { x = 2; }",
            ),
            {
                "name": "src/C.sol:M.m(uint256,uint256)", "hash": "0x3", "counters": [20, 0],
                "files": [0],
                "expressions": [[c(2), c(7)], [c(3), c(8)], [c(4), c(9)], [add(0), add(1)],
                    [add(1), add(2)]],
                "regions": [
                    region(0, 0, "modifier m(uint a, uint b) ", &[add(0)]),
                    region(5, 0, "a > 1 && b > 2", &[0]),
                    own_b[0], own_b[1],
                ],
                "decisions": [{"expansions": [10, 11], "conditions": 2,
                    "vectors": ["F-F", "TFF", "TTT"]}],
            },
        ]
    }))
    .unwrap();
    let source =
        InstrumentedSource::new("src/C.sol".into(), MODIFIER_SOURCE.to_string(), true, coverage);
    let mut build = InstrumentedBuild::new(BTreeMap::from([(0, source)]));
    // `f` ran three times, `a > 1` held twice and `b > 2` once, and `g` once with both.
    let counters =
        [(1, 3), (2, 3), (3, 2), (4, 1), (5, 1), (6, 1), (7, 1), (8, 1), (9, 1), (10, 1)];
    // `f` ran all three vectors, `g` only `TTT`.
    let vectors = [(10, 0), (10, 1), (10, 2), (11, 2)];
    build.add_hits(counters.into_iter(), vectors.into_iter(), &[]);
    build
}

#[test]
fn a_modifier_record_sums_the_decisions_of_its_expansions() {
    let build = modifier_build();
    assert_llvm_agrees(MODIFIER_SOURCE, &build);

    // The modifier's record reports every vector that one of its expansions executed.
    let inputs = build.function_inputs();
    let modifier =
        inputs.iter().find(|input| input.name.ends_with("M.m(uint256,uint256)")).unwrap();
    assert_eq!(modifier.bitmap, [0b111]);
    let user = |name: &str| inputs.iter().find(|input| input.name.contains(name)).unwrap();
    assert_eq!(user("M.f(").bitmap, [0b111]);
    assert_eq!(user("M.g(").bitmap, [0b100]);
}

/// Makes integer-valued floats integers and rounds the others, so that numbers compare by value:
/// `serde_json` does not parse floats exactly.
fn normalize(value: Value) -> Value {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => json!(f as i64),
            Some(f) => json!((f * 1e9).round() as i64),
            None => Value::Number(n),
        },
        Value::Array(a) => Value::Array(a.into_iter().map(normalize).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, normalize(v))).collect()),
        other => other,
    }
}

/// The path and values of the first difference between two JSON values.
fn diff(ours: &Value, theirs: &Value, path: &str) -> Option<String> {
    match (ours, theirs) {
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!(
                    "{path}: {} items vs {}\nours: {ours}\ntheirs: {theirs}",
                    a.len(),
                    b.len()
                ));
            }
            a.iter().zip(b).enumerate().find_map(|(i, (a, b))| diff(a, b, &format!("{path}/{i}")))
        }
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()) {
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => {
                        if let Some(d) = diff(a, b, &format!("{path}/{key}")) {
                            return Some(d);
                        }
                    }
                    (a, b) => return Some(format!("{path}/{key}: {a:?} vs {b:?}")),
                }
            }
            None
        }
        _ => (ours != theirs).then(|| format!("{path}: {ours} vs {theirs}")),
    }
}
