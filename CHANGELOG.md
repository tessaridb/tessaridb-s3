# Changelog

Versions before 1.0 promise no compatibility with each other. Every release
section records which TessariDB version its metadata runs on.

## Unreleased

- The compose file and the process tests run TessariDB `0.34.0-beta`; the whole live suite passes against it.
  The metadata store still needs `0.33.2-beta` or later.
- Bucket lifecycle configuration: PutBucketLifecycleConfiguration, GetBucketLifecycleConfiguration
  (`NoSuchLifecycleConfiguration` when none) and DeleteBucketLifecycle. Rules filter by prefix and say Expiration
  (`Days`, `Date` at midnight UTC, or `ExpiredObjectDeleteMarker`), `NoncurrentVersionExpiration` (with
  `NewerNoncurrentVersions`) and `AbortIncompleteMultipartUpload`; transitions and tag or size filters are refused
  `NotImplemented` rather than accepted and ignored. The rules are stored; applying them comes next.
- ListObjectVersions: every version and delete marker of a bucket, keys in UTF-8 byte order and each key's versions
  newest first with `IsLatest`, paged by `key-marker` and `version-id-marker` (`NextKeyMarker` and
  `NextVersionIdMarker` on a truncated page), rolled up at a delimiter, `encoding-type=url` supported. A
  `version-id-marker` without a `key-marker`, or one spelled as no version id, is refused `InvalidArgument`.
- GetObject and HeadObject take `versionId`: they answer that version (`null` names the null version), `NoSuchVersion`
  for an id naming none and `InvalidArgument` for a malformed one; a version that is a delete marker answers 405 with
  `x-amz-delete-marker` and `Last-Modified`, and a key whose newest version is a delete marker answers 404 naming the
  marker in `x-amz-delete-marker` and `x-amz-version-id`.
- Object reads, the object listing and the uploads listing run as plain reads instead of transactions. A multipart
  object's parts are read before the object is confirmed to still name them, so a read that overlaps an overwrite
  answers the old object or the new one whole, never a part short. Writes that change several records stay one
  transaction.
- DeleteObjects takes a `VersionId` per entry: that version is removed as DeleteObject with `versionId` removes it,
  each `<Deleted>` names its `VersionId`, and a removed or added delete marker is reported with `DeleteMarker` and
  `DeleteMarkerVersionId`. Entries naming a version are refused `AccessDenied` one by one for a caller who does not
  administer the bucket, and the batch's other entries go ahead.
- DeleteObject with `versionId` permanently removes that version: when it was the newest the next newest becomes
  current, and removing a delete marker brings the object back (`x-amz-delete-marker: true`). It needs who administers
  the bucket, not a writer; a malformed id is refused `InvalidArgument`, an id naming no version answers 204. A simple
  delete in a versioned bucket answers the marker it added (`x-amz-delete-marker`, `x-amz-version-id`).
- Versioned buckets keep their versions: in an enabled bucket every write adds a version (its id answered in
  `x-amz-version-id`, by PutObject, CopyObject, CompleteMultipartUpload, GetObject and HeadObject) and a delete adds a
  delete marker, keeping every earlier version and its bytes; in a suspended bucket a write or delete replaces only the
  null version. Concurrent writers of one key get one version each, in commit order. DeleteBucket refuses
  `BucketNotEmpty` while any version or marker remains. A store from an earlier build gives each of its objects a null
  version at start, once.
- Console bucket explorer: each listed key shows its content type as stored at write (an icon and a label) beside
  size, age and ETag; selecting a key shows its headers, user metadata, checksums and part count in a side pane, read
  from its record without opening the object (the arrow keys move the selection, Escape closes it); the list appends
  the next page by the server's cursor as it is scrolled, or with "Load more"; a name filter lists only the keys in the
  open folder that start with it, served by the store as a longer prefix; each row's details, download, copy-key and
  ETag-guarded delete (with a reason) are icon buttons. A caller who may not read a bucket gets the same refusal from
  every one of these routes, with nothing about the bucket or its keys in it.
