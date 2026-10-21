//! Every protected console route, read from the router's own source rather than listed by hand: without a session each
//! answers 401, and signed in as the administrator or a plain member of ANOTHER space each is refused — or, for the
//! few routes that answer about the caller's own space, answers without naming anything of the space it is not in.
//! Each request carries a valid body, so a refusal is the authorization's and not the parser's.

use serde_json::json;
use tessari_s3_core::authz::Role;

use crate::IGNORED;
use crate::access::{Key, user_with_key};
use crate::console::{Seen, send, sign_in};
use crate::console_users::{sign_in_as, states};

/// The router whose protected routes are probed.
const ROUTER: &str = include_str!("../../src/console/mod.rs");

/// The names a probe answer must never carry: the other space, its users, bucket, object and group, as JSON strings.
const MARKERS: [&str; 6] = [
    "\"alpha\"",
    "\"alpha-files\"",
    "\"ann\"",
    "\"boss\"",
    "\"crew\"",
    "\"doc.txt\"",
];

/// The routes that answer about the caller's own space, its own session or this node, so an outsider may be answered
/// with 2xx — as long as nothing of space `alpha` is in it.
const OWN_SCOPE: [(&str, &str); 7] = [
    ("GET", "/api/v1/status"),
    ("GET", "/api/v1/session"),
    ("DELETE", "/api/v1/session"),
    ("GET", "/api/v1/buckets"),
    ("POST", "/api/v1/buckets"),
    ("GET", "/api/v1/usage"),
    ("GET", "/api/v1/users"),
];

/// Every `(method, path)` the protected router registers, in source order.
fn protected_routes() -> Vec<(String, String)> {
    let start = ROUTER
        .find("let protected = axum::Router::new()")
        .expect("the protected router");
    let end = ROUTER[start..]
        .find(".fallback(not_found)")
        .expect("its end");
    let mut routes = Vec::new();
    for piece in ROUTER[start..start.saturating_add(end)]
        .split(".route(")
        .skip(1)
    {
        let path = piece.split('"').nth(1).expect("a path literal").to_owned();
        let handlers = &piece[piece.find(',').expect("handlers")..];
        for method in ["get", "post", "put", "delete"] {
            let mut rest = handlers;
            while let Some(at) = rest.find(&format!("{method}(")) {
                let before = rest[..at].chars().last();
                if matches!(before, Some(' ' | '.' | '\n' | '(')) {
                    routes.push((method.to_uppercase(), path.clone()));
                }
                rest = &rest[at.saturating_add(method.len())..];
            }
        }
    }
    routes
}

