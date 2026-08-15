# Standalone NT backtest worker image.
#
# The normal deployment path is Dockerfile.backtest-runner, where the Rust
# queue daemon starts this package as a child. This image is also useful for a
# one-shot operator replay and makes the package's isolation/readiness contract
# explicit for Compose or a batch scheduler.

FROM python@sha256:fd95fa221297a88e1cf49c55ec1828edd7c5a428187e67b5d1805692d11588db

RUN pip install --no-cache-dir uv==0.9.17 \
    && groupadd --system --gid 10001 lagrange \
    && useradd --system --uid 10001 --gid lagrange --home-dir /opt/lagrange --shell /usr/sbin/nologin lagrange

WORKDIR /opt/lagrange
COPY nt/pyproject.toml nt/uv.lock ./nt/
COPY nt/strategies ./nt/strategies
COPY nt/custom-data ./nt/custom-data
COPY nt/backtest-worker ./nt/backtest-worker
COPY nt/backtest-worker/runtime/backtest-worker-entrypoint /usr/local/bin/backtest-worker

RUN uv sync --project /opt/lagrange/nt --locked \
    && chmod 0755 /usr/local/bin/backtest-worker \
    && chown -R lagrange:lagrange /opt/lagrange /usr/local/bin/backtest-worker

ENV PYTHONUNBUFFERED=1
ENV PYTHONDONTWRITEBYTECODE=1
USER 10001:10001
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/usr/local/bin/backtest-worker", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/backtest-worker"]