- A burst of requests on a cold server no longer fails: new metadata connections sign in at most 4 at a time, a
  connection whose sign-in was refused is dropped instead of reused, and a metadata store that is throttling sign-ins
  answers S3 clients `SlowDown` (503) rather than `InternalError`.
- GetObject and HeadObject with a `partNumber` past the object's last part answer `InvalidPartNumber` (416), and
  `partNumber=1` on a single-part object answers the whole object.
- Upload ids sort by when the upload began (128 random bits kept), so ListMultipartUploads pages with
  `upload-id-marker` neither repeat nor skip an upload, even one that has ended; `max-uploads=0` is refused
  `InvalidArgument`. Upload ids from an earlier build still work.
- UploadPartCopy with `x-amz-copy-source-range` on a source of 5 MiB or less is refused `InvalidRequest`, as S3 does.
- Console access keys: issuing takes a token the form minted when it opened, so a submission sent twice issues one
  key and the repeat shows no secret; each user's keys are listed without secrets and revoked with a reason, refused
  on every node within 5 seconds. Row actions are icon buttons with tooltips.
- Bucket quotas no longer trust an old measurement: when the usage measurement is older than two usage intervals (or
  missing), a write to a limited bucket counts that bucket on the spot (within 2 seconds, through a new index, or it
  is refused `ServiceUnavailable`), logs a warning, and the console marks the sizes stale.
- The console's green and amber text clear 4.5:1 contrast on every light surface (checked by a test).
- The server signs in to TessariDB as its own account — an editor of its one database, never the owner. Start-up
  creates the namespace only when its account may; `compose.yaml` declares the account once with a new
  `S3_META_PASSWORD` (see `.env.example`).
- Spaces, users and grants: an operator creates spaces and users; every user belongs to one space as its
  administrator or a member, may be allowed to create buckets there, and may be made a store-wide operator or cluster
  viewer. A bucket belongs to the space it was created in; a user lists and reaches only its own space's buckets — a
  member only those it created or was granted read or write on — and a bucket of another space answers as if it did
  not exist. One evaluator decides every S3 operation and every console route, deny by default.
- User access keys: issued with the secret shown once, stored sealed (XChaCha20-Poly1305 under a per-space key
  derived from `TESSARIDB_S3_IAM_KEY`, bound to the key, its user and space); without that variable only the root
  credential signs in. Disabling a user or a key, or removing a grant, takes effect on every node within 5 seconds.
- One-key upload credentials: a user who may write a bucket issues, from the console, a key that uploads exactly one
  named key of it — a single PUT or every step of a multipart upload — and nothing else, for 60 seconds to 7 days,
  and only while the issuer may still write there. The metadata store drops it at its expiry.
- Bucket quotas: an operator sets a byte and an object limit on a bucket (console, with a reason that is recorded);
  every write that adds — PutObject, CopyObject, UploadPart, UploadPartCopy, CompleteMultipartUpload — is checked
  against the last usage measurement plus the write and refused `InvalidRequest` "bucket quota exceeded". A bucket can
  pass its limit by what is written within one usage interval; an unreadable limit or measurement refuses the write.
- Real occupancy: the usage pass also counts the bytes each bucket holds on the drives (with the erasure overhead on
  a cluster); the console shows them beside the logical size.
- No object bytes in the metadata: every object, however small, is a data file on the drive (or erasure-coded shards
  on a cluster); TessariDB holds sizes, ETags, checksums and where the bytes are. `TESSARIDB_S3_DATA_DIR` is now
  required. A store written by an earlier build has its small objects moved to data files at start, and the column
  that held them is dropped, after which the metadata store refuses object bytes in a record.
