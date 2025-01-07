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
build: ## Build the project.
	cargo build --features "$(FEATURES)" --profile "$(PROFILE)"

# The following commands use `cross` to build a cross-compile.
#
# These commands require that:
#
# - `cross` is installed (`cargo install cross`).
# - Docker is running.
# - The current user is in the `docker` group.
#
# The resulting binaries will be created in the `target/` directory.
build-%:
	cross build --target $* --features "$(FEATURES)" --profile "$(PROFILE)"

.PHONY: docker-build-push
docker-build-push: docker-build-prepare ## Build and push a cross-arch Docker image tagged with DOCKER_IMAGE_NAME.
	$(MAKE) build-x86_64-unknown-linux-gnu
	mkdir -p $(BIN_DIR)/amd64
	for bin in anvil cast chisel forge; do \
		cp $(CARGO_TARGET_DIR)/x86_64-unknown-linux-gnu/$(PROFILE)/$$bin $(BIN_DIR)/amd64/; \
	done

	$(MAKE) build-aarch64-unknown-linux-gnu
	mkdir -p $(BIN_DIR)/arm64
	for bin in anvil cast chisel forge; do \
		cp $(CARGO_TARGET_DIR)/aarch64-unknown-linux-gnu/$(PROFILE)/$$bin $(BIN_DIR)/arm64/; \
	done

	docker buildx build --file ./Dockerfile.cross . \
		--platform linux/amd64,linux/arm64 \
		$(foreach tag,$(shell echo $(DOCKER_IMAGE_NAME) | tr ',' ' '),--tag $(tag)) \
		--provenance=false \
		--push

.PHONY: docker-build-prepare
docker-build-prepare: ## Prepare the Docker build environment.
	docker run --privileged --rm tonistiigi/binfmt --install amd64,arm64
	@if ! docker buildx inspect cross-builder &> /dev/null; then \
		echo "Creating a new buildx builder instance"; \
		docker buildx create --use --driver docker-container --name cross-builder; \
	else \
		echo "Using existing buildx builder instance"; \
		docker buildx use cross-builder; \
	fi

##@ Other

.PHONY: clean
clean: ## Clean the project.
	cargo clean

## Linting

fmt: ## Run all formatters.
	cargo +nightly fmt
	./.github/scripts/format.sh --check

lint-foundry:
	RUSTFLAGS="-Dwarnings" cargo clippy --workspace --all-targets --all-features

lint-codespell: ensure-codespell
	codespell --skip "*.json"

ensure-codespell:
	@if ! command -v codespell &> /dev/null; then \
		echo "codespell not found. Please install it by running the command `pip install codespell` or refer to the following link for more information: https://github.com/codespell-project/codespell" \
		exit 1; \
    fi

lint: ## Run all linters.
	make fmt && \
	make lint-foundry && \
	make lint-codespell

## Testing

test-foundry:
	cargo nextest run -E 'kind(test) & !test(/issue|forge_std|ext_integration/)'

test-doc:
	cargo test --doc --workspace

test: ## Run all tests.
	make test-foundry && \
	make test-doc

pr: ## Run all tests and linters in preparation for a PR.
	make lint && \
	make test
