use super::Timestamp;

#[test]
fn an_iso8601_instant_reads_back_with_or_without_milliseconds() {
    let instant = Timestamp {
        seconds: 1_255_369_830,
        nanos: 0,
    };
    assert_eq!(
        Timestamp::parse_iso8601("2009-10-12T17:50:30Z"),
        Some(instant)
    );
    assert_eq!(
        Timestamp::parse_iso8601("2009-10-12T17:50:30.000Z"),
        Some(instant)
    );
    assert_eq!(
        Timestamp::parse_iso8601("2009-10-12T17:50:30.123456Z"),
        Some(Timestamp {
            nanos: 123_456_000,
            ..instant
        }),
        "microseconds, as Python SDKs write a datetime"
    );
    assert_eq!(
        Timestamp::parse_iso8601("2009-10-12T17:50:30.5Z"),
        Some(Timestamp {
            nanos: 500_000_000,
            ..instant
        })
    );
    assert_eq!(Timestamp::parse_iso8601("2009-10-12T17:50:30.Z"), None);
    assert_eq!(
        Timestamp::parse_iso8601("2009-10-12T17:50:30.1234567891Z"),
        None
    );
    assert_eq!(Timestamp::parse_iso8601("2009-10-12 17:50:30Z"), None);
    assert_eq!(Timestamp::parse_iso8601("2009-13-12T17:50:30Z"), None);
    assert_eq!(Timestamp::parse_iso8601("2009-10-12T17:50:30"), None);
}

#[test]
fn known_instants_render_in_both_forms() {
    let cases = [
        (
            0,
            0,
            "1970-01-01T00:00:00.000Z",
            "Thu, 01 Jan 1970 00:00:00 GMT",
        ),
        (
            1_255_369_830,
            123_456_789,
            "2009-10-12T17:50:30.123Z",
            "Mon, 12 Oct 2009 17:50:30 GMT",
        ),
        (
            951_782_400,
            0,
            "2000-02-29T00:00:00.000Z",
            "Tue, 29 Feb 2000 00:00:00 GMT",
        ),
        (
            4_102_444_799,
            999_999_999,
            "2099-12-31T23:59:59.999Z",
            "Thu, 31 Dec 2099 23:59:59 GMT",
        ),
    ];
    for (seconds, nanos, iso, http) in cases {
        let at = Timestamp { seconds, nanos };
        assert_eq!(
            (at.iso8601_millis().as_str(), at.http_date().as_str()),
            (iso, http),
            "{seconds}"
        );
        assert_eq!(
            Timestamp::parse_http_date(http),
            Some(Timestamp { seconds, nanos: 0 }),
            "{http}"
        );
    }
}

#[test]
fn only_imf_fixdate_parses() {
    let refused = [
        "",
        "yesterday",
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
        "Mon, 12 Oct 2009 25:00:00 GMT",
        "Mon, 12 Oct 2009 17:50:30 UTC",
    ];
    for text in refused {
        assert_eq!(Timestamp::parse_http_date(text), None, "{text}");
    }
}
