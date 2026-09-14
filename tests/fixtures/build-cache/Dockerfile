# syntax=docker/dockerfile:1.7

FROM rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900 AS builder

ARG TARGETPLATFORM
ARG CACHE_FIXTURE_COMMIT
ARG CACHE_FIXTURE_BUILD_SETTING=default
ENV CARGO_BUILD_JOBS=2
ENV CARGO_TARGET_DIR=/cargo-target
ENV CACHE_FIXTURE_COMMIT="$CACHE_FIXTURE_COMMIT"
ENV CACHE_FIXTURE_BUILD_SETTING="$CACHE_FIXTURE_BUILD_SETTING"
RUN test -n "$CACHE_FIXTURE_COMMIT" \
    && printf '%s' "$CACHE_FIXTURE_COMMIT" | grep -Eq '^[0-9a-f]{40}$'

WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY fixture-lib ./fixture-lib
COPY fixture-app ./fixture-app

ARG CACHE_FIXTURE_RUN_TOKEN=initial
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=build-cache-fixture-registry-v1-${TARGETPLATFORM},sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,id=build-cache-fixture-git-v1-${TARGETPLATFORM},sharing=locked \
    --mount=type=cache,target=/cargo-target,id=build-cache-fixture-target-v1-${TARGETPLATFORM},sharing=locked \
    cargo clean --workspace --release --locked \
    && test -n "$CACHE_FIXTURE_RUN_TOKEN" \
    && printf '%s\n' 'SMOKE_CARGO_PHASE bin=cache-bin-a' \
    && cargo build --locked --release --verbose --package build-cache-fixture-app --bin cache-bin-a \
    && printf '%s\n' 'SMOKE_CARGO_PHASE bin=cache-bin-b' \
    && cargo build --locked --release --verbose --package build-cache-fixture-app --bin cache-bin-b \
    && mkdir -p /build/target/release \
    && cp /cargo-target/release/cache-bin-a /build/target/release/cache-bin-a \
    && cp /cargo-target/release/cache-bin-b /build/target/release/cache-bin-b

FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d

RUN addgroup -S -g 10001 fixture \
    && adduser -S -D -H -u 10001 -G fixture fixture
WORKDIR /opt/fixture
COPY --from=builder /build/target/release/cache-bin-a /usr/local/bin/cache-bin-a
COPY --from=builder /build/target/release/cache-bin-b /usr/local/bin/cache-bin-b

USER 10001:10001
ENTRYPOINT ["/usr/local/bin/cache-bin-a"]
