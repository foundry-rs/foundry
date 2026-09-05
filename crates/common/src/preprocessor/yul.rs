use alloy_json_abi::{Function, JsonAbi, StateMutability};
use alloy_primitives::keccak256;
use foundry_compilers::{
    Compiler, ProjectPathsConfig,
    artifacts::{Contract, SolcLanguage, Source, Sources},
    error::{Result, SolcError},
    multi::{MultiCompiler, MultiCompilerInput, MultiCompilerLanguage},
    project::Preprocessor,
    solc::{SolcCompiler, SolcVersionedInput},
};
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Debug, Default)]
pub struct YulTestPreprocessor {
    tests: Mutex<HashMap<PathBuf, Vec<String>>>,
}

impl Preprocessor<SolcCompiler> for YulTestPreprocessor {
    fn preprocess(
        &self,
        _compiler: &SolcCompiler,
        _input: &mut SolcVersionedInput,
        _paths: &ProjectPathsConfig<SolcLanguage>,
        _mocks: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        Ok(())
    }
}

impl Preprocessor<MultiCompiler> for YulTestPreprocessor {
    fn preprocess(
        &self,
        _compiler: &MultiCompiler,
        _input: &mut MultiCompilerInput,
        _paths: &ProjectPathsConfig<MultiCompilerLanguage>,
        _mocks: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        Ok(())
    }

    fn preprocess_inputs(
        &self,
        _compiler: &MultiCompiler,
        input: MultiCompilerInput,
        paths: &ProjectPathsConfig<MultiCompilerLanguage>,
        _mocks: &mut HashSet<PathBuf>,
    ) -> Result<Vec<MultiCompilerInput>> {
        let MultiCompilerInput::Solc(input) = input else { return Ok(vec![input]) };
        if input.input.language != SolcLanguage::Yul {
            return Ok(vec![MultiCompilerInput::Solc(input)]);
        }

        let test_paths = input
            .input
            .sources
            .keys()
            .filter(|path| is_yul_test(path, paths))
            .cloned()
            .collect::<Vec<_>>();
        if test_paths.is_empty() {
            return Ok(vec![MultiCompilerInput::Solc(input)]);
        }

        let mut inputs = Vec::with_capacity(test_paths.len());
        for test_path in test_paths {
            let mut visited = HashSet::new();
            let mut modules = Vec::new();
            collect_modules(
                &test_path,
                &input.input.sources,
                paths,
                &mut visited,
                &mut HashSet::new(),
                &mut modules,
            )?;

            let mut definitions = String::new();
            let mut names = HashSet::new();
            let mut entrypoints = Vec::new();
            for (path, content) in modules {
                if path != test_path
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.ends_with(".t.yul"))
                {
                    return Err(SolcError::msg(format!(
                        "Yul test suite `{}` cannot import another test suite `{}`",
                        test_path.display(),
                        path.display()
                    )));
                }
                let functions = parse_module(&content).map_err(|error| {
                    SolcError::msg(format!("invalid Yul module `{}`: {error}", path.display()))
                })?;
                for function in &functions {
                    if !names.insert(function.name.to_string()) {
                        return Err(SolcError::msg(format!(
                            "duplicate Yul function `{}` in test suite `{}`",
                            function.name,
                            test_path.display()
                        )));
                    }
                    if path == test_path
                        && (function.name.starts_with("test") || function.name == "setUp")
                    {
                        if function.name.starts_with("testFail") {
                            return Err(SolcError::msg(format!(
                                "legacy `testFail*` Yul tests are not supported; use an explicit assertion in `{}`",
                                function.name
                            )));
                        }
                        if !function.parameters.is_empty() || !function.returns.is_empty() {
                            return Err(SolcError::msg(format!(
                                "Yul test entrypoint `{}` must not have parameters or return values",
                                function.name
                            )));
                        }
                        entrypoints.push(function.name.to_string());
                    }
                }
                definitions.push_str(&without_imports(&content));
                definitions.push('\n');
            }

            if entrypoints.iter().all(|name| name == "setUp") {
                return Err(SolcError::msg(format!(
                    "no test functions found in `{}`",
                    test_path.display()
                )));
            }
            if entrypoints.iter().filter(|name| name.as_str() == "setUp").count() > 1 {
                return Err(SolcError::msg(format!(
                    "multiple `setUp` functions found in `{}`",
                    test_path.display()
                )));
            }

            let suite_name = suite_name(&test_path)?;
            let generated = generate_harness(&suite_name, &entrypoints, &definitions)?;
            self.tests
                .lock()
                .map_err(|_| SolcError::msg("Yul test metadata lock poisoned"))?
                .insert(test_path.clone(), entrypoints);

            let mut split = (*input).clone();
            split.input.sources = Sources::from_iter([(test_path, Source::new(generated))]);
            // Solc emits no contract entry for Yul when only ABI output is requested. Forge's
            // filtered discovery and `--list` paths use ABI-only compilation, so request the
            // smallest real Yul output needed for postprocessing to attach the synthetic ABI.
            let outputs = split
                .input
                .settings
                .output_selection
                .0
                .entry("*".to_string())
                .or_default()
                .entry("*".to_string())
                .or_default();
            if !outputs.iter().any(|output| output == "evm.bytecode.object") {
                outputs.push("evm.bytecode.object".to_string());
            }
            inputs.push(MultiCompilerInput::Solc(Box::new(split)));
        }
        Ok(inputs)
    }

    fn postprocess(
        &self,
        _input: &MultiCompilerInput,
        output: &mut foundry_compilers::CompilerOutput<
            <MultiCompiler as Compiler>::CompilationError,
            Contract,
        >,
    ) -> Result<()> {
        let tests =
            self.tests.lock().map_err(|_| SolcError::msg("Yul test metadata lock poisoned"))?;
        for (path, contracts) in &mut output.contracts {
            let Some(entrypoints) = tests.get(path) else { continue };
            let mut abi = JsonAbi::new();
            for name in entrypoints {
                abi.functions.entry(name.clone()).or_default().push(Function {
                    name: name.clone(),
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                    state_mutability: StateMutability::NonPayable,
                });
            }
            for contract in contracts.values_mut() {
                contract.abi = Some(abi.clone());
            }
        }
        Ok(())
    }
}

