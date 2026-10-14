//! Bucket lifecycle configuration against a real node (S3 ref 08 M7): stored and read back, absent until set, removed
//! by DeleteBucketLifecycle, and refused — not accepted and ignored — where it asks for what this server does not do.

use crate::multipart::{text, texts};
use crate::{IGNORED, Seen, call, fresh};

const RULES: &str = "<LifecycleConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
<Rule><ID>logs</ID><Filter><Prefix>logs/</Prefix></Filter><Status>Enabled</Status>\
<Expiration><Days>30</Days></Expiration>\
<NoncurrentVersionExpiration><NoncurrentDays>7</NoncurrentDays><NewerNoncurrentVersions>2</NewerNoncurrentVersions>\
</NoncurrentVersionExpiration></Rule>\
<Rule><ID>uploads</ID><Filter></Filter><Status>Disabled</Status>\
<AbortIncompleteMultipartUpload><DaysAfterInitiation>3</DaysAfterInitiation></AbortIncompleteMultipartUpload></Rule>\
</LifecycleConfiguration>";

async fn lifecycle(state: &tessari_s3_api::ApiState, method: &str, body: &str) -> Seen {
    call(
        state,
        method,
        "/cycle",
        vec![("lifecycle", None)],
        body.as_bytes(),
    )
    .await
}

fn rule(body: &str, actions: &str) -> String {
    format!(
        "<LifecycleConfiguration><Rule><ID>r</ID><Filter>{body}</Filter><Status>Enabled</Status>{actions}</Rule>\
         </LifecycleConfiguration>"
    )
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_lifecycle_configuration_is_stored_read_back_and_deleted() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cycle", vec![], b"").await.status, 200);
    let none = lifecycle(&state, "GET", "").await;
    assert_eq!(none.status, 404, "{}", none.body);
    assert_eq!(none.code.as_deref(), Some("NoSuchLifecycleConfiguration"));
    let put = lifecycle(&state, "PUT", RULES).await;
    assert_eq!(put.status, 200, "{}", put.body);
    let got = lifecycle(&state, "GET", "").await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(
        texts(&got.body, "Rule", "ID"),
        vec!["logs".to_owned(), "uploads".to_owned()]
    );
    assert_eq!(
        texts(&got.body, "Rule", "Status"),
        vec!["Enabled".to_owned(), "Disabled".to_owned()]
    );
    assert_eq!(text(&got.body, "Prefix").as_deref(), Some("logs/"));
    assert_eq!(text(&got.body, "Days").as_deref(), Some("30"));
    assert_eq!(text(&got.body, "NoncurrentDays").as_deref(), Some("7"));
    assert_eq!(
        text(&got.body, "NewerNoncurrentVersions").as_deref(),
        Some("2")
    );
    assert_eq!(text(&got.body, "DaysAfterInitiation").as_deref(), Some("3"));
    let deleted = lifecycle(&state, "DELETE", "").await;
    assert_eq!(deleted.status, 204, "{}", deleted.body);
    assert_eq!(lifecycle(&state, "GET", "").await.status, 404);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn what_this_server_does_not_do_is_refused_and_a_bad_rule_is_named() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cycle", vec![], b"").await.status, 200);
    let refusals = [
        (
            rule(
                "",
                "<Transition><Days>30</Days><StorageClass>GLACIER</StorageClass></Transition>",
            ),
            501,
            "NotImplemented",
        ),
        (
            rule(
                "<Tag><Key>k</Key><Value>v</Value></Tag>",
                "<Expiration><Days>1</Days></Expiration>",
            ),
            501,
            "NotImplemented",
        ),
        (
            rule("", "<Expiration><Days>0</Days></Expiration>"),
            400,
            "InvalidArgument",
        ),
        (
            rule("", "<Expiration><Days>x</Days></Expiration>"),
            400,
            "MalformedXML",
        ),
        ("<Nope/>".to_owned(), 400, "MalformedXML"),
    ];
    for (document, status, code) in refusals {
        let seen = lifecycle(&state, "PUT", &document).await;
        assert_eq!(seen.status, status, "{document}: {}", seen.body);
        assert_eq!(seen.code.as_deref(), Some(code), "{document}");
    }
    assert_eq!(
        lifecycle(&state, "GET", "").await.status,
        404,
        "nothing refused was stored"
    );
}
