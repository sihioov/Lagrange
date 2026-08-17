FROM rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900 AS builder

RUN apk add --no-cache build-base musl-dev pkgconf openssl-dev postgresql-dev
WORKDIR /build

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY nt ./nt
COPY configs/universes ./configs/universes
COPY data-pipelines/collectors ./data-pipelines/collectors
COPY apps/api-server/auth ./apps/api-server/auth
COPY tests/integration/migration-contract ./tests/integration/migration-contract

RUN cargo build --locked --release --package job-queue --bin recommendation-runner \
    && cargo build --locked --release --package job-queue --bin candidate-runner \
    && cargo build --locked --release --package job-queue --bin backtest-runner

FROM python@sha256:fd95fa221297a88e1cf49c55ec1828edd7c5a428187e67b5d1805692d11588db

RUN pip install --no-cache-dir uv==0.9.17 \
    && groupadd --system --gid 10001 lagrange \
    && useradd --system --uid 10001 --gid lagrange --home-dir /opt/lagrange --shell /usr/sbin/nologin lagrange

WORKDIR /opt/lagrange
COPY --from=builder /build/target/release/recommendation-runner /usr/local/bin/recommendation-runner
COPY --from=builder /build/target/release/candidate-runner /usr/local/bin/candidate-runner
COPY --from=builder /build/target/release/backtest-runner /usr/local/bin/backtest-runner
COPY --from=builder /build/nt ./nt
RUN uv sync --project /opt/lagrange/nt --locked \
    && chown -R lagrange:lagrange /opt/lagrange

USER 10001:10001
HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/recommendation-runner", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/recommendation-runner"]
