# TessariDB S3, as a container — the server only. Its metadata lives in a TessariDB
# node that runs beside it (see compose.yaml); nothing of the database is inside
# this image.
#
# Two stages: the builder carries the Rust toolchain, the image somebody runs
# carries one executable and what it needs at run time.

# ── build ───────────────────────────────────────────────────────────────────
# Debian 13 like the image it feeds, so the binary links the C library it runs on.
FROM rust:1.98-slim-trixie@sha256:4cd829461bd5c4d511c32e269da9cb8929223b666519d8004e35fc8d1d771ab7 AS build

WORKDIR /src
COPY . .

# `--locked`: the committed lock file is the dependency graph the tests ran
# against. One binary, `tessaridb-s3`; the console's web assets are committed
# build output, so no Node is needed here.
RUN cargo build --release --locked -p tessari-s3-node --bin tessaridb-s3

# The data directory, owned by the runtime image's `nonroot` user (65532): the
# image below has no shell to create it.
RUN mkdir -p /out/var/lib/tessaridb-s3/data

# ── run ─────────────────────────────────────────────────────────────────────
# Distroless: the C runtime and CA certificates, no shell, no package manager.
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:e792ab3d241a468a4fd7519ddbbebe66b49b5f365771716ea688ad40b6c6f1c2

COPY --from=build --chown=65532:65532 /out/var/lib/tessaridb-s3 /var/lib/tessaridb-s3
COPY --from=build /src/target/release/tessaridb-s3 /usr/local/bin/tessaridb-s3
COPY LICENSE /usr/share/doc/tessaridb-s3/LICENSE

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

USER nonroot
WORKDIR /var/lib/tessaridb-s3

# `tessaridb-s3 health` sends an unsigned `GET /` to the local listener and
# passes on any HTTP status line — the server's own answer, after its request
# pipeline ran — so a server listening but wedged fails it.
HEALTHCHECK --interval=10s --timeout=5s --start-period=20s --retries=3 \
  CMD ["/usr/local/bin/tessaridb-s3", "health"]

# No init process: the server handles SIGTERM and SIGINT itself (draining
# in-flight requests for TESSARIDB_S3_SHUTDOWN_GRACE_SECS), and the only child
# it ever starts — ffmpeg, when TESSARIDB_S3_FFMPEG names one — it waits for or
# kills and reaps itself, so it is correct as PID 1. This image carries no ffmpeg.
ENTRYPOINT ["/usr/local/bin/tessaridb-s3"]
