#!/bin/bash
# Is this server answering S3?
#
# There is no unauthenticated health route on the S3 surface, so this asks the
# one question any S3 server answers: an unsigned `GET /`. The answer is a 403,
# and that is the point — a status line comes from the server itself, after its
# request pipeline ran, rather than from the kernel's accept queue. A server that
# is listening but wedged sends nothing and fails the check.
set -euo pipefail

port="${TESSARIDB_S3_LISTEN##*:}"
exec 3<>"/dev/tcp/127.0.0.1/${port}"
printf 'GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n' >&3
status="$(head -n 1 <&3)"
case "${status}" in
  HTTP/1.1\ [1-5][0-9][0-9]\ *) exit 0 ;;
  *) echo "tessaridb-s3: GET / answered ${status:-nothing}" >&2 ; exit 1 ;;
esac
