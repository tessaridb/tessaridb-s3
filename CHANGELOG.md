# Changelog

Versions before 1.0 promise no compatibility with each other. Every release
section records which TessariDB version its metadata runs on.

## Unreleased

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
