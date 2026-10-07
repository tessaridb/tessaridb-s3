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