fn is_yul_test(path: &Path, paths: &ProjectPathsConfig<MultiCompilerLanguage>) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.ends_with(".t.yul"))
        && paths.is_test(&paths.root.join(path))
}

fn suite_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".t.yul"))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| SolcError::msg(format!("invalid Yul test path `{}`", path.display())))
}

fn collect_modules(
    path: &Path,
    sources: &Sources,
    paths: &ProjectPathsConfig<MultiCompilerLanguage>,
    visited: &mut HashSet<PathBuf>,
    visiting: &mut HashSet<PathBuf>,
    modules: &mut Vec<(PathBuf, String)>,
) -> Result<()> {
    if visited.contains(path) {
        return Ok(());
    }
    if !visiting.insert(path.to_path_buf()) {
        return Err(SolcError::msg(format!("cyclic Yul import involving `{}`", path.display())));
    }
    let source = sources.get(path).ok_or_else(|| {
        SolcError::msg(format!("Yul import `{}` is not present in compiler input", path.display()))
    })?;
    for import in foundry_compilers::utils::find_yul_imports(&source.content) {
        let absolute = if path.is_absolute() { path.to_path_buf() } else { paths.root.join(path) };
        let parent = absolute.parent().unwrap_or(&paths.root);
        let resolved = paths.resolve_import(parent, Path::new(import.path))?;
        let resolved =
            resolved.strip_prefix(&paths.root).map(Path::to_path_buf).unwrap_or(resolved);
        collect_modules(&resolved, sources, paths, visited, visiting, modules)?;
    }
    visiting.remove(path);
    visited.insert(path.to_path_buf());
    modules.push((path.to_path_buf(), source.content.to_string()));
    Ok(())
}

fn without_imports(source: &str) -> String {
    let mut output = source.to_string();
    for import in foundry_compilers::utils::find_yul_imports(source).into_iter().rev() {
        output.replace_range(import.statement, "");
    }
    output
}

fn generate_harness(name: &str, entrypoints: &[String], definitions: &str) -> Result<String> {
    let mut selectors = HashSet::new();
    let mut dispatch = String::new();
    for entrypoint in entrypoints {
        let hash = keccak256(format!("{entrypoint}()"));
        let selector = &hash[..4];
        if !selectors.insert(selector.to_vec()) {
            return Err(SolcError::msg(format!("selector collision for `{entrypoint}()`")));
        }
        writeln!(
            dispatch,
            "            case 0x{} {{ {entrypoint}() stop() }}",
            alloy_primitives::hex::encode(selector)
        )
        .unwrap();
    }

    Ok(format!(
        r#"object "{name}" {{
    code {{
        datacopy(0, dataoffset("runtime"), datasize("runtime"))
        return(0, datasize("runtime"))
    }}
    object "runtime" {{
        code {{
            if lt(calldatasize(), 4) {{ revert(0, 0) }}
            switch shr(224, calldataload(0))
{dispatch}            default {{ revert(0, 0) }}
{definitions}
        }}
    }}
}}
"#
    ))
}

#[derive(Debug)]
struct YulFunction<'a> {
    name: &'a str,
    parameters: &'a str,
    returns: &'a str,
}

