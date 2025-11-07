# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS chef
WORKDIR /app

RUN apt update && apt install -y build-essential libssl-dev git pkg-config curl perl
RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | sh
RUN cargo binstall cargo-chef sccache

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
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    cargo chef cook --recipe-path recipe.json --profile ${RUST_PROFILE} --no-default-features --features "${RUST_FEATURES}"

ARG TAG_NAME="dev"
ENV TAG_NAME=$TAG_NAME
ARG VERGEN_GIT_SHA="ffffffffffffffffffffffffffffffffffffffff"

# Build the project.
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    cargo build --profile ${RUST_PROFILE} --no-default-features --features "${RUST_FEATURES}"

# `dev` profile outputs to the `target/debug` directory.
RUN ln -s /app/target/debug /app/target/dev \
    && mkdir -p /app/output \
    && mv \
    /app/target/${RUST_PROFILE}/forge \
    /app/target/${RUST_PROFILE}/cast \
    /app/target/${RUST_PROFILE}/anvil \
    /app/target/${RUST_PROFILE}/chisel \
    /app/output/

RUN sccache --show-stats || true

FROM ubuntu:22.04 AS runtime

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
