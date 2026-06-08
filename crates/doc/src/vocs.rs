//! Vocs site file generation.
//!
//! Generates `vocs.config.ts`, `pages/index.mdx`, `package.json`, and `.gitignore`
//! from the emitted MDX pages.

use crate::utils::{git_raw_url, git_source_url};
use foundry_config::DocConfig;
use path_slash::PathExt;
use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
};

/// Map from a Solidity source file location to its vocs page URL.
///
/// Key is `(parent_dir_with_forward_slashes, file_stem)` of the source file.
/// Value is the root-relative vocs URL (no extension, leading `/`).
type SourceToUrl = HashMap<(String, String), String>;

/// Write all vocs site scaffolding into `out_dir`.
///
/// `pages` is the list of relative MDX paths emitted by the render step (relative to
/// `out_dir/pages/`). `root` is the project root and `sources` is the Solidity
/// sources directory; both are searched for a README to use as the homepage.
pub fn write_site_files(
    out_dir: &Path,
    config: &DocConfig,
    pages: &[PathBuf],
    root: &Path,
    sources: &Path,
    branch: Option<&str>,
    commit: Option<&str>,
) -> eyre::Result<()> {
    fs::create_dir_all(out_dir)?;

    // Write user-editable scaffold files only on first run so that manual
    // customisations (tweaked vocs config, added npm deps, custom landing page)
    // are not silently overwritten on subsequent `forge doc` runs.
    write_if_absent(&out_dir.join(".gitignore"), "dist\nnode_modules\n")?;
    write_if_absent(&out_dir.join("package.json"), package_json())?;
    // The sidebar changes every time pages are added/removed so it lives in its own
    // file that is always regenerated.  vocs.config.ts imports it via a relative
    // import, so users can freely edit the main config without losing changes on
    // the next `forge doc` run.
    fs::write(out_dir.join("vocs.sidebar.ts"), vocs_sidebar(pages))?;
    write_if_absent(&out_dir.join("vocs.config.ts"), &vocs_config(config, branch))?;

    // Homepage: config.homepage -> <sources>/README.md -> <root>/README.md -> empty.
    // Rewrite relative links: `.sol` -> generated vocs page; everything else ->
    // a `{repo}/blob/{commit}/...` URL when a repository is configured.
    let (homepage_content, homepage_dir) = find_homepage(config, root, sources);
    let src_to_url = build_source_to_url(pages);
    let homepage_content = if let Some(base_dir) = homepage_dir.as_deref() {
        rewrite_homepage_links(
            &homepage_content,
            base_dir,
            root,
            &src_to_url,
            config.repository.as_deref(),
            commit,
        )
    } else {
        homepage_content
    };
    let homepage_content = escape_mdx_outside_code_fences(&homepage_content);
    let index_path = out_dir.join("src").join("pages").join("index.mdx");
    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Always regenerate: unlike user-editable scaffold files, index.mdx is
    // derived from README and must reflect the latest source on every run.
    fs::write(&index_path, &homepage_content)?;

    Ok(())
}

/// Write `content` to `path` only if the file does not already exist.
fn write_if_absent(path: &Path, content: &str) -> eyre::Result<()> {
    if !path.exists() {
        fs::write(path, content)?;
    }
    Ok(())
}

// ── vocs.config.ts ────────────────────────────────────────────────────────────

