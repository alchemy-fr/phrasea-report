# The Rust toolchain, with the components the development needs (bin/cargo.sh).
FROM rust:1-slim-trixie AS rust-dev
RUN rustup component add rustfmt clippy

FROM rust:1-slim-trixie AS build

WORKDIR /src

# The dependencies first, in a layer of their own: they change far less often
# than the sources.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY src ./src
RUN touch src/main.rs \
    && cargo build --release --locked

FROM debian:trixie-slim

WORKDIR /srv/app

COPY --from=build /src/target/release/phrasea-report /usr/local/bin/phrasea-report

# Port 80 without root: the binary carries CAP_NET_BIND_SERVICE. The working
# directory holds failed_logs.jsonl.
RUN apt-get update \
    && apt-get install -y --no-install-recommends libcap2-bin \
    && setcap cap_net_bind_service=+ep /usr/local/bin/phrasea-report \
    && apt-get purge -y --auto-remove libcap2-bin \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --create-home app \
    && chown app:app /srv/app

USER app

EXPOSE 80

CMD ["phrasea-report"]
