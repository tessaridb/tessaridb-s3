//! Delivering bucket notifications to a webhook against a real node (S3 ref 11 M5): the destination's group hands out
//! each event its bucket's rules send there, the worker posts it as an S3 event record, and a refused delivery is
//! handed out again later rather than lost — a receiver on this test's own port stands in for the destination.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use tessari_s3_daemons::{Daemon, Destination, NotifyWorker};
use tessari_s3_infrastructure::WebhookClient;

use crate::notifications::{CONFIGURATION, notification};
use crate::{IGNORED, call, fresh};

/// What the receiver saw: each delivery's key and event name, and the status it answers with.
#[derive(Clone, Default)]
struct Receiver {
    seen: Arc<Mutex<Vec<serde_json::Value>>>,
    status: Arc<AtomicU16>,
}

async fn receive(State(receiver): State<Receiver>, body: axum::body::Bytes) -> StatusCode {
    let record: serde_json::Value = serde_json::from_slice(&body).expect("a JSON body");
    receiver
        .seen
        .lock()
        .expect("not poisoned")
        .push(record["Records"][0].clone());
    StatusCode::from_u16(receiver.status.load(Ordering::SeqCst)).expect("a status")
}

/// A receiver on an ephemeral port; its URL.
async fn listening(receiver: Receiver) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let address = listener.local_addr().expect("an address");
    let app = Router::new()
        .route("/hooks", post(receive))
        .with_state(receiver);
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{address}/hooks")
}

/// Deliveries the receiver saw per key, in arrival order.
fn by_key(receiver: &Receiver) -> BTreeMap<String, Vec<String>> {
    let mut keys: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for record in receiver.seen.lock().expect("not poisoned").iter() {
        let key = record["s3"]["object"]["key"]
            .as_str()
            .expect("a key")
            .to_owned();
        let name = record["eventName"].as_str().expect("a name").to_owned();
        keys.entry(key).or_default().push(name);
    }
    keys
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn matching_events_reach_the_webhook_once_and_a_refused_one_is_retried() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/hooked", vec![], b"").await.status,
        200
    );
    assert_eq!(
        notification(&state, "PUT", "hooked", CONFIGURATION)
            .await
            .status,
        200
    );
    // Written before the destination's group exists: a new group starts at the oldest message, so it is delivered.
    call(&state, "PUT", "/hooked/logs/early.txt", vec![], b"early").await;
    let receiver = Receiver::default();
    receiver.status.store(200, Ordering::SeqCst);
    let url = listening(receiver.clone()).await;
    let notifications = state.storage().notifications();
    notifications
        .define(["hooks"])
        .await
        .expect("group defined");
    let worker = NotifyWorker::new(
        notifications,
        "hooks".to_owned(),
        Destination::Webhook {
            url,
            client: WebhookClient::new().expect("a client"),
        },
        "us-east-1".to_owned(),
    );
    call(&state, "PUT", "/hooked/logs/a.txt", vec![], b"a").await;
    // No rule names this key: settled without a delivery.
    call(&state, "PUT", "/hooked/other.bin", vec![], b"b").await;
    worker.run_once().await.expect("a run");
    let first = by_key(&receiver);
    assert_eq!(
        first,
        BTreeMap::from([
            (
                "logs/a.txt".to_owned(),
                vec!["ObjectCreated:Put".to_owned()]
            ),
            (
                "logs/early.txt".to_owned(),
                vec!["ObjectCreated:Put".to_owned()]
            ),
        ])
    );
    worker.run_once().await.expect("a second run");
    assert_eq!(
        by_key(&receiver),
        first,
        "a delivered event is not delivered again"
    );

    // The receiver refuses: the event is handed out again after the retry wait, and delivered then.
    receiver.status.store(503, Ordering::SeqCst);
    call(&state, "PUT", "/hooked/logs/b.txt", vec![], b"b").await;
    worker.run_once().await.expect("a refused run");
    receiver.status.store(200, Ordering::SeqCst);
    let mut attempts = 0;
    for _ in 0..40 {
        worker.run_once().await.expect("a retrying run");
        attempts = by_key(&receiver).get("logs/b.txt").map_or(0, Vec::len);
        if attempts == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(attempts, 2, "refused once, then delivered");
    worker.run_once().await.expect("a settled run");
    assert_eq!(
        by_key(&receiver).get("logs/b.txt").map(Vec::len),
        Some(2),
        "delivered, then settled"
    );
}

