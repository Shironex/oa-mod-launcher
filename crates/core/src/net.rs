//! Every network call of the launcher.
//!
//! Three kinds of file are requested: the manifest together with its
//! signature, the mod DLL and the BepInEx zip. All requests go through one
//! client whose rules do not depend on the caller:
//!
//! - **https only.** Plain http is accepted for a loopback address and nothing
//!   else, which is what lets tests and local runs use a server on 127.0.0.1.
//! - **Checked on every hop.** GitHub answers a release download with a
//!   redirect to another host, so the rule is applied to each redirect target,
//!   and a redirect from https to http is refused even when it points at
//!   loopback.
//! - **Bounded.** Small answers have a byte limit, the manifest has a short
//!   total timeout, and a download fails when no bytes arrive for a while.

use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::error::{CoreError, Result};

/// Where the manifest of the stable channel is published.
pub const DEFAULT_FEED: &str =
    "https://github.com/Shironex/oa-mod/releases/latest/download/manifest.json";

/// Overrides [`DEFAULT_FEED`]. Either the full URL of a `manifest.json`, or a
/// base URL that `manifest.json` is appended to.
pub const FEED_ENV: &str = "OA_MOD_LAUNCHER_FEED";

/// How many redirects one request may follow.
pub const MAX_REDIRECTS: usize = 5;

/// How long the manifest or its signature may take, from request to last
/// byte.
pub const TEXT_TIMEOUT: Duration = Duration::from_secs(5);

/// The largest manifest or signature that is read. Both are a few kilobytes
/// at most.
pub const MAX_TEXT_BYTES: usize = 512 * 1024;

/// How long a download may deliver no bytes before it is given up.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

/// How long connecting to a host may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The manifest URL to use: the override from [`FEED_ENV`] when set, otherwise
/// [`DEFAULT_FEED`].
pub fn manifest_url_from_environment() -> String {
    match std::env::var(FEED_ENV) {
        Ok(value) if !value.trim().is_empty() => manifest_url_from(value.trim()),
        _ => DEFAULT_FEED.to_owned(),
    }
}

/// Turn a configured feed value into the URL of a manifest.
fn manifest_url_from(value: &str) -> String {
    if value.ends_with(".json") {
        value.to_owned()
    } else {
        format!("{}/manifest.json", value.trim_end_matches('/'))
    }
}

/// Parse `text` and apply the https rule.
///
/// # Errors
///
/// [`CoreError::InsecureUrl`] for anything that is not https, or http to a
/// loopback address.
pub fn checked_url(text: &str) -> Result<Url> {
    let url = Url::parse(text).map_err(|_| CoreError::InsecureUrl(text.to_owned()))?;
    if is_allowed(&url) {
        Ok(url)
    } else {
        Err(CoreError::InsecureUrl(text.to_owned()))
    }
}

/// The URL of the signature of the file at `url`: the same URL with `.sig`
/// added to the path, which is where `tauri signer sign` puts it on disk.
///
/// Scheme and host stay the same, so a URL that passed the https rule still
/// passes it.
pub fn signature_url(url: &Url) -> Url {
    let mut signature = url.clone();
    signature.set_path(&format!("{}.sig", url.path()));
    signature
}

fn is_allowed(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => is_loopback(url),
        _ => false,
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

/// Whether a redirect from `from` to `to` may be followed.
fn hop_allowed(from: &Url, to: &Url) -> bool {
    match to.scheme() {
        "https" => true,
        // Loopback http may stay on loopback http. Leaving https never is.
        "http" => from.scheme() == "http" && is_loopback(from) && is_loopback(to),
        _ => false,
    }
}

/// The client every request uses.
///
/// # Errors
///
/// [`CoreError::Http`] when the TLS setup fails.
pub fn client(launcher_version: &str) -> Result<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        let allowed = attempt
            .previous()
            .last()
            .is_some_and(|from| hop_allowed(from, attempt.url()));
        if allowed {
            attempt.follow()
        } else {
            attempt.error("a redirect left https")
        }
    });

    reqwest::Client::builder()
        .user_agent(format!("oa-mod-launcher/{launcher_version}"))
        .redirect(policy)
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(STALL_TIMEOUT)
        .build()
        .map_err(|error| CoreError::Http {
            operation: "prepare the network client",
            message: error.to_string(),
        })
}

/// Fetch a small file within [`TEXT_TIMEOUT`], as the bytes that arrived.
///
/// Bytes and not text on purpose: a signature is over the exact bytes of the
/// file, and decoding them first could change what is checked.
///
/// # Errors
///
/// [`CoreError::Http`] for a transport failure, a timeout, a status that is not
/// 2xx or a body over [`MAX_TEXT_BYTES`].
pub async fn fetch_bytes(
    client: &reqwest::Client,
    url: &Url,
    operation: &'static str,
) -> Result<Vec<u8>> {
    let failed = |message: String| CoreError::Http { operation, message };

    let read = async {
        let mut response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|error| failed(describe(&error)))?;
        if !response.status().is_success() {
            return Err(failed(format!(
                "the server answered {}",
                response.status().as_u16()
            )));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| failed(describe(&error)))?
        {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_TEXT_BYTES {
                return Err(failed(format!(
                    "the answer is larger than {MAX_TEXT_BYTES} bytes"
                )));
            }
        }
        Ok(body)
    };

    tokio::time::timeout(TEXT_TIMEOUT, read)
        .await
        .map_err(|_| {
            failed(format!(
                "no answer within {} seconds",
                TEXT_TIMEOUT.as_secs()
            ))
        })?
}

