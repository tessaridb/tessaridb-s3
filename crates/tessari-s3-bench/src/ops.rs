//! The measured operations. Each builds what it sends before starting the clock, and stops the clock at the last
//! byte of the answer; checking a GET body happens after the clock stops.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use reqwest::Method;
use tessari_s3_constants::{BENCH_MULTIPART_PARTS, BENCH_PART_BYTES};

use crate::body::{SizeClass, prepare, sha256_hex};
use crate::client::{EMPTY_SHA256, Failure, S3Client, encode_value, refusal};
use crate::measure::OpOutcome;

/// The BLAKE3 of each key's body as it was PUT, by key index; a GET compares against it.
pub type Digests = Arc<[OnceLock<[u8; 32]>]>;

/// What one operation needs to know.
#[derive(Debug, Clone)]
pub struct Target {
    /// The client.
    pub client: Arc<S3Client>,
    /// The bucket.
    pub bucket: String,
    /// The size class.
    pub class: SizeClass,
    /// The key's index in the phase.
    pub index: usize,
    /// The key.
    pub key: String,
}

fn outcome(latency: Duration, prep: Duration, result: Result<(), Failure>) -> OpOutcome {
    OpOutcome {
        latency,
        prep,
        result,
        mismatch: false,
    }
}

/// PUTs the key's body (or uploads it in parts) and records its digest.
pub async fn put(target: Target, digests: Digests) -> OpOutcome {
    if target.class == SizeClass::Multipart {
        return multipart(target, digests).await;
    }
    let started = Instant::now();
    let len = usize::try_from(target.class.bytes()).unwrap_or(usize::MAX);
    let key = target.key.clone();
    let Ok(prepared) = tokio::task::spawn_blocking(move || prepare(&key, 0, len)).await else {
        return outcome(Duration::ZERO, started.elapsed(), Err(Failure::Transport));
    };
    if let Some(slot) = digests.get(target.index) {
        let _first = slot.set(*blake3::hash(&prepared.body).as_bytes());
    }
    let prep = started.elapsed();
    let path = format!("/{}/{}", target.bucket, target.key);
    let clock = Instant::now();
    let result = match target
        .client
        .send(
            Method::PUT,
            &path,
            "",
            Some(prepared.body),
            &prepared.sha256_hex,
        )
        .await
    {
        Ok(response) => refusal(response).await.map_or(Ok(()), Err),
        Err(_) => Err(Failure::Transport),
    };
    outcome(clock.elapsed(), prep, result)
}

/// GETs the key and checks the body against the PUT's digest once the clock has stopped.
pub async fn get(target: Target, digests: Digests) -> OpOutcome {
    let path = format!("/{}/{}", target.bucket, target.key);
    let clock = Instant::now();
    let response = match target
        .client
        .send(Method::GET, &path, "", None, EMPTY_SHA256)
        .await
    {
        Ok(response) => response,
        Err(_) => return outcome(clock.elapsed(), Duration::ZERO, Err(Failure::Transport)),
    };
    if !response.status().is_success() {
        let failure = refusal(response).await.unwrap_or(Failure::Transport);
        return outcome(clock.elapsed(), Duration::ZERO, Err(failure));
    }
    let mut response = response;
    let mut chunks: Vec<Bytes> = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => chunks.push(chunk),
            Ok(None) => break,
            Err(_) => return outcome(clock.elapsed(), Duration::ZERO, Err(Failure::Transport)),
        }
    }
    let latency = clock.elapsed();
    let expected = digests.get(target.index).and_then(OnceLock::get).copied();
    let matches = tokio::task::spawn_blocking(move || {
        let mut hasher = blake3::Hasher::new();
        for chunk in &chunks {
            hasher.update(chunk);
        }
        expected.is_some_and(|expected| *hasher.finalize().as_bytes() == expected)
    })
    .await
    .unwrap_or(false);
    OpOutcome {
        latency,
        prep: Duration::ZERO,
        result: Ok(()),
        mismatch: !matches,
    }
}

/// HEADs or DELETEs the key.
pub async fn bodiless(target: Target, method: Method) -> OpOutcome {
    let path = format!("/{}/{}", target.bucket, target.key);
    let clock = Instant::now();
    let result = match target
        .client
        .send(method, &path, "", None, EMPTY_SHA256)
        .await
    {
        Ok(response) => refusal(response).await.map_or(Ok(()), Err),
        Err(_) => Err(Failure::Transport),
    };
    outcome(clock.elapsed(), Duration::ZERO, result)
}