/// Generate the user-editable `vocs.config.ts`.
///
/// The sidebar is imported from the always-regenerated `vocs.sidebar.ts` so
/// that users can customise this file freely without losing changes on re-runs.
fn vocs_config(config: &DocConfig, branch: Option<&str>) -> String {
    let title = if config.title.is_empty() { "Documentation" } else { &config.title };

    let mut ts = String::new();
    ts.push_str("import { defineConfig } from 'vocs/config'\n");
    ts.push_str("import { sidebar } from './vocs.sidebar'\n\n");
    ts.push_str("export default defineConfig({\n");
    ts.push_str(&format!("  title: {},\n", json_str(title)));

    if let Some(repo) = &config.repository {
        // GitHub `edit/<branch>/<path>` requires a real branch name (it does not
        // accept `HEAD`). Use the detected current branch when available, else
        // fall back to `main`.
        let edit_branch = branch.unwrap_or("main");
        ts.push_str(&format!(
            "  editLink: {{ pattern: '{}/edit/{edit_branch}/{{path}}' }},\n",
            repo.trim_end_matches('/')
        ));
    }

    // Pin the shiki language bundle. Vocs otherwise scans every MDX page and
    // eagerly loads any code-fence language it finds, which fails for fences
    // like ```ml (OCaml, used by some READMEs as an ASCII tree). With an
    // explicit `langs` list, unknown fences fall back to `plaintext` instead
    // of crashing the highlighter at startup.
    ts.push_str("  codeHighlight: {\n");
    ts.push_str("    fallbackLanguage: 'plaintext',\n");
    ts.push_str("    langs: [\n");
    ts.push_str(
        "      'ansi', 'bash', 'diff', 'html', 'js', 'json', 'jsx',\n      \
         'markdown', 'md', 'mdx', 'plaintext', 'rust', 'sol', 'solidity',\n      \
         'toml', 'ts', 'tsx', 'yaml', 'zsh',\n",
    );
    ts.push_str("    ],\n");
    ts.push_str("  },\n");

    ts.push_str("  sidebar,\n");
    ts.push_str("})\n");
    ts
}

fn vocs_sidebar(pages: &[PathBuf]) -> String {
    let sidebar = build_sidebar(pages);
    let mut ts = String::new();
    ts.push_str("// This file is generated by forge doc. Do not edit manually.\n");
    ts.push_str("// Re-run `forge doc` to update.\n\n");
    ts.push_str("export const sidebar = [\n");
    ts.push_str(&sidebar);
    ts.push_str("]\n");
    ts
}

/// Build a TypeScript sidebar array string from the list of relative page paths.
fn build_sidebar(pages: &[PathBuf]) -> String {
    // Sort pages for deterministic output.
    let mut sorted = pages.to_vec();
    sorted.sort();

    // Group by parent directory -> type category -> (name, link).
    // BTreeMap keeps dirs and type categories in sorted/canonical order.
    let mut groups: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<u8, Vec<(String, String)>>,
    > = Default::default();

    for page in &sorted {
        // Always emit forward-slash URLs so links work on Windows.
        let link = format!("/{}", page.with_extension("").to_slash_lossy());
        let stem = page.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");

        // Split `type.Name` -> (kind, display_name).
        let (kind, name) = stem.split_once('.').unwrap_or(("", stem));
        let cat = type_category_order(kind);

        let dir = page
            .parent()
            .and_then(|p| if p == Path::new("") { None } else { Some(p.to_slash_lossy()) })
            .map(|s| s.to_string())
            .unwrap_or_default();

        groups.entry(dir).or_default().entry(cat).or_default().push((name.to_string(), link));
    }

    let mut out = String::new();
    for (dir, by_type) in &groups {
        if dir.is_empty() {
            // Top-level items, emit flat, preserving type prefix in name for clarity.
            for items in by_type.values() {
                for (name, link) in items {
                    out.push_str(&format!(
                        "    {{ text: {}, link: {} }},\n",
                        json_str(name),
                        json_str(link)
                    ));
                }
            }
            continue;
        }

        // Does this directory have more than one type category?
        let multi_type = by_type.len() > 1;

        out.push_str(&format!("    {{\n      text: {},\n      items: [\n", json_str(dir)));

        for (cat, items) in by_type {
            if multi_type {
                // Emit a collapsed sub-group for this type category.
                let cat_label = type_category_label(*cat);
                out.push_str(&format!(
                    "        {{\n          text: {},\n          collapsed: true,\n          items: [\n",
                    json_str(cat_label)
                ));
                for (name, link) in items {
                    out.push_str(&format!(
                        "            {{ text: {}, link: {} }},\n",
                        json_str(name),
                        json_str(link)
                    ));
                }
                out.push_str("          ],\n        },\n");
            } else {
                // Single type, list items directly without a wrapper.
                for (name, link) in items {
                    out.push_str(&format!(
                        "        {{ text: {}, link: {} }},\n",
                        json_str(name),
                        json_str(link)
                    ));
                }
            }
        }

        out.push_str("      ],\n    },\n");
    }
    out
}

