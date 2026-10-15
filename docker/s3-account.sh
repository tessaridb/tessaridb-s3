#!/bin/bash
# Declares the TessariDB S3 server's own database account, once, as the store's owner.
#
# The server signs in as an editor of its ONE namespace — it keeps the global database
# and its region's database there, and can reach nothing else — never as the owner. The
# owner declares the namespace (letting a bucket ask for its region's majority alone),
# the database and the account; every statement carries
# `IF NOT EXISTS`, so running this again changes nothing (a new password is set with
# `ALTER USER`, not by re-running it).
#
# The statement is written to a private file and run with `-f`, so the account's
# password never appears in a process's arguments. The owner's password reaches the
# client through `TESSARIDB_PASSWORD`, as the client requires.
set -euo pipefail

: "${TESSARIDB_AT:?}" "${TESSARIDB_OWNER:?}" "${TESSARIDB_PASSWORD:?}"
: "${S3_NAMESPACE:?}" "${S3_DATABASE:?}" "${S3_META_USER:?}" "${S3_META_PASSWORD:?}"

for name in "$S3_NAMESPACE" "$S3_DATABASE" "$S3_META_USER"; do
  [[ "$name" =~ ^[a-z][a-z0-9_]{0,62}$ ]] || { echo "not a name: $name" >&2; exit 1; }
done
# The password goes into the statement as text (0.33 takes no parameter there), so a
# quote or a backslash would end it early: refuse rather than escape.
[[ "$S3_META_PASSWORD" != *"'"* && "$S3_META_PASSWORD" != *'\'* ]] \
  || { echo "S3_META_PASSWORD may not contain a quote or a backslash" >&2; exit 1; }
(( ${#S3_META_PASSWORD} >= 16 )) || { echo "S3_META_PASSWORD needs at least 16 characters" >&2; exit 1; }

script="$(mktemp)"
trap 'rm -f "$script"' EXIT
chmod 600 "$script"
cat > "$script" <<STATEMENTS
DEFINE NAMESPACE IF NOT EXISTS ${S3_NAMESPACE} REPLICATION NONE ACKNOWLEDGE MAJORITY OR WEAKER;
USE NAMESPACE ${S3_NAMESPACE};
DEFINE DATABASE IF NOT EXISTS ${S3_DATABASE};
USE NAMESPACE ${S3_NAMESPACE} DATABASE ${S3_DATABASE};
DEFINE USER IF NOT EXISTS ${S3_META_USER} ON NAMESPACE ${S3_NAMESPACE} ROLE editor PASSWORD '${S3_META_PASSWORD}';
STATEMENTS
exec tessaridb --at "$TESSARIDB_AT" --user "$TESSARIDB_OWNER" -f "$script"
