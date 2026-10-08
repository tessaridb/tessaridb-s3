use super::{InternalAuthError, InternalRequest, sign, verify};

const SECRET: &[u8] = b"cluster-secret-0123456789abcdef0123";
const NOW: i64 = 1_791_417_600;

fn request() -> InternalRequest<'static> {
    InternalRequest {
        method: "PUT",
        path: "/internal/v1/shards/0123456789abcdef0123456789abcdef/3?block=262144",
        date: NOW,
        node: "n1",
    }
}

#[test]
fn the_signature_is_hmac_sha256_over_the_request_line_date_and_node() {
    // Computed independently with Python's hmac module over the same string to sign.
    assert_eq!(
        sign(SECRET, &request()),
        "1396767d886bef5230dfcc05b1b3063175975bec6d806b4d6b77d5a717e9dc0a"
    );
    assert_eq!(
        verify(SECRET, &request(), &sign(SECRET, &request()), NOW),
        Ok(())
    );
}

#[test]
fn any_changed_part_or_another_secret_is_refused() {
    let good = sign(SECRET, &request());
    let changed = [
        InternalRequest {
            method: "DELETE",
            ..request()
        },
        InternalRequest {
            path: "/internal/v1/shards/0123456789abcdef0123456789abcdef/4?block=262144",
            ..request()
        },
        InternalRequest {
            date: NOW + 1,
            ..request()
        },
        InternalRequest {
            node: "n2",
            ..request()
        },
    ];
    for other in changed {
        assert_eq!(
            verify(SECRET, &other, &good, NOW),
            Err(InternalAuthError::Signature),
            "{other:?}"
        );
    }
    assert_eq!(
        verify(
            b"another-cluster-secret-0123456789ab",
            &request(),
            &good,
            NOW
        ),
        Err(InternalAuthError::Signature)
    );
    for bad in ["", "zz", &good[..63], &good.to_uppercase()] {
        assert_eq!(
            verify(SECRET, &request(), bad, NOW),
            Err(InternalAuthError::Signature),
            "{bad:?}"
        );
    }
}

#[test]
fn a_request_dated_more_than_five_minutes_away_is_refused_as_skewed() {
    let good = sign(SECRET, &request());
    assert_eq!(verify(SECRET, &request(), &good, NOW + 300), Ok(()));
    assert_eq!(verify(SECRET, &request(), &good, NOW - 300), Ok(()));
    assert_eq!(
        verify(SECRET, &request(), &good, NOW + 301),
        Err(InternalAuthError::Skew)
    );
    assert_eq!(
        verify(SECRET, &request(), &good, NOW - 301),
        Err(InternalAuthError::Skew)
    );
}