/// The Kafka broker the Kafka test produces to (`TESSARIDB_S3_TEST_KAFKA`, `host:port`).
fn kafka_broker() -> String {
    std::env::var("TESSARIDB_S3_TEST_KAFKA")
        .expect("TESSARIDB_S3_TEST_KAFKA is required for this test")
}

/// Every record on `topic`, as (record key, event name) pairs, sorted.
async fn consumed(broker: &str, topic: &str) -> Vec<(String, String)> {
    use rskafka::client::ClientBuilder;
    use rskafka::client::partition::UnknownTopicHandling;
    let client = ClientBuilder::new(vec![broker.to_owned()])
        .build()
        .await
        .expect("a client");
    let partitions = client
        .list_topics()
        .await
        .expect("topics")
        .into_iter()
        .find(|listed| listed.name == topic)
        .expect("the topic")
        .partitions;
    let mut seen = Vec::new();
    for partition in partitions {
        let reader = client
            .partition_client(topic.to_owned(), partition, UnknownTopicHandling::Retry)
            .await
            .expect("a partition");
        let (records, _) = reader
            .fetch_records(0, 1..1_000_000, 1_000)
            .await
            .expect("records");
        for record in records {
            let key = String::from_utf8(record.record.key.expect("a key")).expect("text");
            let value: serde_json::Value =
                serde_json::from_slice(&record.record.value.expect("a value")).expect("JSON");
            let name = value["Records"][0]["eventName"]
                .as_str()
                .expect("a name")
                .to_owned();
            seen.push((key, name));
        }
    }
    seen.sort();
    seen
}

#[tokio::test]
#[ignore = "needs a TessariDB node and a Kafka broker: set TESSARIDB_S3_TEST_META, _USER, _PASSWORD and _KAFKA"]
async fn a_kafka_destination_receives_each_matching_event_keyed_by_its_object() {
    use rskafka::client::ClientBuilder;
    use tessari_s3_infrastructure::KafkaProducer;
    use tessari_s3_infrastructure::tessaridb::Value;
    let _why = IGNORED;
    let broker = kafka_broker();
    let topic = format!("s3-events-{}", uuid::Uuid::new_v4().simple());
    ClientBuilder::new(vec![broker.clone()])
        .build()
        .await
        .expect("a client")
        .controller_client()
        .expect("the controller")
        .create_topic(topic.clone(), 3, 1, 5_000)
        .await
        .expect("topic created");
    let (state, planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/streamed", vec![], b"").await.status,
        200
    );
    // The API refuses a destination the operator did not declare, and this suite declares only a webhook; the
    // configuration is written as the API stores it.
    planter
        .run(
            &format!("UPDATE {}.buckets:'streamed' SET notify = {{ rules: [{{ id: 'to-kafka', target: 'orders', \
             events: ['s3:ObjectCreated:*', 's3:ObjectRemoved:*'], prefix: '', suffix: '' }}] }};", planter.global_database()),
            Vec::<(String, Value)>::new(),
        )
        .await
        .expect("configured");
    let notifications = state.storage().notifications();
    notifications
        .define(["orders"])
        .await
        .expect("group defined");
    let worker = NotifyWorker::new(
        notifications,
        "orders".to_owned(),
        Destination::Kafka(KafkaProducer::new(vec![broker.clone()], topic.clone())),
        "us-east-1".to_owned(),
    );
    call(&state, "PUT", "/streamed/a", vec![], b"a").await;
    call(&state, "PUT", "/streamed/b", vec![], b"b").await;
    call(&state, "DELETE", "/streamed/a", vec![], b"").await;
    worker.run_once().await.expect("a run");
    worker
        .run_once()
        .await
        .expect("a second run delivers nothing again");
    assert_eq!(
        consumed(&broker, &topic).await,
        vec![
            ("streamed/a".to_owned(), "ObjectCreated:Put".to_owned()),
            ("streamed/a".to_owned(), "ObjectRemoved:Delete".to_owned()),
            ("streamed/b".to_owned(), "ObjectCreated:Put".to_owned()),
        ]
    );
}

