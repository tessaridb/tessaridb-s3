# Changelog

Versions before 1.0 promise no compatibility with each other. Every release
section records which TessariDB version its metadata runs on.

## Unreleased

- Shards between nodes (second part of the erasure-coded data plane): a cluster member serves an internal shard
  surface on `TESSARIDB_S3_INTERNAL_LISTEN` where other nodes store, read (whole or by block range) and remove its
  shards. Each request is signed with the cluster secret and refused otherwise; a shard is kept only at exactly the
  length declared and is durable before the write is answered.
- Erasure-coded writes: on a cluster member every object or part above 128 KiB is cut into stripes and written as
  k + m shards, one per node of the layout (the order rotated per object by a hash keyed with the cluster secret).
  A write is acknowledged only once the write quorum of shards is durable (k, or k + 1 when k = m), otherwise it is
  answered `503 ServiceUnavailable` and its shards are reclaimed; a write missing some shards is listed for healing.
  Deleting or replacing an object reclaims its shards from every node.
- Erasure-coded reads: an object written as shards is read back stripe by stripe, whole or by range, from any k
  shards whose blocks verify; a shard that is missing, damaged or on a node that does not answer is replaced by the
  next one, and a read that had to do so lists the object for healing. Fewer than k readable shards answer
  `503 ServiceUnavailable`.
- Healing: every cluster member runs a healing pass every `TESSARIDB_S3_HEAL_INTERVAL_SECS` (default 60). Each
  object listed for healing has every shard checked block by block; a missing or damaged shard is rebuilt from the
  others and written back to its node, byte for byte as it was written, and a shard that verifies is never touched.
  An object with fewer than k readable shards stays listed and is logged as at risk.
- The internal shard surface over mutual TLS (`TESSARIDB_S3_INTERNAL_TLS_CERT`, `_KEY`, `_CA`, all three or
  none): nodes accept only connections whose certificate the cluster's authority issued and check each other's the
  same way, so shard bytes are encrypted and cannot be swapped on the way. Without them the surface stays plain and
  the node warns at start.
- A write cut short no longer blocks the next one: a node removes the temporary files a crash left in its data
  directory on start-up, and a write that stops part way (a client gone, a healing pass stopped) removes its own.
- A committed object whose data is missing is reported as damaged (`500 InternalError`, logged as corruption)
  rather than as a drive error.
- The operator console's API (first part): with `TESSARIDB_S3_CONSOLE_LISTEN` set, a node serves a console on
  its own address — in the clear or over TLS (`TESSARIDB_S3_CONSOLE_TLS_CERT`, `_KEY`). Signing in
  (`POST /api/v1/session`) takes the root access key and secret and answers a one-hour signed session cookie that
  any node of the cluster accepts; every other route answers `401` without it. Sign-ins are limited per client
  address and requests per signed-in key (`429`). `GET /api/v1/status` names the node, its build and its code.
- One node at a time heals or reclaims an object: each takes an expiring claim on the object's data in TessariDB
  first, so healers no longer duplicate work and a heal can no longer write a shard back behind a reclaim. Shard
  bytes left with no record of their object (older than the reclamation grace) are swept by the reclaimer.
- Every metadata statement is bounded in time (`TESSARIDB_S3_META_STATEMENT_TIMEOUT_SECS`, default 30): a metadata
  node that stops answering mid-statement no longer holds the request until the operating system gives up. The
  request answers `503`, and a write's data stays queued, since the statement may have committed.
  Before, such a file refused every later write of the same shard, so an object that lost a node mid-write stayed
  listed for healing for good.
- Cluster membership (first part of the erasure-coded data plane): with `TESSARIDB_S3_ERASURE` and its companion
  settings a node registers itself in the metadata on start-up, and the cluster's first layout — the lowest k + m
  registered node ids — is created once however many nodes propose it. Object data is still stored per node.
- Metadata on a TessariDB cluster: `TESSARIDB_S3_META_ADDRESS` takes several nodes, tried in turn (a node that
  does not answer is passed over; the client follows a redirect to the leader), and the namespace states its
  replication (`TESSARIDB_S3_META_REPLICATION`), required when more than one node is listed.
- Metadata runs on TessariDB `0.33.2-beta` or later: bucket and upload listings page with a bound `AFTER` cursor,
  which that release is the first to accept.
- Repository, licence (BUSL-1.1 with an object-storage-service and resale
  restriction) and mark.
- SigV4 verification: `Authorization` header, presigned URLs and the aws-chunked
  signature chain, with the declared payload hash checked before a write.
- Operation dispatch over the 116 operations of the AWS S3 model, generated from
  the model; unknown or misplaced parameters are refused.
- The `tessaridb-s3` server: path-style and virtual-hosted addressing, the S3
  error envelope with a request id on every response, `503 SlowDown` past the
  in-flight limit, graceful shutdown. Every operation is answered
  `NotImplemented`; nothing is stored yet.
- Buckets in TessariDB: CreateBucket (with an optional location constraint for
  this server's region), HeadBucket, DeleteBucket (refused while it holds
  objects), ListBuckets (prefix, region filter and pages) and GetBucketLocation.
  The metadata schema is applied on start-up.