/// The request that probes `route`: its concrete path and a body a caller allowed to make it would send.
fn probe(
    route: &(String, String),
    key: &str,
    etag: &str,
    who: usize,
) -> Option<(String, Option<String>)> {
    let reason = json!({"reason": "probe"}).to_string();
    let (method, path) = (route.0.as_str(), route.1.as_str());
    let body = match (method, path) {
        ("GET", _) => None,
        ("DELETE", "/api/v1/session") => None,
        ("PUT", "/api/v1/cluster/layout/staged") => {
            Some(json!({"sets": [["n1"]], "reason": "probe"}).to_string())
        }
        ("POST", "/api/v1/cluster/layout/apply") => {
            Some(json!({"id": "probe", "reason": "probe"}).to_string())
        }
        ("POST", "/api/v1/cluster/members/approve" | "/api/v1/cluster/members/remove") => {
            Some(json!({"node": "n1", "reason": "probe"}).to_string())
        }
        ("POST", "/api/v1/cluster/members/token") => {
            Some(json!({"node": "n9", "lifetime_secs": 600, "reason": "probe"}).to_string())
        }
        ("POST", "/api/v1/encryption/unseal" | "/api/v1/encryption/rotate") => {
            Some(json!({"passphrase": "probe-pass", "reason": "probe"}).to_string())
        }
        ("POST", "/api/v1/encryption/keys") => Some(
            json!({"passphrase": "probe-pass", "name": "probe-key", "reason": "probe"}).to_string(),
        ),
        ("POST", "/api/v1/encryption/destroy") => Some(
            json!({"passphrase": "probe-pass", "key": "probe-key", "reason": "probe"}).to_string(),
        ),
        ("POST", "/api/v1/regions/failover") => Some(
            json!({"region": "eu-west-1", "promoted": "eu-west-2", "reason": "probe"}).to_string(),
        ),
        ("POST", "/api/v1/regions/restore") => {
            Some(json!({"region": "eu-west-1", "reason": "probe"}).to_string())
        }
        ("POST", "/api/v1/buckets") => {
            Some(json!({"name": format!("omega-made-{who}"), "reason": "probe"}).to_string())
        }
        ("PUT", "/api/v1/buckets/{bucket}/quota") => {
            Some(json!({"max_bytes": 1, "reason": "probe"}).to_string())
        }
        ("PUT", "/api/v1/buckets/{bucket}/acknowledgement") => {
            Some(json!({"level": "majority", "reason": "probe"}).to_string())
        }
        ("PUT", "/api/v1/buckets/{bucket}/encryption") => {
            Some(json!({"algorithm": "AES256", "reason": "probe"}).to_string())
        }
        ("PUT", "/api/v1/buckets/{bucket}/media") => {
            Some(json!({"rules": [], "reason": "probe"}).to_string())
        }
        ("POST", "/api/v1/buckets/{bucket}/upload-keys") => {
            Some(json!({"key": "x.txt", "expires_in": 600, "reason": "probe"}).to_string())
        }
        ("DELETE", "/api/v1/buckets/{bucket}/object") => {
            Some(json!({"etag": etag, "reason": "probe"}).to_string())
        }
        ("POST", "/api/v1/spaces") => {
            Some(json!({"name": "probe-space", "reason": "probe"}).to_string())
        }
        ("POST", "/api/v1/users") => Some(
            json!({"name": "probe-user", "space": "alpha", "role": "member", "reason": "probe"})
                .to_string(),
        ),
        ("PUT", "/api/v1/users/{user}/disabled") => {
            Some(json!({"disabled": true, "reason": "probe"}).to_string())
        }
        ("PUT", "/api/v1/users/{user}/identity") => {
            Some(json!({"subject": "probe", "reason": "probe"}).to_string())
        }
        ("POST", "/api/v1/users/{user}/keys") => Some(
            json!({"reason": "probe", "idempotency_key": "probe-idempotency-0001"}).to_string(),
        ),
        (
            "PUT",
            "/api/v1/users/{user}/grants/{bucket}"
            | "/api/v1/spaces/{space}/groups/{group}/grants/{bucket}",
        ) => Some(json!({"read": true, "write": true, "reason": "probe"}).to_string()),
        ("POST", "/api/v1/spaces/{space}/groups") => {
            Some(json!({"name": "probe-group", "reason": "probe"}).to_string())
        }
        ("POST" | "PUT" | "DELETE", _) => Some(reason),
        _ => return None,
    };
    let mut concrete = path
        .replace("{bucket}", "alpha-files")
        .replace("{user}", "ann")
        .replace("{key}", key)
        .replace("{space}", "alpha")
        .replace("{group}", "crew");
    if path.ends_with("/object") {
        concrete.push_str("?key=doc.txt");
    } else if path.ends_with("/object/content") {
        concrete.push_str("?key=doc.txt&reason=probe");
    }
    Some((concrete, body))
}