/// What a finished download was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    /// Bytes written to the destination.
    pub bytes: u64,
    /// Lowercase hex SHA-256 of those bytes.
    pub sha256: String,
}

/// Stream `url` into `destination`, hashing the bytes on the way.
///
/// `progress` is called with the bytes received so far, at most once per
/// percent of `expected_size`, so a large file does not flood the window.
/// The download stops as soon as more than `expected_size` bytes arrive.
///
/// # Errors
///
/// [`CoreError::Http`] for a transport failure or a bad status,
/// [`CoreError::SizeMismatch`] when the body is longer than announced,
/// [`CoreError::Io`] when the destination cannot be written.
pub async fn download(
    client: &reqwest::Client,
    url: &Url,
    destination: &Path,
    expected_size: u64,
    progress: &(dyn Fn(u64) + Send + Sync),
) -> Result<Downloaded> {
    const OPERATION: &str = "download the file";
    let failed = |message: String| CoreError::Http {
        operation: OPERATION,
        message,
    };

    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|error| failed(describe(&error)))?;
    if !response.status().is_success() {
        return Err(failed(format!(
            "the server answered {}",
            response.status().as_u16()
        )));
    }

    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(CoreError::io("create the download file", destination))?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let step = (expected_size / 100).max(1);
    let mut next_report = 0;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| failed(describe(&error)))?
    {
        received += chunk.len() as u64;
        if received > expected_size {
            return Err(CoreError::SizeMismatch {
                expected: expected_size,
                actual: received,
            });
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(CoreError::io("write the download file", destination))?;
        if received >= next_report {
            next_report = received + step;
            progress(received);
        }
    }

    file.flush()
        .await
        .map_err(CoreError::io("write the download file", destination))?;
    file.sync_all()
        .await
        .map_err(CoreError::io("write the download file", destination))?;
    progress(received);

    Ok(Downloaded {
        bytes: received,
        sha256: hex(&hasher.finalize()),
    })
}

/// Lowercase hex of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// A request error in words a player can read, without the URL.
fn describe(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "the connection timed out".to_owned()
    } else if error.is_redirect() {
        "a redirect was refused".to_owned()
    } else if error.is_connect() {
        "no connection to the server".to_owned()
    } else {
        "the connection failed".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a valid test URL")
    }

    #[test]
    fn https_is_accepted_and_plain_http_only_for_loopback() {
        assert!(checked_url("https://github.com/a/manifest.json").is_ok());
        assert!(checked_url("http://127.0.0.1:8123/manifest.json").is_ok());
        assert!(checked_url("http://localhost:8123/manifest.json").is_ok());
        assert!(checked_url("http://[::1]:8123/manifest.json").is_ok());

        for refused in [
            "http://github.com/a/manifest.json",
            "http://192.168.1.10/manifest.json",
            "http://127.0.0.1.example.com/manifest.json",
            "http://localhost.example.com/manifest.json",
            "ftp://127.0.0.1/manifest.json",
            "file:///C:/manifest.json",
            "manifest.json",
        ] {
            assert!(
                matches!(checked_url(refused), Err(CoreError::InsecureUrl(_))),
                "{refused} must be refused"
            );
        }
    }

    #[test]
    fn a_redirect_may_never_leave_https() {
        let github = url("https://github.com/a");
        let loopback = url("http://127.0.0.1:1/a");

        assert!(hop_allowed(
            &github,
            &url("https://objects.githubusercontent.com/b")
        ));
        assert!(!hop_allowed(
            &github,
            &url("http://objects.githubusercontent.com/b")
        ));
        assert!(
            !hop_allowed(&github, &url("http://127.0.0.1:1/b")),
            "https to loopback http"
        );
        assert!(!hop_allowed(&github, &url("file:///etc/passwd")));
        assert!(hop_allowed(&loopback, &url("http://127.0.0.1:2/b")));
        assert!(hop_allowed(&loopback, &url("https://github.com/b")));
        assert!(
            !hop_allowed(&loopback, &url("http://example.com/b")),
            "loopback to remote http"
        );
    }

    #[test]
    fn a_configured_feed_may_be_a_file_or_a_base() {
        assert_eq!(
            manifest_url_from("http://127.0.0.1:8123/step1/"),
            "http://127.0.0.1:8123/step1/manifest.json"
        );
        assert_eq!(
            manifest_url_from("http://127.0.0.1:8123/step1"),
            "http://127.0.0.1:8123/step1/manifest.json"
        );
        assert_eq!(
            manifest_url_from("http://127.0.0.1:8123/m/manifest.json"),
            "http://127.0.0.1:8123/m/manifest.json"
        );
    }

    #[test]
    fn the_signature_sits_next_to_its_file() {
        assert_eq!(
            signature_url(&url(
                "https://github.com/o/r/releases/latest/download/manifest.json"
            ))
            .as_str(),
            "https://github.com/o/r/releases/latest/download/manifest.json.sig"
        );
        assert_eq!(
            signature_url(&url("http://127.0.0.1:8123/manifest.json?cache=1")).as_str(),
            "http://127.0.0.1:8123/manifest.json.sig?cache=1",
            "the query stays behind the path"
        );
    }

    #[test]
    fn hex_is_lowercase_and_two_digits_per_byte() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }
}
