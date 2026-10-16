//! The customer's key is never kept (G017 C2): after objects and an upload are sealed under one, no record of the
//! store's databases and no file under the data directory holds the key — as bytes, in base64 or in hex — while the
//! same scans do find what they are pointed at: a seal's salt, a probe in user metadata, a plain object's bytes.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tessari_s3_api::ApiState;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};
use tessari_s3_types::{BucketName, ObjectKey};

use crate::multipart::text;
use crate::sse::body;
use crate::sse_c::{Customer, allowing};
use crate::{IGNORED, call_tls, call_with, live_config, prepared, scratch_dir, test_node};

/// What a scan looks for: the raw bytes, and the two ways text carries them.
struct Needle {
    bytes: Vec<u8>,
    base64: String,
    hex: String,
}

impl Needle {
    fn of(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            base64: STANDARD.encode(bytes),
            hex: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        }
    }

    fn in_bytes(&self, haystack: &[u8]) -> bool {
        [
            self.bytes.as_slice(),
            self.base64.as_bytes(),
            self.hex.as_bytes(),
        ]
        .iter()
        .any(|needle| {
            haystack
                .windows(needle.len())
                .any(|window| window == *needle)
        })
    }

    fn in_value(&self, value: &Value) -> bool {
        match value {
            Value::Bytes(bytes) => self.in_bytes(bytes),
            Value::String(text) => self.in_bytes(text.as_bytes()),
            Value::Array(items) => items.iter().any(|item| self.in_value(item)),
            Value::Object(fields) => fields
                .iter()
                .any(|(name, item)| self.in_bytes(name.as_bytes()) || self.in_value(item)),
            _ => false,
        }
    }
}

/// Every record of every table in `pool`'s database, and the tables whose records could not be read.
async fn records(pool: &MetaPool) -> (Vec<Value>, Vec<String>) {
    let info = pool
        .run("INFO FOR DATABASE;", Vec::new())
        .await
        .expect("database info");
    let tables = match info.into_iter().next() {
        Some(Answer::Value {
            value: Value::Object(info),
            ..
        }) => match info.get("tables") {
            Some(Value::Array(names)) => names
                .iter()
                .filter_map(|name| match name {
                    Value::String(name) => Some(name.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            other => panic!("no table list: {other:?}"),
        },
        other => panic!("not a value answer: {other:?}"),
    };
    let (mut all, mut unread) = (Vec::new(), Vec::new());
    for table in tables {
        match pool
            .run(&format!("SELECT * FROM {table};"), Vec::new())
            .await
        {
            Ok(answers) => {
                for answer in answers {
                    match answer {
                        Answer::Records { records, .. } => {
                            for (id, value) in records {
                                all.push(Value::String(id));
                                all.push(value);
                            }
                        }
                        Answer::Value { value, .. } => all.push(value),
                        _ => {}
                    }
                }
            }
            Err(_) => unread.push(table),
        }
    }
    (all, unread)
}

/// Every file under `dir`, read whole.
fn files(dir: &std::path::Path) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(at) = pending.pop() {
        for entry in std::fs::read_dir(&at).expect("a readable directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(std::fs::read(&path).expect("a readable file"));
            }
        }
    }
    found
}

async fn salt_of(state: &ApiState, key: &str) -> [u8; 32] {
    let object = state
        .storage()
        .objects()
        .get(
            &BucketName::new("sealed").expect("bucket"),
            &ObjectKey::new(key).expect("key"),
        )
        .await
        .expect("read")
        .ok()
        .flatten()
        .expect("the object");
    object
        .customer_seal()
        .expect("sealed under a customer key")
        .salt
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn no_record_and_no_file_holds_a_customer_key_after_it_sealed_objects() {
    let _why = IGNORED;
    let dir = scratch_dir();
    let config = live_config(test_node(), None, &dir);
    let state = ApiState::new(&config, ApiState::system_clock(), prepared(&config).await);
    let global = MetaPool::new(config.meta.clone()).expect("pool");
    let regional = global
        .regional(&config.region)
        .expect("the region's database");
    let key: Vec<u8> = (0_u8..32).map(|at| 0xA5 ^ at.wrapping_mul(37)).collect();
    let customer = Customer::of(&key);
    allowing(&state, "sealed").await;
    let put = call_tls(
        &state,
        "PUT",
        "/sealed/one",
        vec![],
        customer.headers(),
        &body(),
    )
    .await;
    assert_eq!(put.status, 200, "{}", put.body);
    let created = call_tls(
        &state,
        "POST",
        "/sealed/many",
        vec![("uploads", None)],
        customer.headers(),
        b"",
    )
    .await;
    assert_eq!(created.status, 200, "{}", created.body);
    let upload = text(&created.body, "UploadId").expect("an upload id");
    let part = call_tls(
        &state,
        "PUT",
        "/sealed/many",
        vec![
            ("partNumber", Some("1")),
            ("uploadId", Some(upload.as_str())),
        ],
        customer.headers(),
        &body(),
    )
    .await;
    assert_eq!(part.status, 200, "{}", part.body);
    let etag = part
        .headers
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .expect("a part ETag")
        .to_owned();
    let done = call_tls(
        &state,
        "POST",
        "/sealed/many",
        vec![("uploadId", Some(upload.as_str()))],
        vec![],
        &crate::complete::document(&[("1", etag.as_str())]),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let probe = Needle::of(&[0x5A_u8; 32]);
    let plain_bytes = b"plain bytes any drive scan finds".repeat(64);
    // In a bucket with no default, so its bytes are stored as they came.
    let open = crate::call(&state, "PUT", "/open", vec![], b"").await;
    assert_eq!(open.status, 200, "{}", open.body);
    let plain = call_with(
        &state,
        "PUT",
        "/open/plain",
        vec![],
        vec![("x-amz-meta-probe", probe.base64.as_str())],
        &plain_bytes,
    )
    .await;
    assert_eq!(plain.status, 200, "{}", plain.body);

    let secret = Needle::of(&key);
    let (mut scanned, mut unreadable) = (Vec::new(), Vec::new());
    for pool in [&global, &regional] {
        let (found, unread) = records(pool).await;
        unreadable.extend(unread);
        scanned.extend(found);
    }
    // The server's own keys are a vault, read only by REVEAL; nothing a customer's key touches writes there.
    assert_eq!(
        unreadable.iter().map(String::as_str).collect::<Vec<_>>(),
        ["s3_keys"],
        "every table but the server keys' vault is read"
    );
    let salt = Needle::of(&salt_of(&state, "one").await);
    assert!(
        scanned.iter().any(|value| salt.in_value(value)),
        "control: the scan reaches the seal's bytes"
    );
    assert!(
        scanned.iter().any(|value| probe.in_value(value)),
        "control: the scan reaches text in user metadata"
    );
    assert!(
        !scanned.iter().any(|value| secret.in_value(value)),
        "no record holds the customer's key"
    );

    let stored = files(&dir);
    let plain_needle = Needle::of(&plain_bytes[..64]);
    assert!(
        stored.iter().any(|file| plain_needle.in_bytes(file)),
        "control: the drive scan reads the data files"
    );
    assert!(
        !stored.iter().any(|file| secret.in_bytes(file)),
        "no file holds the customer's key"
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}
