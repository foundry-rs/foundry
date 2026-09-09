//! Checks the canonical lint documentation against the registered lints and page template.

use super::{Severity, all_lints};
use crate::linter::Lint;
use eyre::{Result, ensure};
use std::{collections::BTreeSet, fs, path::Path};

#[test]
fn registered_lints_have_docs() {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let mut registered = BTreeSet::new();
    let mut errors = Vec::new();
    for lint in all_lints() {
        assert!(registered.insert(lint.id().to_owned()), "duplicate lint ID: {}", lint.id());
        let path = docs.join(format!("{}.md", lint.id()));
        let result = fs::read_to_string(&path)
            .map_err(eyre::Report::from)
            .and_then(|text| validate_doc(&text, lint.id(), lint.severity()));
        if let Err(error) = result {
            errors.push(format!("{}: {error}", path.display()));
        }
    }
    assert!(!registered.is_empty(), "no registered lints");
    let documented = fs::read_dir(&docs)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .map(|path| path.file_stem().unwrap().to_str().unwrap().to_owned())
        .filter(|id| !matches!(id.as_str(), "README" | "_template"))
        .collect::<BTreeSet<_>>();
    for id in documented.difference(&registered) {
        errors.push(format!("{id}.md: no registered lint"));
    }
    assert!(errors.is_empty(), "invalid lint documentation:\n{}", errors.join("\n"));
}

#[test]
fn registered_lints_have_canonical_help_url() {
    for lint in all_lints() {
        let expected = format!("https://getfoundry.sh/forge/linting/{}", lint.id());
        assert_eq!(lint.help(), expected, "lint `{}` has a non-canonical help URL", lint.id());
    }
}

#[derive(Debug, PartialEq)]
enum Kind<'a> {
    Heading(usize),
    Text,
    Code(&'a str),
}

#[derive(Debug)]
struct Token<'a> {
    kind: Kind<'a>,
    text: String,
    line: usize,
}

fn fence(line: &str) -> Option<(u8, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = *trimmed.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let len = trimmed.bytes().take_while(|&byte| byte == marker).count();
    (len >= 3).then(|| (marker, len, trimmed[len..].trim()))
}

// Fenced examples can contain headings, metadata, and shorter fences; ignore that content.
fn tokens(text: &str) -> Result<Vec<Token<'_>>> {
    let mut result = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((index, line)) = lines.next() {
        let (kind, text) = if let Some((marker, len, language)) = fence(line) {
            let mut code = String::new();
            let mut closed = false;
            for (_, line) in lines.by_ref() {
                if fence(line).is_some_and(|(end, width, tail)| {
                    end == marker && width >= len && tail.is_empty()
                }) {
                    closed = true;
                    break;
                }
                code.push_str(line);
                code.push('\n');
            }
            ensure!(closed, "line {}: unclosed code fence", index + 1);
            (Kind::Code(language), code)
        } else {
            let level = line.bytes().take_while(|&byte| byte == b'#').count();
            if (1..=6).contains(&level) && line[level..].starts_with(' ') {
                (Kind::Heading(level), line[level..].trim().to_owned())
            } else if line.trim().is_empty() {
                continue;
            } else {
                (Kind::Text, line.trim().to_owned())
            }
        };
        result.push(Token { kind, text, line: index + 1 });
    }
    Ok(result)
}

fn validate_doc(text: &str, id: &str, severity: Severity) -> Result<()> {
    ensure!(
        id.starts_with(|c: char| c.is_ascii_lowercase())
            && id.split('-').all(|part| !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())),
        "lint ID must be kebab-case"
    );
    let items = tokens(text)?;
    let [title, level, identity, body @ ..] = items.as_slice() else {
        eyre::bail!("expected title, severity, ID, and sections");
    };
    ensure!(title.kind == Kind::Heading(1) && !title.text.is_empty(), "start with one # title");
    ensure!(
        level.kind == Kind::Text && level.text == format!("**Severity**: `{severity:?}`"),
        "line {}: severity must match the registered lint ({severity:?})",
        level.line
    );
    ensure!(
        identity.kind == Kind::Text && identity.text == format!("**ID**: `{id}`"),
        "line {}: ID must match the registered lint and filename ({id})",
        identity.line
    );
    ensure!(
        body.first().is_some_and(|t| t.kind == Kind::Heading(2) && t.text == "What it does"),
        "start the body with ## What it does (no introductory summary)"
    );
    let mut core = Vec::new();
    let mut seen = BTreeSet::new();
    let mut remaining = body;
    while let Some((heading, tail)) = remaining.split_first() {
        let end = tail.iter().position(|t| matches!(t.kind, Kind::Heading(1 | 2)));
        let (content, rest) = tail.split_at(end.unwrap_or(tail.len()));
        remaining = rest;
        ensure!(heading.kind == Kind::Heading(2), "line {}: only one # title", heading.line);
        let name = heading.text.as_str();
        ensure!(seen.insert(name), "line {}: duplicate section {name}", heading.line);
        match name {
            "What it does" | "Why is this bad?" | "Why restrict this?" | "Example" => {
                core.push(name);
            }
            "Configuration" | "Notes" | "Limitations" | "Known limitations" => {}
            _ => eyre::bail!("line {}: unexpected section {name}", heading.line),
        }
        ensure!(
            content
                .iter()
                .any(|t| !matches!(t.kind, Kind::Heading(_)) && !t.text.trim().is_empty()),
            "line {}: {name} must not be empty",
            heading.line
        );
        for token in content {
            ensure!(
                !(matches!(token.kind, Kind::Heading(_))
                    && matches!(token.text.as_str(), "Bad" | "Good")),
                "line {}: use Use instead: rather than Bad/Good headings",
                token.line
            );
            ensure!(
                !(token.kind == Kind::Text
                    && (token.text.starts_with("**Severity**:")
                        || token.text.starts_with("**ID**:"))),
                "line {}: metadata belongs only below the title",
                token.line
            );
        }
        if matches!(name, "What it does" | "Why is this bad?" | "Why restrict this?") {
            ensure!(content.iter().any(|t| t.kind == Kind::Text), "{name} needs explanatory prose");
        }
        if name == "Example" {
            let separators = content
                .iter()
                .enumerate()
                .filter(|(_, t)| t.kind == Kind::Text && t.text == "Use instead:")
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            ensure!(separators.len() == 1, "Example needs exactly one Use instead: separator");
            let split = separators[0];
            for side in [&content[..split], &content[split + 1..]] {
                ensure!(
                    side.iter()
                        .any(|t| t.kind == Kind::Code("solidity") && !t.text.trim().is_empty()),
                    "Example needs a nonempty solidity code block on each side of Use instead:"
                );
            }
        }
    }
    ensure!(
        matches!(
            core.as_slice(),
            ["What it does", "Why is this bad?" | "Why restrict this?", "Example"]
        ),
        "expected What it does, exactly one Why section, then Example"
    );
    Ok(())
}

