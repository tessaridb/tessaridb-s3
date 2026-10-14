use tessari_s3_storage::notifications::{Due, Event};
use tessari_s3_types::Timestamp;

use super::body;

fn due(key: &str, size: Option<u64>) -> Due {
    Due {
        event: Event {
            position: 42,
            deliveries: 1,
            bucket: "photos".to_owned(),
            key: key.to_owned(),
            name: "s3:ObjectCreated:Put".to_owned(),
            version_id: Some("v1".to_owned()),
            size,
            etag: size.map(|_| "\"abc\"".to_owned()),
            at: Timestamp {
                seconds: 1_760_000_000,
                nanos: 0,
            },
        },
        configuration: "created".to_owned(),
    }
}

#[test]
fn a_delivery_is_one_s3_event_record() {
    let body = body(&due("red flower/ünï.jpg", Some(7)), "us-east-1").expect("encodes");
    let parsed: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
    let record = &parsed["Records"][0];
    assert_eq!(parsed["Records"].as_array().map(Vec::len), Some(1));
    assert_eq!(record["eventVersion"], "2.1");
    assert_eq!(record["eventName"], "ObjectCreated:Put");
    assert_eq!(record["awsRegion"], "us-east-1");
    assert_eq!(record["eventTime"], "2025-10-09T08:53:20.000Z");
    assert_eq!(record["s3"]["configurationId"], "created");
    assert_eq!(record["s3"]["bucket"]["name"], "photos");
    let object = &record["s3"]["object"];
    assert_eq!(object["key"], "red+flower/%C3%BCn%C3%AF.jpg");
    assert_eq!(object["size"], 7);
    assert_eq!(object["eTag"], "\"abc\"");
    assert_eq!(object["versionId"], "v1");
    assert_eq!(object["sequencer"], "000000000000002A");
}

#[test]
fn a_removal_carries_no_size_or_etag() {
    let body = body(&due("k", None), "us-east-1").expect("encodes");
    let parsed: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
    let object = &parsed["Records"][0]["s3"]["object"];
    assert!(
        object.get("size").is_none() && object.get("eTag").is_none(),
        "{object}"
    );
    assert_eq!(object["key"], "k");
}
