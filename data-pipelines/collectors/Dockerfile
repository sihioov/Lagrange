FROM rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900 AS builder

RUN apk add --no-cache build-base musl-dev pkgconf openssl-dev postgresql-dev
WORKDIR /build

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY data-pipelines/collectors ./data-pipelines/collectors
COPY apps/api-server/auth ./apps/api-server/auth
COPY tests/integration/migration-contract ./tests/integration/migration-contract
COPY tests/fixtures/kr-etf/contract ./tests/fixtures/kr-etf/contract

RUN cargo build --locked --release --package collectors --bin research-worker

FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d

RUN apk add --no-cache ca-certificates libgcc libpq libssl3 \
    && addgroup -S -g 10001 lagrange \
    && adduser -S -D -H -u 10001 -G lagrange lagrange

WORKDIR /opt/lagrange
COPY --from=builder /build/target/release/research-worker /usr/local/bin/research-worker
COPY --from=builder /build/tests/fixtures/kr-etf/contract ./tests/fixtures/kr-etf/contract

USER 10001:10001
ENTRYPOINT ["/usr/local/bin/research-worker"]
