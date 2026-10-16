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

# What the server reads. Two have a default here, because without them a
# container answers nothing: `0.0.0.0`, since a container's loopback is
# reachable from nothing outside it, and the data directory inside the volume.
#
#   TESSARIDB_S3_LISTEN           the S3 API's address
#   TESSARIDB_S3_DATA_DIR         where every object's bytes are stored
#
# Required, and deliberately NOT defaulted: an image carrying a secret hands it
# to everybody who pulls it. The server refuses to start without them.
#
#   TESSARIDB_S3_ROOT_ACCESS_KEY / _ROOT_SECRET_KEY  the root credential
#   TESSARIDB_S3_META_ADDRESS     the TessariDB nodes holding the metadata
#   TESSARIDB_S3_META_USER / _META_PASSWORD  the account the server signs in as
#
# Secrets that are optional, and likewise never defaulted:
#
#   TESSARIDB_S3_IAM_KEY          64 hex digits; without it only the root
#                                 credential signs in
#   TESSARIDB_S3_KEYS_PASSPHRASE  unseals the keys objects are encrypted under
#                                 (AES256 or aws:kms); unset, the keys stay
#                                 sealed until an operator unseals them in the
#                                 console, and encrypted reads and writes
#                                 answer 503
#   TESSARIDB_S3_CLUSTER_SECRET   placement key, the same on every member
#   TESSARIDB_S3_JOIN_TOKEN       one-time token from the console for this node
#   TESSARIDB_S3_CONSOLE_OIDC_CLIENT_SECRET  the console's client secret
#
# Everything else has the server's own default when unset:
#
#   TESSARIDB_S3_REGION, _DOMAINS, _ADVERTISE, _MAX_INFLIGHT,
#   _SHUTDOWN_GRACE_SECS
#   TESSARIDB_S3_META_NAMESPACE, _META_DATABASE, _META_CA, _META_REPLICATION,
#   _META_MAX_CONNECTIONS, _META_STATEMENT_TIMEOUT_SECS
#   TESSARIDB_S3_RECLAIM_GRACE_SECS, _RECLAIM_INTERVAL_SECS,
#   _UPLOAD_MAX_AGE_SECS, _USAGE_INTERVAL_SECS
#   TESSARIDB_S3_MEDIA_WORKERS, _FFMPEG (this image carries no ffmpeg)
#   TESSARIDB_S3_NOTIFY_TARGETS
#   a cluster member: TESSARIDB_S3_ERASURE, _NODE_ID, _ZONE, _INTERNAL_LISTEN,
#   _INTERNAL_ADVERTISE, _INTERNAL_TLS_CERT / _KEY / _CA, _FOUNDING_MEMBERS,
#   _HEAL_INTERVAL_SECS, _MOVE_BYTES_PER_SEC
#   the console: TESSARIDB_S3_CONSOLE_LISTEN (9101 below), _CONSOLE_TLS_CERT /
#   _KEY, _CONSOLE_SIGN_INS_PER_MINUTE, _CONSOLE_REQUESTS_PER_MINUTE,
#   _CONSOLE_KEY_SIGN_IN, _CONSOLE_BREAK_GLASS_WEBHOOK, _CONSOLE_OIDC_ISSUER,
#   _CONSOLE_OIDC_CLIENT_ID, _CONSOLE_OIDC_REDIRECT_URI, _CONSOLE_OIDC_MFA_AMR
#
# The README's configuration table states each one's default and meaning.
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