/// Canonical sort order for type categories in the sidebar.
fn type_category_order(kind: &str) -> u8 {
    match kind {
        "contract" => 0,
        "abstract" => 1,
        "interface" => 2,
        "library" => 3,
        "struct" => 4,
        "enum" => 5,
        "type" => 6,
        "error" => 7,
        "event" => 8,
        "function" => 9,
        "constants" => 10,
        _ => 11,
    }
}

/// Human-readable label for a type category.
const fn type_category_label(cat: u8) -> &'static str {
    match cat {
        0 => "Contracts",
        1 => "Abstract Contracts",
        2 => "Interfaces",
        3 => "Libraries",
        4 => "Structs",
        5 => "Enums",
        6 => "Types",
        7 => "Errors",
        8 => "Events",
        9 => "Functions",
        10 => "Constants",
        _ => "Other",
    }
}

// ── homepage ──────────────────────────────────────────────────────────────────

/// Locate the homepage markdown.
///
/// Returns `(content, base_dir)` where `base_dir` is the absolute directory the
/// homepage file lives in (used to resolve its relative links). When no
/// homepage file is found, returns an empty string and no base dir.
fn find_homepage(config: &DocConfig, root: &Path, sources: &Path) -> (String, Option<PathBuf>) {
    // 1. Explicit homepage from config.
    if let Some(hp) = &config.homepage {
        let path = if hp.is_absolute() { hp.clone() } else { root.join(hp) };
        if let Ok(content) = fs::read_to_string(&path) {
            return (content, path.parent().map(Path::to_path_buf));
        }
    }

    // 2. <sources>/README.md (e.g. `src/README.md`).
    let src_readme = if sources.is_absolute() {
        sources.join("README.md")
    } else {
        root.join(sources).join("README.md")
    };
    if let Ok(content) = fs::read_to_string(&src_readme) {
        return (content, src_readme.parent().map(Path::to_path_buf));
    }

    // 3. <root>/README.md
    let readme = root.join("README.md");
    if let Ok(content) = fs::read_to_string(&readme) {
        return (content, readme.parent().map(Path::to_path_buf));
    }

    // 4. Empty fallback.
    (String::new(), None)
}

// ── homepage link rewriting ───────────────────────────────────────────────────

/// Build a `(parent_dir, file_stem) -> vocs_url` map from the emitted MDX pages.
///
/// Each page is named `<type>.<Name>.mdx`; the key mirrors the source `.sol`
/// file (parent dir + item name), so a README link to `path/to/Name.sol`
/// matches the page for the item whose name equals the file stem.
fn build_source_to_url(pages: &[PathBuf]) -> SourceToUrl {
    let mut map = SourceToUrl::new();
    for page in pages {
        let stem = page.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let Some((_kind, name)) = stem.split_once('.') else { continue };
        let dir = page.parent().map(|p| p.to_slash_lossy().into_owned()).unwrap_or_default();
        let url = format!("/{}", page.with_extension("").to_slash_lossy());
        map.insert((dir, name.to_string()), url);
    }
    map
}

/// Escape MDX-sensitive characters (`{` and `<`) in plain-text regions of a
/// Markdown document, leaving fenced code blocks (` ``` ` or `~~~`) untouched.
///
/// Without this, README content with template placeholders like `{FOO}` or
/// HTML-like tokens like `<TOKEN>` would be interpreted as MDX expressions/JSX
/// and break `vocs dev` / `vocs build`.
fn escape_mdx_outside_code_fences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    let mut fence_marker = "";
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if in_fence {
            out.push_str(line);
            if trimmed.starts_with(fence_marker) {
                in_fence = false;
            }
        } else if trimmed.starts_with("```") {
            in_fence = true;
            fence_marker = "```";
            out.push_str(line);
        } else if trimmed.starts_with("~~~") {
            in_fence = true;
            fence_marker = "~~~";
            out.push_str(line);
        } else {
            // Escape `{` and bare `<` (not already `&lt;` or a known entity).
            let mut inline_code_ticks = 0usize;
            let mut pending_ticks = 0usize;
            for ch in line.chars() {
                if ch == '`' {
                    pending_ticks += 1;
                    out.push(ch);
                    continue;
                }

                if pending_ticks > 0 {
                    if inline_code_ticks == 0 {
                        inline_code_ticks = pending_ticks;
                    } else if inline_code_ticks == pending_ticks {
                        inline_code_ticks = 0;
                    }
                    pending_ticks = 0;
                }

                if inline_code_ticks > 0 {
                    out.push(ch);
                } else {
                    match ch {
                        '{' => out.push_str(r"\{"),
                        '<' => out.push_str("&lt;"),
                        c => out.push(c),
                    }
                }
            }
        }
    }
    out
}

