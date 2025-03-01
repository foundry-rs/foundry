use super::VyperLanguage;
use crate::{
    compilers::{vyper::VYPER_EXTENSIONS, ParsedSource},
    ProjectPathsConfig,
};
use foundry_compilers_core::{
    error::{Result, SolcError},
    utils::{capture_outer_and_inner, RE_VYPER_VERSION},
};
use semver::VersionReq;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use winnow::{
    ascii::space1,
    combinator::{alt, opt, preceded},
    token::{take_till, take_while},
    ModalResult, Parser,
};

#[derive(Clone, Debug, PartialEq)]
pub struct VyperImport {
    pub level: usize,
    pub path: Option<String>,
    pub final_part: Option<String>,
}

#[derive(Clone, Debug)]
pub struct VyperParsedSource {
    path: PathBuf,
    version_req: Option<VersionReq>,
    imports: Vec<VyperImport>,
}

impl ParsedSource for VyperParsedSource {
    type Language = VyperLanguage;

    fn parse(content: &str, file: &Path) -> Result<Self> {
        let version_req = capture_outer_and_inner(content, &RE_VYPER_VERSION, &["version"])
            .first()
            .and_then(|(cap, _)| VersionReq::parse(cap.as_str()).ok());

        let imports = parse_imports(content);

        let path = file.to_path_buf();

        Ok(Self { path, version_req, imports })
    }

    fn version_req(&self) -> Option<&VersionReq> {
        self.version_req.as_ref()
    }

    fn contract_names(&self) -> &[String] {
        &[]
    }

    fn language(&self) -> Self::Language {
        VyperLanguage
    }

    fn resolve_imports<C>(
        &self,
        paths: &ProjectPathsConfig<C>,
        include_paths: &mut BTreeSet<PathBuf>,
    ) -> Result<Vec<PathBuf>> {
        let mut imports = Vec::new();
        'outer: for import in &self.imports {
            // skip built-in imports
            if import.level == 0
                && import
                    .path
                    .as_ref()
                    .map(|path| path.starts_with("vyper.") || path.starts_with("ethereum.ercs"))
                    .unwrap_or_default()
            {
                continue;
            }

            // Potential locations of imported source.
            let mut candidate_dirs = Vec::new();

            // For relative imports, vyper always checks only directory containing contract which
            // includes given import.
            if import.level > 0 {
                let mut candidate_dir = Some(self.path.as_path());

                for _ in 0..import.level {
                    candidate_dir = candidate_dir.and_then(|dir| dir.parent());
                }

                let candidate_dir = candidate_dir.ok_or_else(|| {
                    SolcError::msg(format!(
                        "Could not go {} levels up for import at {}",
                        import.level,
                        self.path.display()
                    ))
                })?;

                candidate_dirs.push(candidate_dir);
            } else {
                // For absolute imports, Vyper firstly checks current directory, and then root.
                if let Some(parent) = self.path.parent() {
                    candidate_dirs.push(parent);
                }
                candidate_dirs.push(paths.root.as_path());
            }

            candidate_dirs.extend(paths.libraries.iter().map(PathBuf::as_path));

            let import_path = {
                let mut path = PathBuf::new();

                if let Some(import_path) = &import.path {
                    path = path.join(import_path.replace('.', "/"));
                }

                if let Some(part) = &import.final_part {
                    path = path.join(part);
                }

                path
            };

            for candidate_dir in candidate_dirs {
                let candidate = candidate_dir.join(&import_path);
                for extension in VYPER_EXTENSIONS {
                    let candidate = candidate.clone().with_extension(extension);
                    trace!("trying {}", candidate.display());
                    if candidate.exists() {
                        imports.push(candidate);
                        include_paths.insert(candidate_dir.to_path_buf());
                        continue 'outer;
                    }
                }
            }

            return Err(SolcError::msg(format!(
                "failed to resolve import {}{} at {}",
                ".".repeat(import.level),
                import_path.display(),
                self.path.display()
            )));
        }
        Ok(imports)
    }
}

/// Parses given source trying to find all import directives.
fn parse_imports(content: &str) -> Vec<VyperImport> {
    let mut imports = Vec::new();

    for mut line in content.split('\n') {
        if let Ok(parts) = parse_import(&mut line) {
            imports.push(parts);
        }
    }

    imports
}

/// Parses given input, trying to find (import|from) part1.part2.part3 (import part4)?
fn parse_import(input: &mut &str) -> ModalResult<VyperImport> {
    (
        preceded(
            (alt(["from", "import"]), space1),
            (take_while(0.., |c| c == '.'), take_till(0.., [' '])),
        ),
        opt(preceded((space1, "import", space1), take_till(0.., [' ']))),
    )
        .parse_next(input)
        .map(|((dots, path), last)| VyperImport {
            level: dots.len(),
            path: (!path.is_empty()).then(|| path.to_string()),
            final_part: last.map(|p| p.to_string()),
        })
}

#[cfg(test)]
mod tests {
    use super::{parse_import, VyperImport};
    use winnow::Parser;

    #[test]
    fn can_parse_import() {
        assert_eq!(
            parse_import.parse("import one.two.three").unwrap(),
            VyperImport { level: 0, path: Some("one.two.three".to_string()), final_part: None }
        );
        assert_eq!(
            parse_import.parse("from one.two.three import four").unwrap(),
            VyperImport {
                level: 0,
                path: Some("one.two.three".to_string()),
                final_part: Some("four".to_string()),
            }
        );
        assert_eq!(
            parse_import.parse("from one import two").unwrap(),
            VyperImport {
                level: 0,
                path: Some("one".to_string()),
                final_part: Some("two".to_string()),
            }
        );
        assert_eq!(
            parse_import.parse("import one").unwrap(),
            VyperImport { level: 0, path: Some("one".to_string()), final_part: None }
        );
        assert_eq!(
            parse_import.parse("from . import one").unwrap(),
            VyperImport { level: 1, path: None, final_part: Some("one".to_string()) }
        );
        assert_eq!(
            parse_import.parse("from ... import two").unwrap(),
            VyperImport { level: 3, path: None, final_part: Some("two".to_string()) }
        );
        assert_eq!(
            parse_import.parse("from ...one.two import three").unwrap(),
            VyperImport {
                level: 3,
                path: Some("one.two".to_string()),
                final_part: Some("three".to_string())
            }
        );
    }
}
