FROM docker.io/library/rust:1-slim-bookworm AS build
ARG CARGO_BUILD_JOBS=2
WORKDIR /app
# Dependencies first so they are cached until Cargo.toml/Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
 && cargo build --release --locked \
 && rm -rf src target/release/pibipu* target/release/deps/pibipu-*
COPY src ./src
RUN touch src/main.rs && cargo build --release --locked

# TLS roots are compiled in (rustls + webpki-roots), so the runtime needs nothing but glibc.
FROM docker.io/library/debian:bookworm-slim
WORKDIR /app
COPY --from=build /app/target/release/pibipu /usr/local/bin/pibipu
COPY config.json ./
CMD ["/usr/local/bin/pibipu"]
