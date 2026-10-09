//! Serde representation of a [`Duration`] as a whole number of milliseconds.
//!
//! Every serialised `Duration` in ChillGuard uses this module through
//! `#[serde(with = "crate::duration_millis")]`, so a duration is one unsigned
//! integer in every format: `1800000` in JSON, a varint in Postcard. It is
//! the unit [`Timestamp`](crate::risk::Timestamp) already uses.
//!
//! The representation is exact or it is refused. A duration with a part
//! smaller than a millisecond, or longer than `u64::MAX` milliseconds, fails
//! to serialise; it is never rounded. Durations that come off the wire, and
//! those computed from timestamps, are whole milliseconds by construction.

use core::time::Duration;

use serde::{Deserialize, Deserializer, Serializer, ser};

const NANOS_PER_MILLI: u32 = 1_000_000;

/// Writes `duration` as an unsigned integer of milliseconds.
///
/// # Errors
///
/// Fails when `duration` is not a whole number of milliseconds or does not
/// fit in a `u64` of milliseconds, and when the serializer fails.
pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
    if !duration.subsec_nanos().is_multiple_of(NANOS_PER_MILLI) {
        return Err(ser::Error::custom(
            "duration is not a whole number of milliseconds",
        ));
    }
    let millis = u64::try_from(duration.as_millis())
        .map_err(|_| ser::Error::custom("duration does not fit in 64 bits of milliseconds"))?;
    serializer.serialize_u64(millis)
}

/// Reads a duration from an unsigned integer of milliseconds.
///
/// # Errors
///
/// Fails when the input is not an unsigned integer that fits in a `u64`.
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    u64::deserialize(deserializer).map(Duration::from_millis)
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use core::time::Duration;

    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        #[serde(with = "super")]
        d: Duration,
    }

    fn to_json(d: Duration) -> Result<String, serde_json::Error> {
        serde_json::to_string(&Holder { d })
    }

    fn from_json(text: &str) -> Result<Duration, serde_json::Error> {
        serde_json::from_str::<Holder>(text).map(|holder| holder.d)
    }

    #[test]
    fn serializes_as_an_integer_of_milliseconds() {
        struct Case {
            duration: Duration,
            json: &'static str,
        }
        let cases = [
            Case {
                duration: Duration::ZERO,
                json: r#"{"d":0}"#,
            },
            Case {
                duration: Duration::from_millis(1),
                json: r#"{"d":1}"#,
            },
            Case {
                duration: Duration::from_millis(1_500),
                json: r#"{"d":1500}"#,
            },
            Case {
                duration: Duration::from_mins(30),
                json: r#"{"d":1800000}"#,
            },
            Case {
                duration: Duration::from_hours(24),
                json: r#"{"d":86400000}"#,
            },
            Case {
                duration: Duration::from_millis(u64::MAX),
                json: r#"{"d":18446744073709551615}"#,
            },
        ];
        for case in cases {
            let text = to_json(case.duration).expect("serialises");
            assert_eq!(text, case.json);
            assert_eq!(from_json(&text).expect("deserialises"), case.duration);
        }
    }

    #[test]
    fn round_trips_through_postcard() {
        for duration in [
            Duration::ZERO,
            Duration::from_millis(1),
            Duration::from_mins(5),
            Duration::from_millis(u64::MAX),
        ] {
            let bytes = postcard::to_allocvec(&Holder { d: duration }).expect("serialises");
            let back: Holder = postcard::from_bytes(&bytes).expect("deserialises");
            assert_eq!(back.d, duration);
        }
        // One varint, not the two fields of serde's own `Duration` format.
        let bytes = postcard::to_allocvec(&Holder {
            d: Duration::from_millis(100),
        })
        .expect("serialises");
        assert_eq!(bytes, [100]);
    }

    #[test]
    fn sub_millisecond_durations_are_refused_not_rounded() {
        for duration in [
            Duration::from_nanos(1),
            Duration::from_micros(999),
            Duration::from_micros(1_500),
            Duration::new(30, 1),
            Duration::from_secs_f64(0.000_1),
        ] {
            let error = to_json(duration).expect_err("must not serialise");
            assert!(
                alloc::format!("{error}").contains("whole number of milliseconds"),
                "{duration:?}: {error}"
            );
            assert!(postcard::to_allocvec(&Holder { d: duration }).is_err());
        }
    }

    #[test]
    fn durations_beyond_u64_milliseconds_are_refused() {
        let one_too_many = Duration::from_millis(u64::MAX) + Duration::from_millis(1);
        for duration in [one_too_many, Duration::from_secs(u64::MAX)] {
            let error = to_json(duration).expect_err("must not serialise");
            assert!(
                alloc::format!("{error}").contains("64 bits"),
                "{duration:?}: {error}"
            );
        }
    }

    #[test]
    fn anything_but_an_unsigned_integer_is_rejected() {
        for text in [
            // serde's own `Duration` shape, which this module replaces.
            r#"{"d":{"secs":1800,"nanos":0}}"#,
            r#"{"d":-1}"#,
            r#"{"d":1.5}"#,
            r#"{"d":1800.0}"#,
            r#"{"d":"1800"}"#,
            r#"{"d":null}"#,
            r#"{"d":true}"#,
            r#"{"d":[1800,0]}"#,
            r#"{"d":18446744073709551616}"#,
            r"{}",
        ] {
            assert!(from_json(text).is_err(), "{text} must be rejected");
        }
    }
}
