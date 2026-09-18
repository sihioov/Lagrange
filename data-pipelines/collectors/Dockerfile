ARG RUST_ARTIFACT_SOURCE=source-builder
FROM rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900 AS source-builder

ARG TARGETPLATFORM
ENV CARGO_BUILD_JOBS=2
ENV CARGO_TARGET_DIR=/cargo-target

RUN apk add --no-cache build-base musl-dev pkgconf openssl-dev postgresql-dev
WORKDIR /build

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY data-pipelines/collectors ./data-pipelines/collectors
COPY apps/api-server/auth ./apps/api-server/auth
COPY configs/evidence/kis-range-canonical-approved-manifests.json ./configs/evidence/kis-range-canonical-approved-manifests.json
COPY configs/evidence/kis-historical-price-only-beta-approved-artifacts.json ./configs/evidence/kis-historical-price-only-beta-approved-artifacts.json
COPY configs/evidence/kis-historical-price-only-v3-approved-artifacts.json ./configs/evidence/kis-historical-price-only-v3-approved-artifacts.json
COPY configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json ./configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json
COPY configs/universes/kr-stock-price-beta-v1.json ./configs/universes/kr-stock-price-beta-v1.json
COPY configs/universes/kr-etf-core-v1.yaml ./configs/universes/kr-etf-core-v1.yaml
COPY data/calendars/xkrx/calendar.json ./data/calendars/xkrx/calendar.json
COPY data/calendars/xkrx/manifest.json ./data/calendars/xkrx/manifest.json
COPY data/calendars/xkrx/overrides.json ./data/calendars/xkrx/overrides.json
COPY tests/integration/migration-contract ./tests/integration/migration-contract
COPY tests/fixtures/kr-etf/contract ./tests/fixtures/kr-etf/contract
COPY tests/fixtures/kr-candidates/contract ./tests/fixtures/kr-candidates/contract

# The range/raw, action-range/raw, and ordinary worker binaries must be tied to
# the exact source revision that produced the image. Compose supplies this
# immutable CI argument; do not permit an unpinned or mutable-runtime-only image.
ARG LAGRANGE_CODE_COMMIT
RUN test -n "$LAGRANGE_CODE_COMMIT" \
    && printf '%s' "$LAGRANGE_CODE_COMMIT" | grep -Eq '^[0-9a-f]{40}$'

RUN --mount=type=cache,target=/usr/local/cargo/registry,id=lagrange-cargo-registry-v1,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,id=lagrange-cargo-git-v1,sharing=locked \
    --mount=type=cache,target=/cargo-target,id=lagrange-cargo-target-v1-rust-1.97.1-alpine-3c38f3f82c2f-${TARGETPLATFORM}-release,sharing=locked \
    cargo clean --workspace --release --locked \
    && cargo build --locked --release --package collectors --bin research-worker \
    && cargo build --locked --release --package collectors --bin kis-historical-price-beta-artifact \
    && cargo build --locked --release --package collectors --bin kis-historical-price-v3-artifact \
    && cargo build --locked --release --package collectors --bin kis-historical-price-beta-approval-check \
    && cargo build --locked --release --package collectors --bin kis-action-range-raw \
    && cargo build --locked --release --package collectors --bin kis-stock-price-beta-raw \
    && cargo build --locked --release --package collectors --bin kis-stock-price-beta-materialize \
    && cargo build --locked --release --package collectors --bin kis-historical-price-v3-input-check \
    && cargo build --locked --release --package collectors --bin owner-equity-v2-check \
    && cargo build --locked --release --package collectors --bin owner-equity-v2-materialize \
    && mkdir -p /build/target/release \
    && cp /cargo-target/release/research-worker /build/target/release/research-worker \
    && cp /cargo-target/release/kis-historical-price-beta-artifact /build/target/release/kis-historical-price-beta-artifact \
    && cp /cargo-target/release/kis-historical-price-v3-artifact /build/target/release/kis-historical-price-v3-artifact \
    && cp /cargo-target/release/kis-historical-price-beta-approval-check /build/target/release/kis-historical-price-beta-approval-check \
    && cp /cargo-target/release/kis-action-range-raw /build/target/release/kis-action-range-raw \
    && cp /cargo-target/release/kis-stock-price-beta-raw /build/target/release/kis-stock-price-beta-raw \
    && cp /cargo-target/release/kis-stock-price-beta-materialize /build/target/release/kis-stock-price-beta-materialize \
    && cp /cargo-target/release/kis-historical-price-v3-input-check /build/target/release/kis-historical-price-v3-input-check \
    && cp /cargo-target/release/owner-equity-v2-check /build/target/release/owner-equity-v2-check \
    && cp /cargo-target/release/owner-equity-v2-materialize /build/target/release/owner-equity-v2-materialize

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

ARG LAGRANGE_CODE_COMMIT
LABEL org.opencontainers.image.revision="$LAGRANGE_CODE_COMMIT"
ENV LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT"

RUN apk add --no-cache ca-certificates libgcc libpq libssl3 \
    && addgroup -S -g 10001 lagrange \
    && adduser -S -D -H -u 10001 -G lagrange lagrange \
    && mkdir -p /opt/lagrange/configs/data-rights

WORKDIR /opt/lagrange
COPY --from=builder /build/target/release/research-worker /usr/local/bin/research-worker
COPY --from=builder /build/target/release/kis-historical-price-beta-artifact /usr/local/bin/kis-historical-price-beta-artifact
COPY --from=builder /build/target/release/kis-historical-price-v3-artifact /usr/local/bin/kis-historical-price-v3-artifact
COPY --from=builder /build/target/release/kis-historical-price-beta-approval-check /usr/local/bin/kis-historical-price-beta-approval-check
COPY --from=builder /build/target/release/kis-action-range-raw /usr/local/bin/kis-action-range-raw
COPY --from=builder /build/target/release/kis-stock-price-beta-raw /usr/local/bin/kis-stock-price-beta-raw
COPY --from=builder /build/target/release/kis-stock-price-beta-materialize /usr/local/bin/kis-stock-price-beta-materialize
COPY --from=builder /build/target/release/kis-historical-price-v3-input-check /usr/local/bin/kis-historical-price-v3-input-check
COPY --from=builder /build/target/release/owner-equity-v2-check /usr/local/bin/owner-equity-v2-check
COPY --from=builder /build/target/release/owner-equity-v2-materialize /usr/local/bin/owner-equity-v2-materialize
COPY --from=builder /build/tests/fixtures/kr-etf/contract ./tests/fixtures/kr-etf/contract
COPY --from=builder /build/tests/fixtures/kr-candidates/contract ./tests/fixtures/kr-candidates/contract
COPY --from=builder /build/configs/universes/kr-stock-price-beta-v1.json ./configs/universes/kr-stock-price-beta-v1.json
COPY --from=builder /build/configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json ./configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json

USER 10001:10001
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/research-worker", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/research-worker"]
