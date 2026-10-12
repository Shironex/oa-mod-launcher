//! The network rules against a loopback server: a signed file is fetched and
//! checked, a download is hashed, and limits and redirects are enforced.

mod support;

use oa_mod_launcher_core::{CoreError, net, signature};
use sha2::{Digest, Sha256};
use support::{Reply, TestServer};
use url::Url;

fn client() -> reqwest::Client {
    net::client("0.0.0").expect("a client")
}

fn url(server: &TestServer, path: &str) -> Url {
    net::checked_url(&server.url(path)).expect("a loopback URL is allowed")
}

#[tokio::test]
async fn a_signed_file_is_fetched_and_verified_as_the_bytes_that_arrived() {
    let server = TestServer::start().await;
    let body = br#"{"schema":1,"version":"0.1.0"}"#;
    server.route_signed("/manifest.json", body);
    let manifest = url(&server, "/manifest.json");

    let bytes = net::fetch_bytes(&client(), &manifest, "read the manifest")
        .await
        .expect("the manifest arrives");
    let sig = net::fetch_bytes(
        &client(),
        &net::signature_url(&manifest),
        "read the signature",
    )
    .await
    .expect("the signature arrives");
    let sig = String::from_utf8(sig).expect("a signature is text");

    assert_eq!(bytes, body);
    assert!(signature::verify(&bytes, &sig, &[server.public_key()]).is_ok());
    assert!(matches!(
        signature::verify(b"other bytes", &sig, &[server.public_key()]),
        Err(CoreError::BadSignature)
    ));
    assert_eq!(server.requests(), ["/manifest.json", "/manifest.json.sig"]);
}

#[tokio::test]
async fn a_download_is_written_and_hashed() {
    let server = TestServer::start().await;
    let body = vec![7_u8; 300_000];
    server.route("/OaMod-0.1.0.dll", Reply::Body(body.clone()));
    let temp = tempfile::tempdir().expect("a temporary directory");
    let destination = temp.path().join("OaMod.dll.part");

    let done = net::download(
        &client(),
        &url(&server, "/OaMod-0.1.0.dll"),
        &destination,
        body.len() as u64,
        &|_| {},
    )
    .await
    .expect("the download finishes");

    assert_eq!(done.bytes, body.len() as u64);
    assert_eq!(done.sha256, format!("{:x}", Sha256::digest(&body)));
    assert_eq!(std::fs::read(&destination).expect("the file"), body);
}

#[tokio::test]
async fn a_download_longer_than_announced_is_stopped() {
    let server = TestServer::start().await;
    server.route("/big.zip", Reply::Body(vec![0_u8; 4096]));
    let temp = tempfile::tempdir().expect("a temporary directory");

    let result = net::download(
        &client(),
        &url(&server, "/big.zip"),
        &temp.path().join("big.zip.part"),
        100,
        &|_| {},
    )
    .await;

    assert!(matches!(
        result,
        Err(CoreError::SizeMismatch { expected: 100, .. })
    ));
}

#[tokio::test]
async fn a_missing_file_and_a_redirect_to_plain_http_are_errors() {
    let server = TestServer::start().await;
    server.route(
        "/moved.json",
        Reply::Redirect("http://example.com/manifest.json".to_owned()),
    );
    server.route("/here.json", Reply::Body(b"{}".to_vec()));
    server.route("/local.json", Reply::Redirect(server.url("/here.json")));

    for path in ["/absent.json", "/moved.json"] {
        let result = net::fetch_bytes(&client(), &url(&server, path), "read the manifest").await;
        assert!(
            matches!(result, Err(CoreError::Http { .. })),
            "{path} must fail"
        );
    }
    // Loopback to loopback is the one plain http redirect that is followed.
    let followed =
        net::fetch_bytes(&client(), &url(&server, "/local.json"), "read the manifest").await;
    assert_eq!(followed.expect("the redirect is followed"), b"{}");
}
