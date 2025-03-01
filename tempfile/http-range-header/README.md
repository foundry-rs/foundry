# Range header parsing

[![Latest workflow](https://github.com/MarcusGrass/parse-range-headers/workflows/check_commit/badge.svg)](https://github.com/MarcusGrass/parse-range-headers/actions)
[![CratesIo](https://shields.io/crates/v/http-range-header)](https://crates.io/crates/http-range-header)

The main goals of this parser is:
* Follow specification [RFC-2616](https://www.ietf.org/rfc/rfc2616.txt)
* Behave as expected [MDN](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Range)
* Accuracy - parses headers strictly
* Security - Never panics, ensured by fuzzing
* Stability
* No dependecies

Secondary goals are:
* Speed
* Information on why the header was rejected

The parser is strict. Any range where all parts are not syntactically correct and makes sense in the context of the underlying 
resource will be rejected.

## Dev release checklist

1. Make sure CI passes
2. Run [cargo fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html) 
`cargo +nightly fuzz run random_string_input`, at least a minute should be good enough. If it doesn't 
error out it has passed.
3. Check msrv with for example [cargo msrv](https://github.com/foresterre/cargo-msrv),
   if a higher msrv is wanted/needed, bump it so that it's less than or equal to [tower-http's](https://github.com/tower-rs/tower-http)
4. Update changelog
5. Update version
6. Publish