/// The `s3_notifications` messages, as (message name, key, event name), sorted.
async fn in_the_topic(
    planter: &tessari_s3_infrastructure::tessaridb::MetaPool,
) -> Vec<(String, String, String)> {
    use tessari_s3_infrastructure::tessaridb::{Answer, Value};
    let answers = planter
        .run(
            "SELECT * FROM s3_notifications;",
            Vec::<(String, Value)>::new(),
        )
        .await
        .expect("topic read");
    let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
        return Vec::new();
    };
    let mut seen: Vec<(String, String, String)> = records
        .into_iter()
        .map(|(id, record)| {
            let Value::Object(fields) = record else {
                panic!("a message object")
            };
            let Some(Value::Array(list)) = fields.get("Records") else {
                panic!("Records: {fields:?}")
            };
            let Some(Value::Object(first)) = list.first() else {
                panic!("one record")
            };
            let name = match first.get("eventName") {
                Some(Value::String(name)) => name.clone(),
                other => panic!("{other:?}"),
            };
            let Some(Value::Object(s3)) = first.get("s3") else {
                panic!("s3")
            };
            let Some(Value::Object(object)) = s3.get("object") else {
                panic!("object")
            };
            let key = match object.get("key") {
                Some(Value::String(key)) => key.clone(),
                other => panic!("{other:?}"),
            };
            (format!("{id:?}"), key, name)
        })
        .collect();
    seen.sort();
    seen
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn by_default_notifications_go_to_the_databases_own_topic_once_each() {
    use tessari_s3_core::notification::DATABASE_TARGET;
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/kept", vec![], b"").await.status, 200);
    // No destination is declared for this: the database's own topic is always there.
    let configuration = "<NotificationConfiguration><QueueConfiguration><Id>all</Id>\
        <Queue>arn:tessaridb:sqs:::tessaridb</Queue><Event>s3:ObjectCreated:*</Event>\
        <Event>s3:ObjectRemoved:*</Event></QueueConfiguration></NotificationConfiguration>";
    let put = notification(&state, "PUT", "kept", configuration).await;
    assert_eq!(put.status, 200, "{}", put.body);
    let notifications = state.storage().notifications();
    notifications
        .define([DATABASE_TARGET])
        .await
        .expect("group defined");
    let worker = NotifyWorker::new(
        notifications.clone(),
        DATABASE_TARGET.to_owned(),
        Destination::Topic(notifications.clone()),
        "us-east-1".to_owned(),
    );
    call(&state, "PUT", "/kept/a", vec![], b"a").await;
    call(&state, "DELETE", "/kept/a", vec![], b"").await;
    worker.run_once().await.expect("a run");
    let first = in_the_topic(&planter).await;
    assert_eq!(
        first
            .iter()
            .map(|(_, key, name)| (key.as_str(), name.as_str()))
            .collect::<Vec<_>>(),
        vec![("a", "ObjectCreated:Put"), ("a", "ObjectRemoved:Delete")]
    );
    // A redelivery of an event already appended leaves the topic as it was.
    notifications
        .publish(1, "{\"Records\":[]}")
        .await
        .expect("a repeat is not an error");
    worker.run_once().await.expect("a second run");
    assert_eq!(in_the_topic(&planter).await, first, "each event once");
}
