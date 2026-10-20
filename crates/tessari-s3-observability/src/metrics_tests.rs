use super::{Metrics, Served, size_class};
use crate::metrics_state::{Counted, RunOutcome, VaultState};

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

#[test]
fn failed_verifications_are_counted_up_to_the_storage_total() {
    let metrics = Metrics::new();
    metrics.verification_failures(3);
    metrics.verification_failures(5);
    metrics.verification_failures(5);
    let text = metrics.encode().expect("encoded");
    assert!(
        text.lines()
            .any(|line| line == "s3_data_verification_failures_total 5"),
        "{text}"
    );
}

#[test]
fn the_state_series_carry_the_values_last_set() {
    let metrics = Metrics::new();
    let state = metrics.state();
    state.meta_probe(0.25);
    state.vault(VaultState::Sealed);
    state.data_space(1000, 400);
    state.usage(Counted {
        buckets: 2,
        objects: 7,
        bytes: 900,
        taken_secs: 1_800_000_000,
    });
    state.uploads(3);
    state.run("heal", RunOutcome::Succeeded);
    state.run("heal", RunOutcome::Succeeded);
    state.run("heal", RunOutcome::TimedOut);
    let text = metrics.encode().expect("encoded");
    for line in [
        format!(
            "s3_build_info{{version=\"{}\"}} 1",
            env!("CARGO_PKG_VERSION")
        ),
        "s3_meta_probe_seconds{} 0.25".to_owned(),
        "s3_keys_vault{state=\"sealed\"} 1".to_owned(),
        "s3_keys_vault{state=\"unsealed\"} 0".to_owned(),
        "s3_keys_vault{state=\"uninitialised\"} 0".to_owned(),
        "s3_data_capacity_bytes{} 1000".to_owned(),
        "s3_data_free_bytes{} 400".to_owned(),
        "s3_buckets{} 2".to_owned(),
        "s3_objects{} 7".to_owned(),
        "s3_stored_bytes{} 900".to_owned(),
        "s3_usage_taken_seconds{} 1800000000".to_owned(),
        "s3_uploads_open{} 3".to_owned(),
        "s3_daemon_runs_total{daemon=\"heal\",outcome=\"succeeded\"} 2".to_owned(),
        "s3_daemon_runs_total{daemon=\"heal\",outcome=\"timed_out\"} 1".to_owned(),
    ] {
        assert!(text.lines().any(|seen| seen == line), "{line} in\n{text}");
    }
    let started = text
        .lines()
        .find_map(|line| line.strip_prefix("s3_start_time_seconds "))
        .and_then(|value| value.parse::<i64>().ok())
        .expect("the start time is exported");
    assert!(started > 1_700_000_000, "{started}");
}

#[test]
fn a_state_gauge_never_set_has_no_sample() {
    let text = Metrics::new().encode().expect("encoded");
    for name in [
        "s3_meta_probe_seconds{} ",
        "s3_data_free_bytes{} ",
        "s3_uploads_open{} ",
        "s3_buckets{} ",
    ] {
        assert!(
            !text.lines().any(|line| line.starts_with(name)),
            "{name} in\n{text}"
        );
    }
}
