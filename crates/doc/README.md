# forge-doc

Solidity documentation generator powered by [`solar`](https://github.com/paradigmxyz/solar).

## Overview

`forge-doc` walks the solar HIR (High-level Intermediate Representation) and emits
[vocs](https://vocs.dev/docs)-flavoured MDX pages suitable for publishing as a documentation site.

### Key components

- **`DocBuilder`**: entry point, configures source roots, output directory, and optional
  Git/deployment metadata, then calls `build()` to generate the docs.
- **`render`**: AST → MDX conversion with HIR lookups. Handles contracts, functions, structs,
  enums, errors, events, and UDVTs, including `@inheritdoc` resolution and inline `{Link}`
  rewriting. Depends on HIR for inheritance and cross-reference resolution.
- **`hir_ext`**: HIR-aware helpers including name-to-page mapping, inheritance links, inheritdoc
  resolution, and inline link replacement.
- **`utils`**: small utility functions, `git_source_url` and `read_deployments`.
- **`vocs`**: vocs site scaffolding, generates `vocs.config.ts`, `vocs.sidebar.ts`
  (always regenerated, imported by `vocs.config.ts`), `package.json`, `.gitignore`, and
  `src/pages/index.mdx`.
