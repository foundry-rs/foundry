---
forge-doc: patch
---

Preserve `<` and `{` inside unambiguous fenced code blocks (` ``` ` / `~~~`) in NatSpec when rendering `forge doc` pages, so Solidity examples like `if (a < b) { revert Err(); }` reach the documentation verbatim instead of being escaped to entities. Prose, inline code spans, and table cells keep the existing MDX-hazard escaping.
