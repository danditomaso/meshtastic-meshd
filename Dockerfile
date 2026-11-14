# meshd: discovery helper for the Meshtastic web client. Linux hosts only; see README.md.

FROM rust:1.100-slim-trixie AS build

# btleplug talks to BlueZ through libdbus on Linux.
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libdbus-1-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Cache dependencies separately from source.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY src ./src
RUN touch src/main.rs && cargo build --release --locked

FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends libdbus-1-3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --no-create-home meshd

COPY --from=build /src/target/release/meshd /usr/local/bin/meshd

USER meshd
EXPOSE 4405
ENTRYPOINT ["meshd"]