fn parse_module(source: &str) -> std::result::Result<Vec<YulFunction<'_>>, String> {
    let imports = foundry_compilers::utils::find_yul_imports(source);
    let mut functions = Vec::new();
    let mut cursor = 0;
    loop {
        cursor = skip_trivia(source, cursor)?;
        if cursor == source.len() {
            break;
        }
        if let Some(import) = imports.iter().find(|import| import.statement.start == cursor) {
            cursor = import.statement.end;
            continue;
        }
        let (keyword, next) = identifier(source, cursor)?;
        if keyword != "function" {
            return Err(format!("expected `function` or `import` at byte {cursor}"));
        }
        cursor = skip_trivia(source, next)?;
        let (name, next) = identifier(source, cursor)?;
        cursor = skip_trivia(source, next)?;
        let (parameters, next) = delimited(source, cursor, b'(', b')')?;
        cursor = skip_trivia(source, next)?;
        let returns = if source[cursor..].starts_with("->") {
            cursor = skip_trivia(source, cursor + 2)?;
            let body = find_unquoted(source, cursor, b'{')?;
            let returns = source[cursor..body].trim();
            cursor = body;
            returns
        } else {
            ""
        };
        let (_, next) = delimited(source, cursor, b'{', b'}')?;
        functions.push(YulFunction { name, parameters: parameters.trim(), returns });
        cursor = next;
    }
    Ok(functions)
}

fn skip_trivia(source: &str, mut cursor: usize) -> std::result::Result<usize, String> {
    let bytes = source.as_bytes();
    loop {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"//") {
            cursor += 2;
            while bytes.get(cursor).is_some_and(|byte| *byte != b'\n') {
                cursor += 1;
            }
        } else if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            let Some(end) = source[cursor + 2..].find("*/") else {
                return Err("unterminated block comment".to_string());
            };
            cursor += end + 4;
        } else {
            return Ok(cursor);
        }
    }
}

fn identifier(source: &str, start: usize) -> std::result::Result<(&str, usize), String> {
    let bytes = source.as_bytes();
    if !bytes.get(start).is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_') {
        return Err(format!("expected identifier at byte {start}"));
    }
    let mut end = start + 1;
    while bytes.get(end).is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_') {
        end += 1;
    }
    Ok((&source[start..end], end))
}

fn delimited(
    source: &str,
    start: usize,
    open: u8,
    close: u8,
) -> std::result::Result<(&str, usize), String> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&open) {
        return Err(format!("expected `{}` at byte {start}", open as char));
    }
    let mut depth = 1usize;
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'/' if bytes.get(cursor + 1) == Some(&b'/') => {
                cursor += 2;
                while bytes.get(cursor).is_some_and(|byte| *byte != b'\n') {
                    cursor += 1;
                }
            }
            b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
                let Some(end) = source[cursor + 2..].find("*/") else {
                    return Err("unterminated block comment".to_string());
                };
                cursor += end + 4;
            }
            b'"' | b'\'' => cursor = skip_string(source, cursor)?,
            byte if byte == open => {
                depth += 1;
                cursor += 1;
            }
            byte if byte == close => {
                depth -= 1;
                if depth == 0 {
                    return Ok((&source[start + 1..cursor], cursor + 1));
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    Err(format!("unterminated `{}`", open as char))
}

fn find_unquoted(
    source: &str,
    mut cursor: usize,
    needle: u8,
) -> std::result::Result<usize, String> {
    let bytes = source.as_bytes();
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'/' if bytes.get(cursor + 1) == Some(&b'/') => {
                cursor += 2;
                while bytes.get(cursor).is_some_and(|byte| *byte != b'\n') {
                    cursor += 1;
                }
            }
            b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
                let Some(end) = source[cursor + 2..].find("*/") else {
                    return Err("unterminated block comment".to_string());
                };
                cursor += end + 4;
            }
            b'"' | b'\'' => cursor = skip_string(source, cursor)?,
            byte if byte == needle => return Ok(cursor),
            _ => cursor += 1,
        }
    }
    Err(format!("expected `{}`", needle as char))
}

fn skip_string(source: &str, start: usize) -> std::result::Result<usize, String> {
    let bytes = source.as_bytes();
    let quote = bytes[start];
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor += 2,
            byte if byte == quote => return Ok(cursor + 1),
            _ => cursor += 1,
        }
    }
    Err("unterminated string".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_functions_without_comment_false_positives() {
        let functions = parse_module(
            r#"
import "Math.yul"
// function test_ignored() {}
function helper(a, b) -> result { result := add(a, b) }
function test_example() { let value := "function test_fake() {}" }
"#,
        )
        .unwrap();
        assert_eq!(
            functions.iter().map(|function| function.name).collect::<Vec<_>>(),
            ["helper", "test_example"]
        );
        assert_eq!(functions[0].parameters, "a, b");
        assert_eq!(functions[0].returns, "result");
    }

    #[test]
    fn generates_dispatcher() {
        let harness = generate_harness(
            "Example",
            &["setUp".to_string(), "test_example".to_string()],
            "function setUp() {} function test_example() {}",
        )
        .unwrap();
        assert!(harness.contains("case 0x0a9254e4 { setUp() stop() }"));
        assert!(harness.contains("case 0x22c34ba7 { test_example() stop() }"));
    }
}