- Objects up to 128 KiB, stored with their metadata in TessariDB: PutObject,
  GetObject, HeadObject and DeleteObject. The MD5 ETag; Content-MD5 and the
  CRC32, CRC32C, CRC64NVME, SHA1 and SHA256 checksums validated and stored, with
  CRC64NVME computed when none is sent; aws-chunked bodies with their signed or
  unsigned trailers; If-None-Match `*` and If-Match as one compare-and-set;
  conditional reads, a single Range and the `response-*` overrides. Headers for
  features not implemented here (tagging, encryption, Object Lock, ACLs other
  than owner-only) are refused rather than ignored.
- Objects above 128 KiB, up to 5 GiB per PUT (`EntityTooLarge` beyond), in data
  files under `TESSARIDB_S3_DATA_DIR`: the body streams to disk one 1 MiB block
  at a time with a BLAKE3 hash per block, is synced and renamed into place before
  the object is committed, and every block is verified when it is read — a
  damaged block is never served. A file's id is queued for reclamation in
  TessariDB before the file exists and leaves the queue only in the transaction
  that commits the object; an overwrite or a delete queues the data it replaces
  in that same transaction. The metadata schema is applied as one transaction.
- A reclaimer removes queued data files once they are past a grace period
  (24 h by default), never a file an object references; a commit that comes
  after its file was reclaimed is refused instead of pointing at nothing.
- ListObjectsV2 and ListObjects: keys in UTF-8 byte order read by seeking the
  bucket's records in TessariDB, a delimiter rolled up into one common prefix
  that counts once against `max-keys` (at most 1,000) and is skipped with one
  seek, `start-after` and `marker`, and V2 continuation tokens authenticated
  for the bucket, prefix and delimiter they were issued for — a page resumes
  correctly after the key it ended on was deleted. `encoding-type=url` encodes
  keys and prefixes; without it, a key XML cannot carry is refused rather than
  written as invalid XML. Objects of an earlier life of the bucket are not
  listed.
- DeleteObjects: up to 1,000 keys per request, each removed and reported on its
  own (a missing key counts as deleted), Quiet. The body must carry Content-MD5
  or a checksum, which is checked before anything is removed. A request naming a
  version or a per-object condition is refused whole with `NotImplemented`, so it
  deletes nothing.
- Multipart uploads, first half: CreateMultipartUpload (with its checksum
  algorithm and type recorded), UploadPart (each part in its own verified data
  file, a part number uploaded again replacing the earlier one), ListParts
  (paged by part number) and AbortMultipartUpload. Upload ids are 128 random
  bits, bound to their bucket, key and bucket incarnation; parts are never
  listed or read as objects; every byte an abort or a replacement drops is
  queued for reclamation, and the reclaimer never removes a file a part still
  holds.
- CompleteMultipartUpload: the listed parts are checked (ascending order,
  ETag and any listed checksum, 5 MiB for every part but the last) and become
  the object in one commit that also drops the unlisted parts and ends the
  upload; `If-None-Match` and `If-Match` apply. The object carries the
  multipart ETag (`MD5 of the part MD5s-N`) and a full-object CRC combined from
  the parts' CRCs, or the composite checksum its upload declared. GetObject and
  HeadObject read it across its part files, with a Range or `partNumber`;
  overwriting or deleting it queues every part's file. CreateMultipartUpload
  now accepts a declared checksum algorithm (it was refused as an unknown
  checksum header) and refuses a type the algorithm does not allow.
- ListMultipartUploads: a bucket's open uploads by key in byte order and,
  within a key, by initiation; paged by `key-marker` and `upload-id-marker`,
  rolled up at a delimiter (a CommonPrefix counts once), `encoding-type=url`.
  An upload that completes or is aborted leaves the listing in the same commit.
- CopyObject: up to 5 GiB, any source (inline, data file or multipart) becomes a
  single-part object with its own bytes, an MD5 ETag and a checksum recomputed
  over the copy and checked against the source's; `x-amz-metadata-directive`
  COPY or REPLACE, the copy-source conditions (412 when one does not hold), the
  destination's If-None-Match / If-Match, and copying onto itself only with
  REPLACE. A version, SSE-C and a source past 5 GiB are refused by name.
- UploadPartCopy: a part made of a source object's bytes — `x-amz-copy-source-range`
  (`bytes=first-last` only) or the whole source — with the copy-source
  conditions, an MD5 ETag of the copied bytes and a data file of its own; the
  upload is checked before the source is read.
- GetObjectTagging answers an existing object's tag set, which is always empty
  (PutObject refuses `x-amz-tagging` and nothing else writes tags) — the AWS
  CLI reads it before a server-side multipart copy.
- The upload reaper: a multipart upload open longer than
  `TESSARIDB_S3_UPLOAD_MAX_AGE_SECS` (7 days by default, counted from its
  initiation) is aborted exactly as AbortMultipartUpload would, so its parts'
  files are reclaimed — the backstop for uploads nobody finishes.
