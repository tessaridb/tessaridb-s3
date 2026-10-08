# TessariDB S3, as a container — the server only. Its metadata lives in a TessariDB
# node that runs beside it (see compose.yaml); nothing of the database is inside
# this image.
#
# Two stages: the builder carries the Rust toolchain, the image somebody runs
# carries one executable and what it needs at run time.

# ── build ───────────────────────────────────────────────────────────────────
FROM rust:1.98-slim-bookworm AS build

WORKDIR /src
COPY . .

# `--locked`: the committed lock file is the dependency graph the tests ran
# against. One binary, `tessaridb-s3`; the console's web assets are committed
# build output, so no Node is needed here.
RUN cargo build --release --locked -p tessari-s3-node --bin tessaridb-s3

# ── run ─────────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

# `bash` for the health check, which speaks HTTP over `/dev/tcp` instead of
# adding an HTTP client. The upgrade takes the base's security fixes published
# after it was built.
RUN apt-get update \
 && apt-get upgrade -y --no-install-recommends \
 && apt-get install -y --no-install-recommends bash \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /var/lib/tessaridb-s3 --shell /usr/sbin/nologin s3 \
 && mkdir -p /var/lib/tessaridb-s3/data \
 && chown -R s3:s3 /var/lib/tessaridb-s3

COPY --from=build /src/target/release/tessaridb-s3 /usr/local/bin/tessaridb-s3
COPY docker/healthcheck.sh /usr/local/bin/tessaridb-s3-healthcheck
COPY LICENSE /usr/share/doc/tessaridb-s3/LICENSE
RUN chmod 0755 /usr/local/bin/tessaridb-s3-healthcheck

# Defaults that make a container reachable: `0.0.0.0`, because a container's
# loopback answers nothing outside it. Everything else the server reads —
# above all the credentials (`TESSARIDB_S3_ROOT_ACCESS_KEY` / `_SECRET_KEY`,
# `TESSARIDB_S3_META_ADDRESS` / `_USER` / `_PASSWORD`, `TESSARIDB_S3_IAM_KEY`)
# — is deliberately NOT defaulted: an image carrying a secret hands it to
# everybody who pulls it. The server refuses to start without the required ones.
ENV TESSARIDB_S3_LISTEN=0.0.0.0:9100 \
    TESSARIDB_S3_DATA_DIR=/var/lib/tessaridb-s3/data

VOLUME ["/var/lib/tessaridb-s3"]
EXPOSE 9100/tcp 9101/tcp

USER s3
WORKDIR /var/lib/tessaridb-s3

HEALTHCHECK --interval=10s --timeout=5s --start-period=20s --retries=3 \
  CMD ["/usr/local/bin/tessaridb-s3-healthcheck"]

# No init process: the server handles SIGTERM and SIGINT itself (draining
# in-flight requests for TESSARIDB_S3_SHUTDOWN_GRACE_SECS) and forks nothing,
# so it is correct as PID 1.
ENTRYPOINT ["/usr/local/bin/tessaridb-s3"]