/// One ListObjectsV2 page (up to 1,000 keys) under `prefix`, read to the end.
pub async fn list(client: Arc<S3Client>, bucket: String, prefix: String) -> OpOutcome {
    let query = format!("list-type=2&max-keys=1000&prefix={}", encode_value(&prefix));
    let path = format!("/{bucket}");
    let clock = Instant::now();
    let result = match client
        .send(Method::GET, &path, &query, None, EMPTY_SHA256)
        .await
    {
        Ok(response) if response.status().is_success() => response
            .bytes()
            .await
            .map(|_| ())
            .map_err(|_| Failure::Transport),
        Ok(response) => Err(refusal(response).await.unwrap_or(Failure::Transport)),
        Err(_) => Err(Failure::Transport),
    };
    outcome(clock.elapsed(), Duration::ZERO, result)
}

/// Create, parts of `BENCH_PART_BYTES`, complete. Latency is the time on the wire summed over every request;
/// building each part is preparation.
async fn multipart(target: Target, digests: Digests) -> OpOutcome {
    let path = format!("/{}/{}", target.bucket, target.key);
    let (mut latency, mut prep) = (Duration::ZERO, Duration::ZERO);
    let clock = Instant::now();
    let created = target
        .client
        .send(Method::POST, &path, "uploads", None, EMPTY_SHA256)
        .await;
    let upload_id = match created {
        Ok(response) if response.status().is_success() => {
            let text = response.text().await.unwrap_or_default();
            element(&text, "UploadId")
        }
        Ok(response) => {
            let failure = refusal(response).await.unwrap_or(Failure::Transport);
            return outcome(clock.elapsed(), prep, Err(failure));
        }
        Err(_) => None,
    };
    latency = latency.saturating_add(clock.elapsed());
    let Some(upload_id) = upload_id else {
        return outcome(latency, prep, Err(Failure::Transport));
    };
    let mut hasher = blake3::Hasher::new();
    let mut etags = Vec::new();
    let len = usize::try_from(BENCH_PART_BYTES).unwrap_or(usize::MAX);
    for part in 1..=BENCH_MULTIPART_PARTS {
        let started = Instant::now();
        let key = target.key.clone();
        let Ok(prepared) = tokio::task::spawn_blocking(move || prepare(&key, part, len)).await
        else {
            return outcome(latency, prep, Err(Failure::Transport));
        };
        hasher.update(&prepared.body);
        prep = prep.saturating_add(started.elapsed());
        let query = format!("partNumber={part}&uploadId={}", encode_value(&upload_id));
        let clock = Instant::now();
        let sent = target
            .client
            .send(
                Method::PUT,
                &path,
                &query,
                Some(prepared.body),
                &prepared.sha256_hex,
            )
            .await;
        let result = match sent {
            Ok(response) if response.status().is_success() => response
                .headers()
                .get("etag")
                .and_then(|etag| etag.to_str().ok())
                .map(str::to_owned)
                .ok_or(Failure::Transport),
            Ok(response) => Err(refusal(response).await.unwrap_or(Failure::Transport)),
            Err(_) => Err(Failure::Transport),
        };
        latency = latency.saturating_add(clock.elapsed());
        match result {
            Ok(etag) => etags.push((part, etag)),
            Err(failure) => return outcome(latency, prep, Err(failure)),
        }
    }
    let mut xml = String::from("<CompleteMultipartUpload>");
    for (part, etag) in &etags {
        xml.push_str(&format!(
            "<Part><PartNumber>{part}</PartNumber><ETag>{etag}</ETag></Part>"
        ));
    }
    xml.push_str("</CompleteMultipartUpload>");
    let sha256_hex = sha256_hex(xml.as_bytes());
    let query = format!("uploadId={}", encode_value(&upload_id));
    let clock = Instant::now();
    let result = match target
        .client
        .send(
            Method::POST,
            &path,
            &query,
            Some(Bytes::from(xml)),
            &sha256_hex,
        )
        .await
    {
        // A Complete can fail inside a 200, so the body is read for an error element too.
        Ok(response) if response.status().is_success() => {
            let text = response.text().await.unwrap_or_default();
            element(&text, "Code")
                .map_or(Ok(()), |code| Err(Failure::Refused { status: 200, code }))
        }
        Ok(response) => Err(refusal(response).await.unwrap_or(Failure::Transport)),
        Err(_) => Err(Failure::Transport),
    };
    latency = latency.saturating_add(clock.elapsed());
    if result.is_ok()
        && let Some(slot) = digests.get(target.index)
    {
        let _first = slot.set(*hasher.finalize().as_bytes());
    }
    outcome(latency, prep, result)
}

/// The text of the first `<name>…</name>` element.
fn element(xml: &str, name: &str) -> Option<String> {
    let (_, rest) = xml.split_once(&format!("<{name}>"))?;
    let (value, _) = rest.split_once(&format!("</{name}>"))?;
    Some(value.to_owned())
}
