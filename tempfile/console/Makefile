all: test

check:
	@cargo check --all-features

build:
	@cargo build --all-features

doc:
	@cargo doc --all-features

test:
	@echo "CARGO TESTS"
	@cargo test
	@cargo test --all-features
	@cargo test --no-default-features

check-minver:
	@echo "MINVER CHECK"
	@cargo minimal-versions check
	@cargo minimal-versions check --all-features
	@cargo minimal-versions check --no-default-features

format:
	@rustup component add rustfmt 2> /dev/null
	@cargo fmt --all

format-check:
	@rustup component add rustfmt 2> /dev/null
	@cargo fmt --all -- --check

lint:
	@rustup component add clippy 2> /dev/null
	@cargo clippy --examples --tests

msrv-lock:
	@cargo update -p proptest --precise=1.0.0
	@cargo update -p byteorder --precise=1.4.0

.PHONY: all doc build check test format format-check lint check-minver msrv-lock
