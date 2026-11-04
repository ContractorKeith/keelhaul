#![allow(clippy::unwrap_used, reason = "test assertions and fixtures may panic")]
mod common;
#[path = "common/docker.rs"]
mod docker;
#[path = "common/s3_proxy.rs"]
mod s3_proxy;
use common::backend_suite::{name, run_suite};
use keelhaul::backend::{
    Backend, BackendError,
    s3::{S3Backend, S3Config, S3Location},
};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use std::{sync::atomic::Ordering, time::Duration};

struct Fixture {
    bucket: Bucket,
    credentials: Credentials,
}
impl Fixture {
    fn new() -> Self {
        docker::require_s3();
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let bucket = Bucket::new(
            "http://127.0.0.1:7070".parse().unwrap(),
            UrlStyle::Path,
            format!("keelhaul-{}", hex::encode(nonce)),
            "us-east-1",
        )
        .unwrap();
        let fixture = Self {
            bucket,
            credentials: Credentials::new("keelhaul-test", "keelhaul-test-password"),
        };
        ureq::put(
            fixture
                .bucket
                .create_bucket(&fixture.credentials)
                .sign(Duration::from_secs(60))
                .as_str(),
        )
        .send_empty()
        .expect("create unique S3 test bucket");
        fixture
    }
    fn backend(&self, endpoint: &str, prefix: &str) -> S3Backend {
        S3Backend::new({
            let mut config = S3Config::new(
                S3Location::parse(&format!("s3:{endpoint}/{}/{prefix}", self.bucket.name()))
                    .unwrap(),
                self.credentials.clone(),
            );
            config.timeout = Duration::from_secs(10);
            config.verbose = true;
            config
        })
        .unwrap()
    }
    fn put(&self, key: &str, bytes: &[u8]) {
        ureq::put(
            self.bucket
                .put_object(Some(&self.credentials), key)
                .sign(Duration::from_secs(60))
                .as_str(),
        )
        .send(bytes)
        .unwrap();
    }
    fn clean(&self) {
        loop {
            let mut response = ureq::get(
                self.bucket
                    .list_objects_v2(Some(&self.credentials))
                    .sign(Duration::from_secs(60))
                    .as_str(),
            )
            .call()
            .unwrap();
            let xml = response.body_mut().read_to_string().unwrap();
            let page = rusty_s3::actions::ListObjectsV2::parse_response(&xml).unwrap();
            if page.contents.is_empty() {
                break;
            }
            for object in page.contents {
                ureq::delete(
                    self.bucket
                        .delete_object(Some(&self.credentials), &object.key)
                        .sign(Duration::from_secs(60))
                        .as_str(),
                )
                .call()
                .unwrap();
            }
        }
        ureq::delete(
            self.bucket
                .delete_bucket(&self.credentials)
                .sign(Duration::from_secs(60))
                .as_str(),
        )
        .call()
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Clean test buckets even after an assertion fails, without double-panicking.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.clean()));
        if result.is_err() && !std::thread::panicking() {
            panic!("S3 test bucket cleanup failed");
        }
    }
}

#[test]
fn s3_conformance() {
    if !docker::enabled() {
        return;
    }
    let fixture = Fixture::new();
    let backend = fixture.backend("http://127.0.0.1:7070", "repo/nested");
    run_suite(&backend, true, |raw, bytes| {
        fixture.put(&format!("repo/nested/{raw}"), bytes)
    });
    // A sibling repository in the same bucket must remain invisible and intact.
    fixture.put("repo/nested-sibling/config", b"sibling");
    assert!(
        backend
            .list(keelhaul::backend::Kind::Config)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn s3_retries_503_and_fails_fast_on_403() {
    if !docker::enabled() {
        return;
    }
    let fixture = Fixture::new();
    fixture.put("config", b"value");
    let proxy = s3_proxy::Proxy::start(3, 503);
    let backend = fixture.backend(&proxy.endpoint, "");
    assert_eq!(backend.read(&name("config")).unwrap(), b"value");
    assert_eq!(proxy.requests.load(Ordering::Acquire), 4);
    let forbidden = s3_proxy::Proxy::start(usize::MAX, 403);
    let backend = fixture.backend(&forbidden.endpoint, "");
    let error = backend.read(&name("config")).unwrap_err();
    assert!(matches!(error, BackendError::Io { .. }));
    assert!(error.to_string().contains("403"));
    assert_eq!(forbidden.requests.load(Ordering::Acquire), 1);
    assert_eq!(keelhaul::exit::code(Err(keelhaul::exit::Failure::Error)), 1);
    // Also exercise a real signature rejection from versitygw.
    let backend = S3Backend::new({
        let mut config = S3Config::new(
            S3Location::parse(&format!(
                "s3:http://127.0.0.1:7070/{}",
                fixture.bucket.name()
            ))
            .unwrap(),
            Credentials::new("keelhaul-test", "wrong-secret"),
        );
        config.timeout = Duration::from_secs(5);
        config
    })
    .unwrap();
    let error = backend.read(&name("config")).unwrap_err();
    assert!(error.to_string().contains("403"));
    for secret in ["wrong-secret", "keelhaul-test", "Signature"] {
        assert!(!format!("{error:?}").contains(secret));
    }
}

#[test]
fn s3_capture_pagination() {
    if !docker::enabled() {
        return;
    }
    let fixture = Fixture::new();
    for id in 0..1003 {
        fixture.put(&format!("index/{id:064x}"), b"x");
    }
    let mut token = None;
    let mut total = 0;
    let mut pages = 0;
    loop {
        let mut action = fixture.bucket.list_objects_v2(Some(&fixture.credentials));
        action.with_prefix("index/");
        if let Some(token) = &token {
            action.with_continuation_token(token);
        }
        let mut response = ureq::get(action.sign(Duration::from_secs(60)).as_str())
            .call()
            .unwrap();
        let xml = response.body_mut().read_to_string().unwrap();
        let page = rusty_s3::actions::ListObjectsV2::parse_response(&xml).unwrap();
        if let Ok(directory) = std::env::var("KEELHAUL_CAPTURE_S3") {
            std::fs::write(
                std::path::Path::new(&directory).join(format!("list-page-{pages}.xml")),
                &xml,
            )
            .unwrap();
        }
        total += page.contents.len();
        pages += 1;
        token = page.next_continuation_token;
        if token.is_none() {
            break;
        }
    }
    assert_eq!((total, pages), (1003, 2));
}

#[test]
fn s3_nonexistent_bucket_is_an_error_not_a_missing_object() {
    if !docker::enabled() {
        return;
    }
    docker::require_s3();
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).unwrap();
    let bucket = format!("keelhaul-missing-{}", hex::encode(nonce));
    let location = S3Location::parse(&format!("s3:http://127.0.0.1:7070/{bucket}")).unwrap();
    let backend = S3Backend::new(S3Config::new(
        location,
        Credentials::new("keelhaul-test", "keelhaul-test-password"),
    ))
    .unwrap();
    let error = backend.read(&name("config")).unwrap_err();
    assert!(matches!(error, BackendError::BucketNotFound(_)));
    assert_eq!(error.to_string(), format!("bucket not found: {bucket}"));
}
