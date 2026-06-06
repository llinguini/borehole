# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Stage 1: builder
# ---------------------------------------------------------------------------
# NOTE: requires Rust >= 1.85 (some transitive deps, e.g. `getrandom 0.4`, need
# the `edition2024` Cargo feature). Pinned to match the verified local
# toolchain; bump as needed.
FROM rust:1.95-alpine AS builder

# Build dependencies.
#   - musl-dev / build-base: C/C++ toolchain for native crates.
#   - pkgconfig, openssl-dev: pkg-config metadata and headers.
#   - cmake, perl: required to build `aws-lc-sys`, the native backend pulled in
#     by `rustls 0.23` (aws-lc-rs). Without these the release build fails.
RUN apk add --no-cache \
    musl-dev \
    build-base \
    pkgconfig \
    openssl-dev \
    cmake \
    perl

# Target the statically-linked musl libc so the binary can run on `scratch`.
RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /app

# Copy the full workspace (see .dockerignore for what is excluded).
COPY . .

# Build only the server binary in release mode for the musl target.
RUN cargo build --release --target x86_64-unknown-linux-musl -p server

# ---------------------------------------------------------------------------
# Stage 2: runtime
# ---------------------------------------------------------------------------
FROM scratch

# The statically-linked server binary.
COPY --from=builder \
    /app/target/x86_64-unknown-linux-musl/release/borehole-server /borehole-server

# System root certificates, required for TLS.
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

# Control plane port.
EXPOSE 7000
# Public tunnel port range.
EXPOSE 30000-40000

ENTRYPOINT ["/borehole-server", "/etc/borehole/config.json"]
