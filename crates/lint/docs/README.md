# Forge lint documentation

This directory contains the canonical Markdown explanation for each registered `forge-lint` rule.
Include documentation changes in the same Foundry PR as the lint. The
[Foundry Book](https://github.com/foundry-rs/book)'s weekly update imports these files with
`import:lints` and generates the reference pages and navigation. The lint's `help` URL points to
the published page at `https://getfoundry.sh/forge/linting/<id>`.

## Adding a new lint

When you add a new lint with `declare_forge_lint!`, you **must** also add a documentation file at
`crates/lint/docs/<str_id>.md`. The Book's `import:lints -- --foundry <checkout>` command
validates the documentation and registered metadata, then generates pages and navigation.
Use `import:lints -- --check` in the Book to validate the committed import offline.

Use [`_template.md`](./_template.md) as a starting point.

Follow the [lint writing style guide](../../../docs/dev/lintrules.md#lint-writing-style) for naming,
diagnostics, and examples. This documentation format follows
[Clippy's lint documentation guidance](https://doc.rust-lang.org/clippy/development/adding_lints.html#documentation).

- `What it does` describes the conditions actually checked, including relevant exclusions.
- `Why is this bad?` explains the consequence. For style or policy choices that can reasonably be
  allowed, use `Why restrict this?` instead and explain the tradeoff. Choose by purpose, not severity.
- `Example` contains a short triggering Solidity example, then `Use instead:` and a non-triggering
  alternative. Preserve the intended behavior where possible and explain tradeoffs where it changes.
  State any required context, imports, compiler version, or lint configuration. If local suppression
  is the appropriate alternative, explain why the flagged code is acceptable before showing it.
- Use backticks for code in prose. Keep security, correctness, and gas claims specific; explain
  limitations instead of implying every match is a bug or every suggested change is always safe.
- Write for lint users, not lint implementers. Omit AST/HIR details, alias tracking, traversal
  rules, analysis budgets, diagnostic placement, and comparisons with other detectors. Keep
  caveats only when they help the reader interpret or address a warning; put implementation
  explanations in developer documentation or source comments.
- Start with `What it does`; do not repeat it in an introductory summary. Keep shared severity,
  file-exclusion, and suppression instructions in the linting guide. Omit generic review reminders
  and extra sections that repeat the problem or remedy.

## File structure

For a self-contained triggering example, put `{{produces}}` on its own line immediately after
the Solidity code block. The Book importer runs that block as `src/Example.sol` with `forge lint`
and replaces the marker with the actual diagnostics for this lint, including source spans and
help. An invalid example or a missing expected diagnostic fails the import. Use a Forge binary
matching the imported Foundry revision. Normal Book builds use the committed output, not Forge.

Keep fragments unmarked until they include the declarations they need. Do not add `//~ ERROR`
or `//~ WARN` test annotations to user-facing examples; the generated output shows the actual
severity and message. Lint behavior remains covered by Foundry's existing tests; documentation
validation runs in the Book at import time.

Each lint doc file should follow this structure:

````markdown
# <human-readable lint name>

**Severity**: `<High | Med | Low | Info | Gas | CodeSize>`
**ID**: `<str_id>`

## What it does

Explain precisely what the lint flags.

## Why is this bad?

Explain the impact (security, correctness, gas, readability).

## Example

```solidity
// triggering example
```

Use instead:

```solidity
// non-triggering, recommended example
```

## Configuration

Document any inline-config or `foundry.toml` options that affect this lint. Omit this section when
the lint has no additional configuration.
````
