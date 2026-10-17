use super::{Metrics, Served, size_class};

#[test]
fn a_size_lands_in_its_class_at_each_boundary() {
    for (bytes, class) in [
        (0, "lt_64KiB"),
        ((64 << 10) - 1, "lt_64KiB"),
        (64 << 10, "lt_1MiB"),
        (1 << 20, "lt_16MiB"),
        (16 << 20, "lt_128MiB"),
        (128 << 20, "ge_128MiB"),
        (u64::MAX, "ge_128MiB"),
    ] {
        assert_eq!(size_class(bytes), class, "{bytes}");
    }
}

#[test]
fn a_served_request_is_counted_and_timed_under_its_labels() {
    let metrics = Metrics::new();
    let put = Served {
        operation: "PutObject",
        status: 200,
        code: "None",
        bytes: 10,
        seconds: 0.004,
    };
    metrics.served(&put);
    metrics.served(&put);
    metrics.served(&Served {
        operation: "GetObject",
        status: 404,
        code: "NoSuchKey",
        bytes: 0,
        seconds: 0.001,
    });
    metrics.shed();
    let text = metrics.encode().expect("encoded");
    for line in [
        r#"s3_requests_total{operation="PutObject",status="200",code="None"} 2"#,
        r#"s3_requests_total{operation="GetObject",status="404",code="NoSuchKey"} 1"#,
        r#"s3_request_duration_seconds_count{operation="PutObject",size="lt_64KiB"} 2"#,
        "s3_requests_shed_total 1",
    ] {
        assert!(text.lines().any(|seen| seen == line), "{line} in\n{text}");
    }
}
