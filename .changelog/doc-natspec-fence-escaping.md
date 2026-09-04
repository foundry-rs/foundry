---
forge-doc: patch
---

Fix `forge doc` corrupting fenced code examples in NatSpec: `<` and bare `{` inside a `@dev`/`@notice` fenced or inline code span were being escaped to HTML entities, rendering literally instead of as the original character.
