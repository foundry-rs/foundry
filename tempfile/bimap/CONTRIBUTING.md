# Contributing

Thank you for your interest in improving `bimap-rs`!

1. [How to contribute](#how-to-contribute)
    1. [Bug reports](#bug-reports)
    1. [Feature requests](#feature-requests)
    1. [Direct pull requests](#direct-pull-requests)
1. [Local development](#local-development)

## How to contribute

### Bug reports

`bimap-rs` tries to be well tested but mistakes can still slip through. If something isn't working
as expected, please open an issue to let me know! It helps to include the following information:

- `bimap-rs` version and enabled/disabled features
- Rust version (`rustc --version`)
- Environment (other dependencies, operating system, etc)
- Minimal example that illustrates the issue

### Feature requests

If there's a feature you'd like to see in `bimap-rs`, open an issue describing the feature in
as much detail as you can.

### Direct pull requests

For small and/or uncontroversial features and bug fixes, feel free to skip the issue process and
just submit a pull request.

## Local development

Building `bimap-rs` locally is as simple as cloning the repository.

```shell
$ # clone the repository
$ git clone https://github.com/billyrieger/bimap-rs.git
$ cd bimap-rs

$ # build the library
$ cargo build
```

The full test suite runs the library tests and documentation tests with different combinations of
feature flags.

```shell
$ # run the test suite
$ cargo test
$ cargo test --all-features
$ cargo test --no-default-features
```

Don't forget to format your code! You'll need the nightly version of `rustfmt`.

```shell
$ # install nightly rustfmt
$ rustup toolchain install nightly
$ rustup component add --toolchain nightly rustfmt

$ # format the repository
$ cargo +nightly fmt
```