# syntax=docker/dockerfile:1

ARG FOUNDRY_BUILD_STAGE=builder-online

FROM rust:1-bookworm@sha256:13c186980fa33cc12759b429662a1322939dbe697484b7c33b47dd2698d28460 AS build-base
WORKDIR /app

RUN apt update && apt install -y build-essential libssl-dev git pkg-config curl perl

ARG RUST_PROFILE
ARG RUST_FEATURES
ARG TARGETARCH

ENV CARGO_INCREMENTAL=0

ARG TAG_NAME="dev"
ENV TAG_NAME=$TAG_NAME
ARG VERGEN_GIT_SHA="ffffffffffffffffffffffffffffffffffffffff"
ENV VERGEN_GIT_SHA=$VERGEN_GIT_SHA

# Build the project.
COPY . .

FROM build-base AS builder-online
RUN cargo build --locked --profile ${RUST_PROFILE} --no-default-features --features "${RUST_FEATURES}"

FROM build-base AS builder-approved
RUN --network=none set -eux; \
    test -d .approved-dependencies/cargo-home/vendor; \
    case "$TARGETARCH" in \
      amd64) export SVM_TARGET_PLATFORM="linux-amd64" ;; \
      arm64) export SVM_TARGET_PLATFORM="linux-aarch64" ;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    export SVM_RELEASES_LIST_JSON="../../../solc/${SVM_TARGET_PLATFORM}.json"; \
    cargo build --frozen --profile ${RUST_PROFILE} --no-default-features --features "${RUST_FEATURES}"

FROM ${FOUNDRY_BUILD_STAGE} AS builder

# `dev` profile outputs to the `target/debug` directory.
RUN ln -s /app/target/debug /app/target/dev \
    && mkdir -p /app/output \
    && mv \
    /app/target/${RUST_PROFILE}/forge \
    /app/target/${RUST_PROFILE}/cast \
    /app/target/${RUST_PROFILE}/anvil \
    /app/target/${RUST_PROFILE}/chisel \
    /app/target/${RUST_PROFILE}/solar \
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
