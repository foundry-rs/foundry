//! Vocs site file generation.
//!
//! Generates `vocs.config.ts`, `pages/index.mdx`, `package.json`, and `.gitignore`
//! from the emitted MDX pages.

use foundry_config::DocConfig;
use path_slash::PathExt;
use std::{
    fs,
    path::{Path, PathBuf},
};

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
) -> eyre::Result<()> {
    fs::create_dir_all(out_dir)?;

    fs::write(out_dir.join(".gitignore"), "dist\nnode_modules\n")?;
    fs::write(out_dir.join("package.json"), package_json())?;
    fs::write(out_dir.join("vocs.config.ts"), vocs_config(config, pages, branch))?;

    // Homepage: config.homepage -> <sources>/README.md -> <root>/README.md -> empty.
    let homepage_content = find_homepage(config, root, sources);
    let index_path = out_dir.join("src").join("pages").join("index.mdx");
    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(index_path, homepage_content)?;

    Ok(())
}

// ── vocs.config.ts ────────────────────────────────────────────────────────────

fn vocs_config(config: &DocConfig, pages: &[PathBuf], branch: Option<&str>) -> String {
    let title = if config.title.is_empty() { "Documentation" } else { &config.title };

    let sidebar = build_sidebar(pages);

    let mut ts = String::new();
    ts.push_str("import { defineConfig } from 'vocs/config'\n\n");
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

    ts.push_str("  sidebar: [\n");
    ts.push_str(&sidebar);
    ts.push_str("  ],\n");
    ts.push_str("})\n");
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

fn find_homepage(config: &DocConfig, root: &Path, sources: &Path) -> String {
    // 1. Explicit homepage from config.
    if let Some(hp) = &config.homepage {
        let path = if hp.is_absolute() { hp.clone() } else { root.join(hp) };
        if let Ok(content) = fs::read_to_string(&path) {
            return content;
        }
    }

    // 2. <sources>/README.md (e.g. `src/README.md`).
    let src_readme = if sources.is_absolute() {
        sources.join("README.md")
    } else {
        root.join(sources).join("README.md")
    };
    if let Ok(content) = fs::read_to_string(&src_readme) {
        return content;
    }

    // 3. <root>/README.md
    let readme = root.join("README.md");
    if let Ok(content) = fs::read_to_string(&readme) {
        return content;
    }

    // 4. Empty fallback.
    String::new()
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
    "waku": "^1.0.0-alpha.4"
  }
}
"#
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn json_str(s: &str) -> String {
    // Escape double quotes and wrap.
    format!("'{}'", s.replace('\'', "\\'"))
}
