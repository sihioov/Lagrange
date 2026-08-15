# One-shot migration image. It has the SQLx CLI and tracked migrations, but
# no serving application or broker credentials. Run it as migration_owner and
# gate every serving worker on successful completion.

FROM rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900 AS builder

RUN apk add --no-cache build-base musl-dev pkgconf openssl-dev postgresql-dev
RUN cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features postgres,rustls

FROM postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a

COPY --from=builder /usr/local/cargo/bin/sqlx /usr/local/bin/sqlx
COPY migrations /opt/lagrange/migrations
COPY deploy/db/migrate.sh /usr/local/bin/lagrange-migrate
COPY deploy/db/bootstrap-roles.sh /usr/local/bin/lagrange-bootstrap-roles
RUN chmod 0755 /usr/local/bin/lagrange-migrate /usr/local/bin/lagrange-bootstrap-roles

USER 999:999
ENTRYPOINT ["/usr/local/bin/lagrange-migrate"]
