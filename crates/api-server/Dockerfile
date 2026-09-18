# Lagrange Station API server — reproducible, non-root production image.
# Both base images use the same immutable pins as the other Rust services in
# this repository.  The build context is the workspace root (see Compose).
ARG RUST_ARTIFACT_SOURCE=source-builder
FROM rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900 AS source-builder

ARG TARGETPLATFORM
ENV CARGO_BUILD_JOBS=2
ENV CARGO_TARGET_DIR=/cargo-target

RUN apk add --no-cache build-base musl-dev pkgconf openssl-dev postgresql-dev
WORKDIR /build

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY configs/evidence/kis-range-canonical-approved-manifests.json ./configs/evidence/kis-range-canonical-approved-manifests.json
COPY configs/evidence/kis-historical-price-only-beta-approved-artifacts.json ./configs/evidence/kis-historical-price-only-beta-approved-artifacts.json
COPY configs/evidence/kis-historical-price-only-v3-approved-artifacts.json ./configs/evidence/kis-historical-price-only-v3-approved-artifacts.json
COPY configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json ./configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json
COPY configs/universes ./configs/universes
COPY data/calendars/xkrx/calendar.json ./data/calendars/xkrx/calendar.json
COPY data/calendars/xkrx/manifest.json ./data/calendars/xkrx/manifest.json
COPY data/calendars/xkrx/overrides.json ./data/calendars/xkrx/overrides.json
# api-server's production auth router is a workspace package dependency;
# keep its manifest and source in the dependency build context.
COPY apps/api-server/auth ./apps/api-server/auth
COPY data-pipelines/collectors ./data-pipelines/collectors
COPY tests/integration/migration-contract ./tests/integration/migration-contract

RUN --mount=type=cache,target=/usr/local/cargo/registry,id=lagrange-cargo-registry-v1,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,id=lagrange-cargo-git-v1,sharing=locked \
    --mount=type=cache,target=/cargo-target,id=lagrange-cargo-target-v1-rust-1.97.1-alpine-3c38f3f82c2f-${TARGETPLATFORM}-release,sharing=locked \
    cargo clean --workspace --release --locked \
    && cargo build --locked --release --package api-server --bin api-server \
    && mkdir -p /build/target/release \
    && cp /cargo-target/release/api-server /build/target/release/api-server

FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d AS verified-artifacts

ARG RUST_ARTIFACT_BUNDLE_SHA256
ARG RUST_ARTIFACT_HELPER_SHA256
COPY --from=release_artifacts / /release-artifacts
RUN set -e \
    && test -n "$RUST_ARTIFACT_BUNDLE_SHA256" \
    && test -n "$RUST_ARTIFACT_HELPER_SHA256" \
    && test "$(sha256sum /release-artifacts/.release-build/release-build-layout.sh | awk '{print $1}')" = "$RUST_ARTIFACT_HELPER_SHA256" \
    && apk add --no-cache python3 \
    && RUST_ARTIFACT_BUNDLE_SHA256="$RUST_ARTIFACT_BUNDLE_SHA256" \
       RUST_ARTIFACT_HELPER_SHA256="$RUST_ARTIFACT_HELPER_SHA256" \
       RELEASE_BUILD_LAYOUT_EXECUTION=1 \
       /bin/sh -e /release-artifacts/.release-build/release-build-layout.sh \
         --guard-build /release-artifacts/.release-build/request.json /release-artifacts /build

ARG RUST_ARTIFACT_SOURCE
FROM ${RUST_ARTIFACT_SOURCE} AS builder

FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d

# The binary's provenance is part of the production image identity.  Compose
# supplies this as an immutable CI build argument; reject missing or shortened
# values before publishing an image rather than falling back to a mutable
# runtime environment variable.
ARG LAGRANGE_CODE_COMMIT
RUN test -n "$LAGRANGE_CODE_COMMIT" \
    && printf '%s' "$LAGRANGE_CODE_COMMIT" | grep -Eq '^[0-9a-f]{40}$'
LABEL org.opencontainers.image.revision="$LAGRANGE_CODE_COMMIT"
ENV LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT"

RUN apk add --no-cache ca-certificates libgcc libstdc++ libpq libssl3 \
    && addgroup -S -g 10001 lagrange \
    && adduser -S -D -H -u 10001 -G lagrange lagrange

WORKDIR /opt/lagrange
COPY --from=builder /build/target/release/api-server /usr/local/bin/api-server

USER 10001:10001
EXPOSE 8080
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=10s --timeout=5s --start-period=15s --retries=10 \
    CMD wget -q -O /dev/null http://127.0.0.1:8080/readyz || exit 1
ENTRYPOINT ["/usr/local/bin/api-server"]
