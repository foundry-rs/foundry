FROM rust:alpine as builder

ENV GLIBC_REPO=https://github.com/sgerrand/alpine-pkg-glibc
ENV GLIBC_VERSION=2.35-r0

RUN set -ex && apk update \
    && apk add --no-cache ca-certificates \
        curl bash git jq build-base linux-headers libstdc++ ca-certificates \
    && wget -q -O /etc/apk/keys/sgerrand.rsa.pub https://alpine-pkgs.sgerrand.com/sgerrand.rsa.pub; \
    for pkg in glibc-${GLIBC_VERSION} glibc-bin-${GLIBC_VERSION}; \
        do curl -sSL ${GLIBC_REPO}/releases/download/${GLIBC_VERSION}/${pkg}.apk -o /tmp/${pkg}.apk; done \
    && apk add --no-cache /tmp/*.apk \
    && rm /tmp/*.apk

WORKDIR /usr/src/foundry
COPY . .

ENV RUSTFLAGS="-C target-cpu=native"
RUN cargo build --release

RUN strip /usr/src/foundry/target/release/cast \
    && strip /usr/src/foundry/target/release/forge

FROM alpine:latest

COPY --from=builder /usr/src/foundry/target/release/cast /usr/local/bin/cast
COPY --from=builder /usr/src/foundry/target/release/forge /usr/local/bin/forge

ENTRYPOINT ["/bin/sh", "-c"]
