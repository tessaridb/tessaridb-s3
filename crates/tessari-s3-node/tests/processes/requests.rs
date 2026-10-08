//! Signed S3 requests to one node over real HTTP — whole, or with a fault injected while the body is still in
//! flight.

use std::future::Future;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use tessari_s3_types::Timestamp;
use tokio::sync::mpsc;

use crate::s3::{ACCESS_KEY, Node, SECRET};
use crate::signer;

/// How long one request may take, faults included.
const REQUEST_WITHIN: Duration = Duration::from_secs(120);

/// `YYYYMMDDTHHMMSSZ` for now.
fn amz_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).expect("seconds fit"))
        .expect("after the epoch");
    let iso = Timestamp { seconds, nanos: 0 }.iso8601_millis();
    format!(
        "{}{}{}T{}{}{}Z",
        &iso[0..4],
        &iso[5..7],
        &iso[8..10],
        &iso[11..13],
        &iso[14..16],
        &iso[17..19]
    )
}

/// A request to `node` signed over `body`, not yet sent.
fn signed(node: &Node, method: &str, path: &str, body: &[u8]) -> reqwest::RequestBuilder {
    let amz_date = amz_now();
    let unsigned = signer::Unsigned {
        method,
        host: &node.s3,
        path,
        query: Vec::new(),
        headers: Vec::new(),
        access_key: ACCESS_KEY,
        secret: SECRET,
        region: "us-east-1",
        amz_date: &amz_date,
    };
    let (target, headers) = signer::sign_with_body(&unsigned, body);
    let mut request = reqwest::Client::new()
        .request(
            reqwest::Method::from_bytes(method.as_bytes()).expect("method"),
            format!("http://{}{target}", node.s3),
        )
        .timeout(REQUEST_WITHIN);
    for (name, value) in headers.iter().filter(|(name, _)| name != "host") {
        request = request.header(name.as_str(), value.as_str());
    }
    request
}

/// Sends one signed request to `node` and answers its status and body.
pub async fn call(node: &Node, method: &str, path: &str, body: &[u8]) -> (u16, String) {
    let response = signed(node, method, path, body)
        .body(body.to_vec())
        .send()
        .await
        .expect("the node answers");
    let status = response.status().as_u16();
    (status, response.text().await.unwrap_or_default())
}

/// A PUT's status and body, or the transport's error when the node did not answer.
pub async fn put(node: &Node, path: &str, body: &[u8]) -> Result<(u16, String), reqwest::Error> {
    let response = signed(node, "PUT", path, body)
        .body(body.to_vec())
        .send()
        .await?;
    let status = response.status().as_u16();
    Ok((status, response.text().await.unwrap_or_default()))
}

/// A GET's status and body bytes, or the transport's error when the node did not answer.
pub async fn fetch(node: &Node, path: &str) -> Result<(u16, Vec<u8>), reqwest::Error> {
    let response = signed(node, "GET", path, b"").send().await?;
    let status = response.status().as_u16();
    Ok((status, response.bytes().await?.to_vec()))
}

/// PUTs `body` to `node` with the first `sent` bytes queued before `between` runs and the rest after it, and
/// answers the status — or the transport's error when the node stopped answering.
pub async fn put_interrupted<F: Future<Output = ()>>(
    node: &Node,
    path: &str,
    body: &[u8],
    sent: usize,
    between: F,
) -> Result<u16, reqwest::Error> {
    let (chunks, queued) = mpsc::channel::<Result<Bytes, std::io::Error>>(4);
    let stream = futures_util::stream::unfold(queued, |mut queued| async move {
        queued.recv().await.map(|chunk| (chunk, queued))
    });
    let request = signed(node, "PUT", path, body)
        .header("content-length", body.len().to_string())
        .body(reqwest::Body::wrap_stream(stream));
    let sending = tokio::spawn(request.send());
    let (head, tail) = body.split_at(sent);
    for part in head.chunks(64 * 1024) {
        chunks
            .send(Ok(Bytes::copy_from_slice(part)))
            .await
            .expect("the request reads its body");
    }
    between.await;
    for part in tail.chunks(64 * 1024) {
        if chunks.send(Ok(Bytes::copy_from_slice(part))).await.is_err() {
            // The request ended early; its outcome says why.
            break;
        }
    }
    drop(chunks);
    let response = sending.await.expect("the request task")?;
    Ok(response.status().as_u16())
}

/// GETs `path` from `node`, runs `between` once the first bytes have arrived, and answers the status and the whole
/// body read after it.
pub async fn get_interrupted<F: Future<Output = ()>>(
    node: &Node,
    path: &str,
    between: F,
) -> (u16, Vec<u8>) {
    let response = signed(node, "GET", path, b"")
        .send()
        .await
        .expect("the node answers");
    let status = response.status().as_u16();
    let mut body = response.bytes_stream();
    let mut bytes = body
        .next()
        .await
        .expect("a first chunk")
        .expect("the first chunk reads")
        .to_vec();
    between.await;
    while let Some(chunk) = body.next().await {
        bytes.extend_from_slice(&chunk.expect("the rest of the body reads"));
    }
    (status, bytes)
}