/// What is wrong with `seen` as an outsider's answer to `route`, if anything.
fn wrong(route: &(String, String), seen: &Seen) -> Option<String> {
    let own = OWN_SCOPE
        .iter()
        .any(|(method, path)| route.0 == *method && route.1 == *path);
    let body = String::from_utf8_lossy(&seen.bytes);
    match seen.status {
        403 | 404 => None,
        200..=299 if own => MARKERS
            .iter()
            .find(|marker| body.contains(*marker))
            .map(|marker| format!("names {marker}: {body}")),
        status => Some(format!("answered {status}: {body}")),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn every_protected_console_route_refuses_a_stranger_and_another_spaces_users() {
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    user_with_key(storage, "boss", "alpha", Role::SpaceAdmin, false).await;
    let eve: Key = user_with_key(storage, "eve", "omega", Role::SpaceAdmin, false).await;
    let mal: Key = user_with_key(storage, "mal", "omega", Role::Member, true).await;
    assert_eq!(
        crate::call_as(
            &state,
            ann.pair(),
            "PUT",
            "/alpha-files",
            vec![],
            vec![],
            b""
        )
        .await
        .status,
        200
    );
    let put = crate::call_as(
        &state,
        ann.pair(),
        "PUT",
        "/alpha-files/doc.txt",
        vec![],
        vec![],
        b"plan",
    )
    .await;
    let etag = put
        .headers
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .expect("an ETag")
        .trim_matches('"')
        .to_owned();
    // A usage measurement exists, so the routes that answer with one have something of alpha they could leak.
    assert!(
        storage
            .usage()
            .refresh("probe")
            .await
            .expect("measured")
            .is_some()
    );
    let root = sign_in(&console).await;
    let group = send(
        &console,
        "POST",
        "/api/v1/spaces/alpha/groups",
        Some(&root),
        Some(&json!({"name": "crew", "reason": "probe"}).to_string()),
    )
    .await;
    assert!(
        group.status < 300,
        "{}",
        String::from_utf8_lossy(&group.bytes)
    );

    let mut routes = protected_routes();
    // The probe sources its list from the router; a few routes it must have found, whatever else is added.
    for known in [
        ("DELETE", "/api/v1/users/{user}/keys/{key}"),
        (
            "PUT",
            "/api/v1/spaces/{space}/groups/{group}/grants/{bucket}",
        ),
        ("GET", "/api/v1/buckets/{bucket}/object/content"),
        ("POST", "/api/v1/cluster/layout/apply"),
    ] {
        assert!(
            routes.iter().any(|(m, p)| m == known.0 && p == known.1),
            "{known:?} not read from the router"
        );
    }
    // Signing out ends the probe's own session, so it goes last.
    routes.sort_by_key(|(method, path)| method == "DELETE" && path == "/api/v1/session");
    let source = routes.len();

    let mut problems = Vec::new();
    for route in &routes {
        let (path, body) = probe(route, &ann.id, &etag, 0)
            .unwrap_or_else(|| panic!("{route:?} has no probe request: add one"));
        let stranger = send(&console, &route.0, &path, None, body.as_deref()).await;
        if stranger.status != 401 {
            problems.push(format!("{route:?} without a session: {}", stranger.status));
        }
    }
    let mut verified = 0_usize;
    for (who, outsider) in [&eve, &mal].into_iter().enumerate() {
        let token = sign_in_as(&console, outsider).await.expect("signed in");
        for route in &routes {
            let (path, body) = probe(route, &ann.id, &etag, who).expect("probed above");
            let seen = send(&console, &route.0, &path, Some(&token), body.as_deref()).await;
            match wrong(route, &seen) {
                Some(problem) => problems.push(format!("{route:?} as {}: {problem}", outsider.id)),
                None => verified = verified.saturating_add(1),
            }
        }
    }
    // Nothing of space alpha moved: its bucket, object, group and user are as they were.
    assert_eq!(
        crate::call_as(
            &state,
            ann.pair(),
            "GET",
            "/alpha-files/doc.txt",
            vec![],
            vec![],
            b""
        )
        .await
        .bytes,
        b"plan"
    );
    println!(
        "paths: source={source} | enforced={} | verified={verified}",
        source.saturating_mul(2)
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