- Reclamation and healing run on TessariDB queues: a replaced, deleted or abandoned data file waits in a queue until
  its grace ends, and the ids waiting for healing in another; each node takes distinct entries one at a time, and an
  entry a node dies holding is handed out again 30 seconds later. A heal that keeps failing steps aside and, after
  ten attempts, is shown at risk in the console's healing backlog. A store written by an earlier build has its
  reclamation and healing lists moved onto the queues at start.
- The console for users: sign in with any key (root or a user's); menus, screens and routes follow what that key
  may do — the cluster view only for those allowed to see it; spaces, users, keys, grants and quotas for operators and
  space administrators (their own space only).
- A container image that holds the server only (`Dockerfile`, runs as an unprivileged user, a health check) and a
  `compose.yaml` that runs it beside a TessariDB container; data survives a restart in named volumes.

- Storage usage: every node measures how many objects and how many bytes each bucket holds every
  `TESSARIDB_S3_USAGE_INTERVAL_SECS` (default 60); an expiring claim in TessariDB lets one member measure per pass.
  The console shows the totals with the time they were taken, each bucket's objects and size, and the space of every
  member's data drive (capacity, free, available) with a bar that turns amber at 80 % and red at 95 %.
- Shards between nodes (second part of the erasure-coded data plane): a cluster member serves an internal shard
  surface on `TESSARIDB_S3_INTERNAL_LISTEN` where other nodes store, read (whole or by block range) and remove its
  shards. Each request is signed with the cluster secret and refused otherwise; a shard is kept only at exactly the
  length declared and is durable before the write is answered.
- Erasure-coded writes: on a cluster member every object or part is cut into stripes and written as
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
- The operator console's data (second part): `GET /api/v1/status` adds a cluster member's members and its heal
  backlog; `GET/POST /api/v1/buckets` and `DELETE /api/v1/buckets/{bucket}` list, create and delete buckets
  through the same rules as S3; `GET /api/v1/buckets/{bucket}/objects` lists a bucket a page at a time (prefix,
  delimiter, cursor); `…/object?key=` describes an object, `…/object/content?key=` downloads it as an attachment
  of an opaque type, and `DELETE …/object?key=` deletes it only under the ETag given (`412` when it changed).
  Every change and every download is appended to a year-long action record in TessariDB with the signed-in key,
  the outcome and the operator's reason — required to delete — and `GET /api/v1/actions` reads it newest first.
- The operator console's page: the console address now serves a web page — sign-in, an overview of the node and
  its cluster, buckets (create, delete with a reason), a bucket's keys browsed by prefix a page at a time, an
  object's details with a recorded download and an ETag-guarded delete, and the action record — in light and dark.
  It loads nothing from off the node, runs under a strict content security policy, writes every name it shows as
  text, and does everything through the console API. The page's routes need no session (they carry no data) and
  are limited per client address.
- The console's overview says which cluster members answer: `GET /api/v1/status` gives each member an `answering`
  flag — the node itself, and every other member that answered a probe signed with the cluster secret within two
  seconds — and the page shows it per member and as "N of M answering". Members answer that probe on the internal
  surface at `GET /internal/v1/health`, behind the same signature check as the shard routes.
- Nodes starting together agree on the first layout again: a node whose proposal was refused because another
  node's committed after it began proposes again and reads that layout back (at most three times), instead of
  failing with the store's `retry` refusal.
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
- PutObject, GetObject, HeadObject and DeleteObject, with the metadata in TessariDB. The MD5 ETag; Content-MD5 and the
  CRC32, CRC32C, CRC64NVME, SHA1 and SHA256 checksums validated and stored, with
  CRC64NVME computed when none is sent; aws-chunked bodies with their signed or
  unsigned trailers; If-None-Match `*` and If-Match as one compare-and-set;
  conditional reads, a single Range and the `response-*` overrides. Headers for
  features not implemented here (tagging, encryption, Object Lock, ACLs other
  than owner-only) are refused rather than ignored.
- Objects up to 5 GiB per PUT (`EntityTooLarge` beyond), in data
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
- CopyObject: up to 5 GiB, any source (a data file or multipart) becomes a
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
