# syntax=docker/dockerfile:1.4
FROM alpine:3.15 AS build-environment
WORKDIR /opt

RUN apk update

WORKDIR /opt/foundry

RUN set -eux; \
	\
	apk add --no-cache --virtual .foundry-deps \
		ca-certificates \
		clang \
		lld \
        build-base \
        linux-headers \
        git \
        curl \
        findutils \
	; \
	\
	dpkgArch="$(dpkg --print-architecture | awk -F- '{ print $NF }')"; \
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs > rustup.sh; \
    chmod +x ./rustup.sh; \
    ./rustup.sh -y; \
    export CARGO_HOME=/opt/foundry/.cargo;

ENV CARGO_HOME=/opt/foundry/.cargo

# Copy seperate
COPY ./Cargo.toml /opt/foundry/Cargo.toml
COPY ./Cargo.lock /opt/foundry/Cargo.lock
COPY . .

RUN source $HOME/.profile && cargo build --release

# build for release
RUN rm -rf /opt/foundry/target/release/deps/
RUN rm -rf /opt/foundry/target/release/build/

# Strip binaries
RUN find /opt/foundry/target/release -type f -exec file {} + | awk -F: '/:.*ELF/{print $1}' | xargs strip --strip-all
RUN find /opt/foundry/target/release -type f | xargs file | grep 'ELF.*stripped' | cut -f1 -d: | xargs strip --strip-all

# Cleanup deps
RUN apk del --no-network .foundry-deps;

FROM alpine:3.15 AS foundry-client

# TODO: Explore using `docker.io/frolvlad/alpine-glibc:alpine-3.15_glibc-2.34`

ENV GLIBC_KEY=https://alpine-pkgs.sgerrand.com/sgerrand.rsa.pub
ENV GLIBC_KEY_FILE=/etc/apk/keys/sgerrand.rsa.pub
ENV GLIBC_RELEASE=https://github.com/sgerrand/alpine-pkg-glibc/releases/download/2.35-r0/glibc-2.35-r0.apk

RUN set -eux; \
	\
	apk add --no-cache --virtual .musl-deps \
        linux-headers \
        gcompat \
	; \
	\
    wget -q -O ${GLIBC_KEY_FILE} ${GLIBC_KEY}; \
    wget -O glibc.apk ${GLIBC_RELEASE}; \
    apk add glibc.apk --force; \
    apk del --no-network .musl-deps;

COPY --from=build-environment /opt/foundry/target/release/forge /usr/local/bin/forge
COPY --from=build-environment /opt/foundry/target/release/cast /usr/local/bin/cast
COPY --from=build-environment /opt/foundry/target/release/anvil /usr/local/bin/anvil

EXPOSE 8545/tcp
EXPOSE 8545/udp
EXPOSE 8180
EXPOSE 3001/tcp

STOPSIGNAL SIGQUIT

ENTRYPOINT ["/bin/sh", "-c"]


LABEL org.label-schema.build-date=$BUILD_DATE \
      org.label-schema.name="Foundry" \
      org.label-schema.description="Foundry Toolchain" \
      org.label-schema.url="https://getfoundry.sh" \
      org.label-schema.vcs-ref=$VCS_REF \
      org.label-schema.vcs-url="https://github.com/foundry-rs/foundry.git" \
      org.label-schema.vendor="Foundry-rs" \
      org.label-schema.version=$VERSION \
      org.label-schema.schema-version="1.0"
