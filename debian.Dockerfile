# syntax=docker/dockerfile:1.4
# based on @dboreham's docker file (https://github.com/dboreham/foundry/blob/cerc-release/Dockerfile-debian)
# discussion in https://github.com/foundry-rs/foundry/issues/2358

ARG TARGETARCH
ARG DEBIAN_VERSION=bullseye-20230502

ARG DEBIAN_FRONTEND=noninteractive

FROM debian:$DEBIAN_VERSION as build-environment

SHELL ["/bin/bash", "-c"]

WORKDIR /opt

RUN apt-get update \
    && apt-get install -y clang lld curl build-essential \
    && rm -rf /var/lib/apt/lists/* \
    && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs > rustup.sh \
    && chmod +x ./rustup.sh \
    && ./rustup.sh -y

# Works around an arm-specific rust bug, see: https://github.com/cross-rs/cross/issues/598
RUN set -e; [[ "$TARGETARCH" = "arm64" || $(uname -m) = "aarch64" ]] && echo "export CFLAGS=-mno-outline-atomics" >> $HOME/.profile || true

WORKDIR /opt/foundry

COPY . .

RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/opt/foundry/target \
    source $HOME/.profile && cargo build --release \
    && mkdir out \
    && mv target/release/forge out/forge \
    && mv target/release/cast out/cast \
    && mv target/release/anvil out/anvil \
    && strip out/forge \
    && strip out/cast \
    && strip out/anvil;

FROM debian:$DEBIAN_VERSION-slim as foundry-client

RUN apt-get update \
    && apt-get -y install --no-install-recommends git

COPY --from=build-environment /opt/foundry/out/forge /usr/local/bin/forge
COPY --from=build-environment /opt/foundry/out/cast /usr/local/bin/cast
COPY --from=build-environment /opt/foundry/out/anvil /usr/local/bin/anvil

RUN adduser -Du 1000 foundry

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
