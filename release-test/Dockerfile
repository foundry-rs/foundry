# https://github.com/paritytech/foundry-polkadot/releases/

FROM ubuntu:24.04 AS builder

ARG VERSION=stable

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

RUN curl -L https://raw.githubusercontent.com/paritytech/foundry-polkadot/refs/heads/master/foundryup/install | bash && \
    /root/.foundry/bin/foundryup-polkadot --platform linux --arch amd64 --install ${VERSION}

FROM ubuntu:24.04

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /root/.foundry/bin/ /usr/local/bin/
