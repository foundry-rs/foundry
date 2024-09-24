# Heavily inspired by Lighthouse: https://github.com/sigp/lighthouse/blob/693886b94176faa4cb450f024696cb69cda2fe58/Makefile
.DEFAULT_GOAL := help

# Cargo profile for builds. Default is for local builds, CI uses an override.
PROFILE ?= release

# List of features to use when building. Can be overridden via the environment.
# No jemalloc on Windows
ifeq ($(OS),Windows_NT)
    FEATURES ?= rustls aws-kms cli asm-keccak
else
    FEATURES ?= jemalloc rustls aws-kms cli asm-keccak
endif

##@ Help

.PHONY: help
help: ## Display this help.
	@awk 'BEGIN {FS = ":.*##"; printf "Usage:\n  make \033[36m<target>\033[0m\n"} /^[a-zA-Z_0-9-]+:.*?##/ { printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2 } /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } ' $(MAKEFILE_LIST)

##@ Build

.PHONY: build
build: ## Perform a `cargo` build
	cargo build --features "$(FEATURES)" --profile "$(PROFILE)"

##@ Other

.PHONY: clean
clean:
	cargo clean

fmt: ## Run the formatter
	cargo +nightly fmt

lint-foundry:
	cargo clippy --workspace --all-targets --all-features

lint-codespell: ensure-codespell
	codespell --skip "*.json"

ensure-codespell:
	@if ! command -v codespell &> /dev/null; then \
		echo "codespell not found. Please install it by running the command `pip install codespell` or refer to the following link for more information: https://github.com/codespell-project/codespell" \
		exit 1; \
    fi

lint: ## Run all linters
	make fmt && \
	make lint-foundry && \
	make lint-codespell

test-doc:
	cargo test --doc --workspace

test: ## Run all tests
	make test && \
	make test-doc

pr: ## Run all tests and linters for a PR
	make lint && \
	make test