const VALID: &str = "# Example\n\n**Severity**: `Info`\n**ID**: `example`\n\n\
    ## What it does\n\nReports a pattern.\n\n## Why is this bad?\n\nExplains the consequence.\n\n\
    ## Example\n\n```solidity\nbad();\n```\n\nUse instead:\n\n```solidity\ngood();\n```\n";

#[test]
fn accepts_documentation_variants() {
    for severity in [
        Severity::High,
        Severity::Med,
        Severity::Low,
        Severity::Info,
        Severity::Gas,
        Severity::CodeSize,
    ] {
        validate_doc(&VALID.replace("`Info`", &format!("`{severity:?}`")), "example", severity)
            .unwrap();
    }
    for text in [
        VALID.replace("Why is this bad?", "Why restrict this?"),
        VALID.replace('\n', "\r\n"),
        VALID.replace("## Why", "## Known limitations\n\nAn exclusion.\n\n## Why")
            + "\n## Configuration\n\n```toml\nsetting = true\n```\n\n## Notes\n\nA note.\n\n## Limitations\n\nA limitation.\n",
        include_str!("../../docs/_template.md")
            .replace("`<High | Med | Low | Info | Gas | CodeSize>`", "`Info`")
            .replace("`<str_id>`", "`example`"),
    ] {
        validate_doc(&text, "example", Severity::Info).unwrap();
    }
    for marker in ["````", "~~~"] {
        let text = VALID
            .replace("```", marker)
            .replace("bad();", "## Example\n**ID**: `other`\nUse instead:\n### Bad\n```");
        validate_doc(&text, "example", Severity::Info).unwrap();
    }
}

#[test]
fn rejects_invalid_documentation() {
    for (from, to, error) in [
        ("# Example", "Example", "one # title"),
        ("**Severity**: `Info`", "", "severity"),
        ("`Info`", "`High`", "severity"),
        ("`example`", "`other`", "ID must match"),
        ("## What", "Summary.\n\n## What", "no introductory summary"),
        ("## Why is this bad?\n\nExplains the consequence.\n\n", "", "exactly one Why"),
        ("Reports a pattern.", "", "must not be empty"),
        ("Reports a pattern.", "```solidity\nf();\n```", "explanatory prose"),
        ("Use instead:", "", "exactly one Use instead:"),
        ("Use instead:", "### Bad", "Bad/Good headings"),
        ("Use instead:", "### Good", "Bad/Good headings"),
        ("bad();", "", "nonempty solidity"),
        ("good();", " ", "nonempty solidity"),
        ("```solidity", "```text", "nonempty solidity"),
        ("```solidity", "~~~solidity", "unclosed code fence"),
        ("```solidity", "````solidity", "unclosed code fence"),
    ] {
        let text = VALID.replacen(from, to, 1);
        let result = validate_doc(&text, "example", Severity::Info).unwrap_err().to_string();
        assert!(result.contains(error), "{from:?} -> {to:?}: {result}");
    }
    for (suffix, error) in [
        ("# Extra\n", "only one # title"),
        ("## Example\n", "duplicate section"),
        ("## Why restrict this?\n\nPolicy.\n", "exactly one Why"),
        ("## Scope and controls\n\nText.\n", "unexpected section"),
        ("## Notes\n\n### Detail\n", "must not be empty"),
        ("**ID**: `example`\n", "metadata belongs only"),
        ("Use instead:\n", "exactly one Use instead:"),
    ] {
        let result = validate_doc(&(VALID.to_owned() + suffix), "example", Severity::Info)
            .unwrap_err()
            .to_string();
        assert!(result.contains(error), "{suffix:?}: {result}");
    }
    let reordered = VALID
        .replace("## Why is this bad?", "## Temporary")
        .replace("## Example", "## Why is this bad?")
        .replace("## Temporary", "## Example");
    assert!(validate_doc(&reordered, "example", Severity::Info).is_err());
    assert!(
        validate_doc(VALID.trim_end().trim_end_matches("```"), "example", Severity::Info).is_err()
    );
    for id in ["Not_kebab", "example-", "example--lint", "1example"] {
        let text = VALID.replace("`example`", &format!("`{id}`"));
        assert!(validate_doc(&text, id, Severity::Info).is_err(), "{id}");
    }
}
