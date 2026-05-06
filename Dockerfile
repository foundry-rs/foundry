# syntax=docker/dockerfile:1

FROM rust:1-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55 AS chef
WORKDIR /app

RUN apt update && apt install -y build-essential libssl-dev git pkg-config curl perl
RUN set -eux; \
    BINSTALL_VERSION="v1.18.1"; \
    case "$(dpkg --print-architecture)" in \
      amd64) ARCH="x86_64-unknown-linux-musl"; SHA256="cf2a4b54494ea8555d6349685e9a301efc1051d9fba6308c76914b2486f8700f" ;; \
      arm64) ARCH="aarch64-unknown-linux-musl"; SHA256="c55962a0115f9716b709216de7f8bdd59d6ba8738779e60b051b4593f677717a" ;; \
      *) echo "unsupported architecture" >&2; exit 1 ;; \
    esac; \
    curl -L --proto '=https' --tlsv1.2 -sSf \
      "https://github.com/cargo-bins/cargo-binstall/releases/download/${BINSTALL_VERSION}/cargo-binstall-${ARCH}.tgz" \
      -o /tmp/cargo-binstall.tgz; \
    echo "${SHA256}  /tmp/cargo-binstall.tgz" | sha256sum -c -; \
    tar -xzf /tmp/cargo-binstall.tgz -C /usr/local/cargo/bin cargo-binstall; \
    rm /tmp/cargo-binstall.tgz
RUN cargo binstall -y cargo-chef sccache

# Prepare the cargo-chef recipe.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Build the project.
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json

ARG RUST_PROFILE
ARG RUST_FEATURES

ENV CARGO_INCREMENTAL=0 \
    RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/sccache

# Build dependencies.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=shared \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=shared \
    --mount=type=cache,target=$SCCACHE_DIR,sharing=shared \
    cargo chef cook --recipe-path recipe.json --profile ${RUST_PROFILE} --no-default-features --features "${RUST_FEATURES}"

ARG TAG_NAME="dev"
ENV TAG_NAME=$TAG_NAME
ARG VERGEN_GIT_SHA="ffffffffffffffffffffffffffffffffffffffff"
ENV VERGEN_GIT_SHA=$VERGEN_GIT_SHA

# Build the project.
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=shared \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=shared \
    --mount=type=cache,target=$SCCACHE_DIR,sharing=shared \
    cargo build --profile ${RUST_PROFILE} --no-default-features --features "${RUST_FEATURES}" \
    && sccache --show-stats || true

# `dev` profile outputs to the `target/debug` directory.
RUN ln -s /app/target/debug /app/target/dev \
    && mkdir -p /app/output \
    && mv \
    /app/target/${RUST_PROFILE}/forge \
    /app/target/${RUST_PROFILE}/cast \
    /app/target/${RUST_PROFILE}/anvil \
    /app/target/${RUST_PROFILE}/chisel \
    /app/output/

FROM ubuntu:22.04@sha256:eb29ed27b0821dca09c2e28b39135e185fc1302036427d5f4d70a41ce8fd7659 AS runtime

# Install runtime dependencies.
RUN apt update && apt install -y git

COPY --from=builder /app/output/* /usr/local/bin/

RUN groupadd -g 1000 foundry && \
    useradd -m -u 1000 -g foundry foundry
USER foundry

ENTRYPOINT ["/bin/sh", "-c"]

LABEL org.label-schema.build-date=$BUILD_DATE \
      org.label-schema.name="Foundry" \
      org.label-schema.description="Foundry" \
      org.label-schema.url="https://getfoundry.sh" \
      org.label-schema.vcs-ref=$VCS_REF \
      org.label-schema.vcs-url="https://github.com/foundry-rs/foundry.git" \
      org.label-schema.vendor="Foundry-rs" \
      org.label-schema.version=$VERSION \
      org.label-schema.schema-version="1.0"
