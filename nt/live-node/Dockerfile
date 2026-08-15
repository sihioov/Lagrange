# Credential-free deterministic Phase-3 live-node runtime. No KIS client or
# credentials are contacted by this image; a non-dry-run request fails before
# credential read.

FROM python@sha256:fd95fa221297a88e1cf49c55ec1828edd7c5a428187e67b5d1805692d11588db

RUN groupadd --system --gid 10001 lagrange \
    && useradd --system --uid 10001 --gid lagrange --home-dir /opt/lagrange --shell /usr/sbin/nologin lagrange

WORKDIR /opt/lagrange
COPY nt/live-node/live_node ./live-node/live_node
COPY nt/live-node/runtime/live-node-entrypoint /usr/local/bin/live-node

RUN chmod 0755 /usr/local/bin/live-node \
    && chown -R lagrange:lagrange /opt/lagrange /usr/local/bin/live-node

ENV PYTHONUNBUFFERED=1
ENV PYTHONDONTWRITEBYTECODE=1
ENV PYTHONPATH=/opt/lagrange/live-node
ENV LIVE_NODE_MODE=disabled
ENV LIVE_NODE_DRY_RUN=1
USER 10001:10001
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/live-node", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/live-node"]
