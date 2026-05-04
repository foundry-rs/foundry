//! Vocs site file generation.
//!
//! Generates `vocs.config.ts`, `pages/index.mdx`, `package.json`, and `.gitignore`
//! from the emitted MDX pages.

use foundry_config::DocConfig;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Write all vocs site scaffolding into `out_dir`.
///
/// `pages` is the list of relative MDX paths emitted by the render step (relative to
/// `out_dir/pages/`). `root` is the project root, used to find README files.
pub fn write_site_files(
    out_dir: &Path,
    config: &DocConfig,
    pages: &[PathBuf],
    root: &Path,
) -> eyre::Result<()> {
    fs::create_dir_all(out_dir)?;

    fs::write(out_dir.join(".gitignore"), "dist/\ncache/\nnode_modules/\n")?;
    fs::write(out_dir.join("package.json"), package_json())?;
    fs::write(out_dir.join("vocs.config.ts"), vocs_config(config, pages))?;

    // Homepage: config.homepage -> <sources>/README.md -> <root>/README.md -> empty.
    let homepage_content = find_homepage(config, root);
    let index_path = out_dir.join("pages").join("index.mdx");
    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(index_path, homepage_content)?;

    Ok(())
}

// ── vocs.config.ts ────────────────────────────────────────────────────────────

fn vocs_config(config: &DocConfig, pages: &[PathBuf]) -> String {
    let title = if config.title.is_empty() { "Documentation" } else { &config.title };

    let sidebar = build_sidebar(pages);

    let mut ts = String::new();
    ts.push_str("import { defineConfig } from 'vocs'\n\n");
    ts.push_str("export default defineConfig({\n");
    // The config sits next to `pages/` rather than at a project root with a
    // `docs/` subdirectory (vocs' default), so override `rootDir` accordingly.
    ts.push_str("  rootDir: '.',\n");
    ts.push_str(&format!("  title: {},\n", json_str(title)));

    if let Some(repo) = &config.repository {
        ts.push_str(&format!(
            "  editLink: {{ pattern: '{}/edit/main/{{path}}' }},\n",
            repo.trim_end_matches('/')
        ));
    }

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
        let link = format!("/{}", page.with_extension("").display());
        let stem = page.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");

        // Split `type.Name` -> (kind, display_name).
        let (kind, name) = stem.split_once('.').unwrap_or(("", stem));
        let cat = type_category_order(kind);

        let dir = page
            .parent()
            .and_then(|p| if p == Path::new("") { None } else { p.to_str() })
            .unwrap_or("")
            .to_string();

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

fn find_homepage(config: &DocConfig, root: &Path) -> String {
    // 1. Explicit homepage from config.
    if let Some(hp) = &config.homepage {
        let path = if hp.is_absolute() { hp.clone() } else { root.join(hp) };
        if let Ok(content) = fs::read_to_string(&path) {
            return content;
        }
    }

    // 2. <root>/README.md
    let readme = root.join("README.md");
    if let Ok(content) = fs::read_to_string(&readme) {
        return content;
    }

    // 3. Empty fallback.
    String::new()
}

// ── package.json ──────────────────────────────────────────────────────────────

const fn package_json() -> &'static str {
    r#"{
  "devDependencies": {
    "vocs": "^1"
  },
  "scripts": {
    "dev": "vocs dev",
    "build": "vocs build"
  }
}
"#
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn json_str(s: &str) -> String {
    // Escape double quotes and wrap.
    format!("'{}'", s.replace('\'', "\\'"))
}
