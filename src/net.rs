//! Reading what the online services answer.
//!
//! ureq counts its own limit (10 MiB) on the bytes that come over the wire, BELOW the gzip decoder, and gzip is on
//! by default. So a small compressed answer can still unpack into something far larger than the limit. Every answer
//! is read through here instead, with a limit on the bytes AFTER decoding.

use std::io::Read;

use serde_json::Value;

/// The most decoded bytes read from one answer. The real ones are a few KB to a few hundred KB.
pub const MAX_ANSWER_BYTES: u64 = 2 * 1024 * 1024;

/// `body.read_limited_json()` is `read_json::<Value>()` with a limit on the decoded size.
pub trait BodyExt {
    fn read_limited_json(&mut self) -> Result<Value, ureq::Error>;
}

impl BodyExt for ureq::Body {
    fn read_limited_json(&mut self) -> Result<Value, ureq::Error> {
        parse_limited(self.as_reader(), MAX_ANSWER_BYTES)
    }
}

/// JSON from `reader`, which is read for at most `limit` bytes: an answer that is longer is cut, so it no longer
/// parses (a JSON error, which says nothing about the service being down).
fn parse_limited(reader: impl Read, limit: u64) -> Result<Value, ureq::Error> {
    let mut bytes = Vec::new();
    reader.take(limit).read_to_end(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_normal_answer_is_read() {
        let v = parse_limited(Cursor::new(br#"{"a":[1,2,3]}"#.to_vec()), 1024).unwrap();
        assert_eq!(v["a"].as_array().map(Vec::len), Some(3));
        assert!(parse_limited(Cursor::new(Vec::new()), 1024).is_err(), "nothing is no JSON");
    }

    /// A small answer that unpacks to something endless (what a gzip bomb does) is read only up to the limit.
    #[test]
    fn an_answer_that_never_ends_is_read_only_up_to_the_limit() {
        struct Counting<'a, R: Read>(R, &'a mut u64);
        impl<R: Read> Read for Counting<'_, R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.0.read(buf)?;
                *self.1 += n as u64;
                Ok(n)
            }
        }
        let mut seen = 0u64;
        let err = parse_limited(Counting(std::io::repeat(b'x'), &mut seen), 64 * 1024).unwrap_err();
        assert!(matches!(err, ureq::Error::Json(_)), "cut text is a JSON error, not a connection error: {err}");
        assert!(seen <= 64 * 1024, "read {seen} bytes");
    }

    /// Run by hand against a local server (`SKY_NET_TEST_URL=http://127.0.0.1:PORT cargo test a_gzip_bomb -- --ignored`)
    /// that answers `/ok` with a small gzip JSON and `/bomb` with a gzip file of about 1 MB that unpacks to 600 MiB.
    #[test]
    #[ignore = "needs a local test server"]
    fn a_gzip_bomb_from_a_real_server_is_cut() {
        let base = std::env::var("SKY_NET_TEST_URL").expect("SKY_NET_TEST_URL");
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(30))).build().into();
        let ok = agent.get(format!("{base}/ok")).call().unwrap().body_mut().read_limited_json().unwrap();
        assert_eq!(ok["hello"], "gzip", "a gzip answer is decoded before it is read");
        let started = std::time::Instant::now();
        let err = agent.get(format!("{base}/bomb")).call().and_then(|mut r| r.body_mut().read_limited_json()).unwrap_err();
        assert!(matches!(err, ureq::Error::Json(_)), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    #[test]
    fn an_answer_just_over_the_limit_is_cut_and_one_at_the_limit_is_whole() {
        let body = format!("[{}0]", "0,".repeat(40));
        let whole = body.len() as u64;
        assert!(parse_limited(Cursor::new(body.clone().into_bytes()), whole).is_ok());
        assert!(parse_limited(Cursor::new(body.into_bytes()), whole - 1).is_err());
    }
}