/// Rewrite inline `[text](url)` markdown links in the homepage:
/// * `.sol` paths that resolve to a known page → vocs URL.
/// * Any other relative path under `root` → `{repo}/blob/{commit}/...`.
/// * Absolute URLs, anchors, and unresolved targets are left untouched.
fn rewrite_homepage_links(
    text: &str,
    base_dir: &Path,
    root: &Path,
    src_to_url: &SourceToUrl,
    repo: Option<&str>,
    commit: Option<&str>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("](") {
        out.push_str(&rest[..open + 2]);
        rest = &rest[open + 2..];
        // Scan the URL, counting parens so we don't split on `(` / `)` inside it.
        let bytes = rest.as_bytes();
        let mut i = 0;
        let mut depth = 1usize;
        let mut closed = false;
        while i < bytes.len() {
            match bytes[i] {
                b'(' => {
                    depth += 1;
                    i += 1;
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                b'\\' => {
                    i += 2; // skip escaped character
                }
                _ => {
                    i += 1;
                }
            }
        }
        let target = &rest[..i];
        match try_rewrite_target(target, base_dir, root, src_to_url, repo, commit) {
            Some(new) => out.push_str(&new),
            None => out.push_str(target),
        }
        if closed {
            out.push(')');
            rest = &rest[i + 1..];
        } else {
            rest = &rest[i..];
            break;
        }
    }
    out.push_str(rest);
    out
}

/// Resolve `target` against `base_dir` and return either the matching vocs
/// page URL (`.sol`) or a `{repo}/blob/{commit}/...` URL for any other relative
/// path under `root`.
fn try_rewrite_target(
    target: &str,
    base_dir: &Path,
    root: &Path,
    src_to_url: &SourceToUrl,
    repo: Option<&str>,
    commit: Option<&str>,
) -> Option<String> {
    // Skip absolute URLs and pure anchors.
    if target.is_empty()
        || target.starts_with('#')
        || target.starts_with("//")
        || target.contains("://")
        || target.starts_with("mailto:")
    {
        return None;
    }

    // Split off `#fragment` / `?query`; we'll preserve the fragment on the rewrite.
    let (path_part, suffix) = target.find(['#', '?']).map_or((target, ""), |i| target.split_at(i));
    if path_part.is_empty() {
        return None;
    }

    let path = Path::new(path_part);
    // A leading `/` in a README link means "relative to project root", not an
    // OS-absolute filesystem path. Resolve against `root` so `/src/Foo.sol`
    // becomes `<root>/src/Foo.sol` rather than failing to strip the root prefix.
    let abs = if path.is_absolute() {
        let without_root_prefix = path.strip_prefix("/").unwrap_or(path);
        normalize_path(&root.join(without_root_prefix))
    } else {
        normalize_path(&base_dir.join(path))
    };
    let rel = abs.strip_prefix(root).ok()?.to_path_buf();

    // `.sol` -> vocs page.
    if path.extension().and_then(|e| e.to_str()) == Some("sol") {
        let stem = rel.file_stem().and_then(|s| s.to_str())?;
        let dir = rel.parent().map(|p| p.to_slash_lossy().into_owned()).unwrap_or_default();
        if let Some(url) = src_to_url.get(&(dir, stem.to_string())) {
            return Some(url.clone());
        }
    }

    // Fall back to a repo URL for everything else under the project root.
    // Use raw (download) URLs for image assets so they render inline rather
    // than pointing at the GitHub blob viewer page.
    let repo = repo?;
    let is_image = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico"
            )
        })
        .unwrap_or(false);
    let mut url = if is_image {
        git_raw_url(repo, commit.unwrap_or("HEAD"), root, &abs)?
    } else {
        git_source_url(repo, commit.unwrap_or("HEAD"), root, &abs)?
    };
    url.push_str(suffix);
    Some(url)
}

