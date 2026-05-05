# forge-doc

Solidity documentation generator powered by [`solar`](https://github.com/paradigmxyz/solar).

## Overview

`foundry-doc` walks the solar HIR (High-level Intermediate Representation) and emits
[vocs](https://vocs.dev/docs)-flavoured MDX pages suitable for publishing as a documentation site.

### Key components

- **`DocBuilder`**: entry point, configures source roots, output directory, and optional
  Git/deployment metadata, then calls `build()` to generate the docs.
- **`render`**: pure AST -> MDX conversion. Handles contracts, functions, structs, enums,
  errors, events, and UDVTs, including `@inheritdoc` resolution and inline `{Link}` rewriting.
- **`hir_ext`**: HIR-aware helpers including name-to-page mapping, inheritance links, inheritdoc
  resolution, and inline link replacement.
- **`utils`**: small utility functions, `git_source_url`
  and `read_deployments`.
- **`vocs`**: types and serialisation for the vocs `sidebar.json` and `config.ts` manifests.
