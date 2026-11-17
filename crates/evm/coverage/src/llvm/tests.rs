use super::*;
use crate::instrumented::{InstrumentedBuild, InstrumentedSource};
use foundry_compilers::artifacts::SourceCoverage;
use serde_json::{Value, json};
use std::{collections::BTreeMap, process::Command};

const SOURCE: &str = "\
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
    let inputs = build().function_inputs();
    assert_eq!(inputs.len(), 6);
    let object = CoverageObject::new(&inputs).unwrap();
    let ours = export_json(&CoverageMapping::new(inputs));

    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/C.sol"), SOURCE).unwrap();
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
        let theirs: Value = serde_json::from_str(&exported).unwrap();
        if let Some(difference) = diff(&normalize(ours.clone()), &normalize(theirs), "") {
            panic!("llvm-cov {version}: {difference}");
        }
    }
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