/// Lexically resolve `.` and `..` components without touching the filesystem.
fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

// ── package.json ──────────────────────────────────────────────────────────────

const fn package_json() -> &'static str {
    r#"{
  "scripts": {
    "dev": "vocs dev",
    "build": "vocs build",
    "preview": "vocs preview"
  },
  "dependencies": {
    "react": "^19",
    "react-dom": "^19",
    "vocs": "https://pkg.pr.new/wevm/vocs@next",
    "waku": "1.0.0-alpha.6"
  }
}
"#
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("serializing a string cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_links() {
        let map = build_source_to_url(&[
            PathBuf::from("src/contract.Morpho.mdx"),
            PathBuf::from("src/interfaces/interface.IMorpho.mdx"),
            PathBuf::from("src/libraries/library.MathLib.mdx"),
        ]);
        let root = Path::new("/repo");
        let repo = Some("https://github.com/x/y");
        let commit = Some("abc123");
        let input = "[Morpho](./src/Morpho.sol), \
                     [IMorpho](src/interfaces/IMorpho.sol#L10), \
                     [Unknown](src/Unknown.sol), \
                     [Contrib](./CONTRIBUTING.md), \
                     [Logo](./img/logo.png), \
                     [ext](https://x.com), \
                     [anchor](#section)";

        let out = rewrite_homepage_links(input, Path::new("/repo"), root, &map, repo, commit);
        // .sol with known page -> vocs URL.
        assert!(out.contains("[Morpho](/src/contract.Morpho)"));
        // .sol with fragment -> vocs URL (fragment dropped, no anchor in MDX).
        assert!(out.contains("[IMorpho](/src/interfaces/interface.IMorpho)"));
        // .sol without a known page -> repo blob URL.
        assert!(out.contains("[Unknown](https://github.com/x/y/blob/abc123/src/Unknown.sol)"));
        // Other relative paths -> repo blob URL.
        assert!(out.contains("[Contrib](https://github.com/x/y/blob/abc123/CONTRIBUTING.md)"));
        assert!(out.contains("[Logo](https://github.com/x/y/raw/abc123/img/logo.png)"));
        // Absolute URL and pure anchor untouched.
        assert!(out.contains("[ext](https://x.com)"));
        assert!(out.contains("[anchor](#section)"));
    }

    #[test]
    fn escape_mdx_leaves_code_fences_alone() {
        let input = "\
# Title

Plain text with {placeholder} and <TOKEN> here.
Inline code keeps `forge create <Contract>` and `{OWNER}` unchanged.

```solidity
contract Foo {
    mapping(address => uint256) public balances;
}
```

More text: {another} and <bar/>.

~~~shell
echo {not escaped}
~~~

End {brace}.
";
        let out = escape_mdx_outside_code_fences(input);

        // Plain-text regions: { → \{ and < → &lt;  (} and > are left alone).
        assert!(out.contains(r"\{placeholder}"), "{{ in plain text should be escaped");
        assert!(out.contains("&lt;TOKEN>"), "< in plain text should be escaped");
        assert!(out.contains(r"\{another}"));
        assert!(out.contains("&lt;bar/>"));
        assert!(out.contains(r"\{brace}"));
        assert!(out.contains("`forge create <Contract>`"), "inline code span must be unchanged");
        assert!(out.contains("`{OWNER}`"), "inline code span braces must be unchanged");

        // Inside ``` fences: untouched.
        assert!(
            out.contains("mapping(address => uint256)"),
            "code fence content must be unchanged"
        );
        assert!(!out.contains(r"mapping(address => uint256\)"), "no stray escaping inside fence");

        // Inside ~~~ fences: untouched.
        assert!(out.contains("echo {not escaped}"), "~~~ fence content must be unchanged");
    }

    #[test]
    fn json_str_emits_valid_typescript_strings() {
        assert_eq!(json_str(r#"Acme\"#), r#""Acme\\""#);
        assert_eq!(json_str("Bob's Docs"), r#""Bob's Docs""#);
        assert_eq!(json_str(r#"Quote " Docs"#), r#""Quote \" Docs""#);
    }
}
