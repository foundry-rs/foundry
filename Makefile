.PHONY: format lint test ready

format: 
	cargo +nightly fmt

lint: 
	cargo +nightly clippy --all-features -- -D warnings

test:
	cargo check
	cargo test
	cargo doc --open

ready: format lint test
