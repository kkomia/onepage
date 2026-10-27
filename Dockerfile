# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
WORKDIR /build

COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && printf 'fn main() {}\n' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY build.rs ./build.rs
COPY src ./src
COPY templates ./templates
COPY assets ./assets
COPY migrations ./migrations
# COPY may preserve older mtimes; force a rebuild of the application crate.
RUN touch build.rs src/main.rs \
    && cargo build --release --locked

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 onepage \
    && useradd --uid 10001 --gid onepage --no-create-home --shell /usr/sbin/nologin onepage \
    && mkdir -p /data \
    && chown onepage:onepage /data

COPY --from=builder /build/target/release/onepage /app/onepage
COPY LICENSE /usr/share/doc/onepage/LICENSE
COPY assets/fonts/licenses /usr/share/doc/onepage/font-licenses

WORKDIR /app
ENV ONEPAGE_BIND=0.0.0.0:8686 \
    ONEPAGE_DATA_DIR=/data \
    ONEPAGE_LOG=info

USER onepage
EXPOSE 8686
VOLUME ["/data"]
STOPSIGNAL SIGTERM

ENTRYPOINT ["/app/onepage"]
