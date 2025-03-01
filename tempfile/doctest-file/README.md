Procedural macro that sources doctests from dedicated files into Rustdoc documentation with support
for hiding lines.

Example usage:
```rust,text
/// ```
#[doc = doctest_file::include_doctest!("examples/foo.rs")]
/// ```
```

Unlike `include_str!()`, `include_doctest!()` allows you to hide certain lines, just as you can with
in-band doctests using the [`#` syntax]. Instead of prefixing every line which is to be hidden with
`#`, though, you can instead suffix them with `//`. An empty end-of-line comment does not make an
`.rs` file malformed, so you can receive rich editor support while writing doctests and put them in
the `examples/` directory. Note that whitespace is ignored during detection of this suffix.

Under the hood, `//` suffixes are simply translated to `#` prefixes, which are then interpreted by
Rustdoc.

Blocks of lines can also be hidden – this helps work around Rustfmt's inflexible formatting choices.
`//{` starts a hidden block, which only includes the line it is on if there is nothing on the line
other than that marker (aside from whitespace). `//}` ends a hidden block whilst unconditionally
hiding the line it's on.

Another feature that comes in handy when storing doctests in separate files is auto-dedent: the
macro will reduce indentation of visible lines such that the least indented visible line appears
completely unindented. This means that you don't need to bend the indentation rules that are
enforced by Rustfmt to get your doctest looking right in Rustdoc.

Additionally, blank lines at the end of the file are removed, also helping interoperability with
Rustfmt.

[`#` syntax]: https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html#hiding-portions-of-the-example

This crate has zero dependencies: no `proc_macro2`, no `syn`, no `quote`. It is also incredibly
simple, at just over 200 lines of code.

# License
`doctest-file` is licensed under the 0-clause BSD license, meaning that it is impossible to violate
the licensing terms. This is functionally equivalent to the public domain, but is also legal in
countries such as Germany.
