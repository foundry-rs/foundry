---
cast: patch
---

Fixed a panic (`attempt to subtract with overflow`) in `cast creation-code --without-args`/`--only-args` when a user-supplied `--abi-path` ABI declares more constructor argument bytes than the deployed bytecode actually contains, returning an error instead.
