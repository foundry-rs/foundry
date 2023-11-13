FROM lukemathwalker/cargo-chef:latest-rust-1.72.1 as chef
WORKDIR /opt

FROM chef as planner

# Get the foundry project
RUN git clone https://github.com/foundry-rs/foundry.git

WORKDIR /opt/foundry

# Compute a lock-like file for our project
RUN cargo chef prepare  --recipe-path recipe.json

FROM chef as builder

WORKDIR /opt/foundry

# Get the foundry project
COPY --from=planner /opt/foundry /opt/foundry
# Get the lock-like file
COPY --from=planner /opt/foundry/recipe.json recipe.json

RUN apt-get update -y && apt-get install -y gcc-aarch64-linux-gnu

# Build our project dependencies, not our application!
RUN cargo chef cook --release --recipe-path recipe.json
# Up to this point, if our dependency tree stays the same,
# all layers should be cached.

# Conditional for cross compliation
RUN CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc CFLAGS=-mno-outline-atomics cargo build --release

# Strip any debug symbols
RUN strip /opt/foundry/target/release/forge \
    && strip /opt/foundry/target/release/cast \
    && strip /opt/foundry/target/release/anvil \
    && strip /opt/foundry/target/release/chisel

FROM debian:bookworm-slim AS foundry

# Foundry tools
COPY --from=builder /opt/foundry/target/release/forge /usr/local/bin/forge
COPY --from=builder /opt/foundry/target/release/cast /usr/local/bin/cast
COPY --from=builder /opt/foundry/target/release/anvil /usr/local/bin/anvil
COPY --from=builder /opt/foundry/target/release/chisel /usr/local/bin/chisel

RUN useradd -u 1000 -m foundry

USER foundry

# TODO(User and group here)

ENTRYPOINT ["/bin/sh", "-c"]