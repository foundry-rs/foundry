# Heavily inspired by:
# - Lighthouse: https://github.com/sigp/lighthouse/blob/693886b94176faa4cb450f024696cb69cda2fe58/Makefile
# - Reth: https://github.com/paradigmxyz/reth/blob/1f642353ca083b374851ab355b5d80207b36445c/Makefile
.DEFAULT_GOAL := help

# Cargo profile for builds.
PROFILE ?= dev

# The docker image name
DOCKER_IMAGE_NAME ?= ghcr.io/foundry-rs/foundry:latest

BIN_DIR = dist/bin
CARGO_TARGET_DIR ?= target

# List of features to use when building. Can be overridden via the environment.
# No jemalloc on Windows
ifeq ($(OS),Windows_NT)
    FEATURES ?= aws-kms gcp-kms turnkey cli asm-keccak
else
    FEATURES ?= jemalloc aws-kms gcp-kms turnkey cli asm-keccak
endif

##@ Help

.PHONY: help
help: ## Display this help.
	@awk 'BEGIN {FS = ":.*##"; printf "Usage:\n  make \033[36m<target>\033[0m\n"} /^[a-zA-Z_0-9-]+:.*?##/ { printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2 } /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } ' $(MAKEFILE_LIST)

##@ Build

.PHONY: build
build: ## Build the project.
	cargo build --features "$(FEATURES)" --profile "$(PROFILE)"

.PHONY: build-docker
build-docker: ## Build the docker image.
	docker build . -t "$(DOCKER_IMAGE_NAME)" \
	--build-arg "RUST_PROFILE=$(PROFILE)" \
	--build-arg "RUST_FEATURES=$(FEATURES)" \
	--build-arg "TAG_NAME=dev" \
	--build-arg "VERGEN_GIT_SHA=$(shell git rev-parse HEAD)"

##@ Test

## Run unit/doc tests and generate html coverage report in `target/llvm-cov/html` folder.
## Notice that `llvm-cov` supports doc tests only in nightly builds because the `--doc` flag
## is unstable (https://github.com/taiki-e/cargo-llvm-cov/issues/2).
.PHONY: test-coverage
test-coverage:
	cargo +nightly llvm-cov --no-report nextest -E 'kind(test) & !test(/\b(issue|ext_integration)/)' && \
	cargo +nightly llvm-cov --no-report --doc && \
	cargo +nightly llvm-cov report --doctests --open

.PHONY: test-unit
test-unit: ## Run unit tests.
	cargo nextest run -E 'kind(test) & !test(/\b(issue|ext_integration)/)'

.PHONY: test-doc
test-doc: ## Run doc tests.
	cargo test --doc --workspace

.PHONY: test
test: ## Run all tests.
	make test-unit && \
	make test-doc

##@ Linting

.PHONY: fmt
fmt: ## Run all formatters.
	cargo +nightly fmt
	./.github/scripts/format.sh --check

.PHONY: lint-clippy
lint-clippy: ## Run clippy on the codebase.
	cargo +nightly clippy \
	--workspace \
	--all-targets \
	--all-features \
	-- -D warnings

.PHONY: lint-typos
lint-typos: ## Run typos on the codebase.
	@command -v typos >/dev/null || { \
		echo "typos not found. Please install it by running the command `cargo install typos-cli` or refer to the following link for more information: https://github.com/crate-ci/typos"; \
		exit 1; \
	}
	typos

.PHONY: lint
lint: ## Run all linters.
	make fmt && \
	make lint-clippy && \
	make lint-typos

##@ Other

.PHONY: clean
clean: ## Clean the project.
	cargo clean

.PHONY: deny
deny: ## Perform a `cargo` deny check.
	cargo deny --all-features check all

.PHONY: pr
pr: ## Run all checks and tests.
	make deny && \
	make lint && \
	make test

# dprint formatting commands
.PHONY: dprint-fmt
dprint-fmt: ## Format code with dprint
	@if ! command -v dprint > /dev/null; then \
		echo "Installing dprint..."; \
		cargo install dprint; \
	fi
	dprint fmt

.PHONY: dprint-check
dprint-check: ## Check formatting with dprint
	@if ! command -v dprint > /dev/null; then \
		echo "Installing dprint..."; \
		cargo install dprint; \
	fi
	dprint check
