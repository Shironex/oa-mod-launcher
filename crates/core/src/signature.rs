//! The proof that a manifest was published by the owner.
//!
//! The SHA-256 in the manifest only proves that a file is the one the manifest
//! names. The manifest comes from the same place as the file, so somebody who
//! can change a release can change both. The signature closes that gap: it is
//! made with a private key that never leaves the owner's machine, and the
//! launcher carries the public half.
//!
//! The keys are minisign keys in the wire format of the Tauri CLI
//! (`pnpm tauri signer generate`, `pnpm tauri signer sign`): the `.pub` file
//! and the `.sig` file are each the base64 of the usual minisign text. They are
//! read here the way `tauri-plugin-updater` reads them, so one tool makes the
//! keys for the launcher's own updates and for the manifest.
//!
//! A signature is over the exact bytes of the file. Nothing may decode,
//! reformat or parse those bytes before [`verify`] has accepted them.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use minisign_verify::{PublicKey, Signature};

use crate::error::{CoreError, Result};

/// The public keys a release build trusts: the one manifest key, which signs
/// every release. It is the content of the `.pub` file written by
/// `pnpm tauri signer generate`. The private half is not in the repository.
///
/// There is no spare key. An installed launcher trusts this key and no other,
/// for as long as it is installed: if the key is lost, everyone installs a new
/// launcher by hand.
pub const RELEASE_KEYS: [&str; 1] = [
    "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDlEQUYyODBFQzJEQUVFMkMKUldRczd0ckNEaWl2blpiNk1UMVZ3b3BXQk0wZnUzMW1hUDMxUEJxMkh5SHRhRWdabjdreDhOUGEK",
];

/// The public half of `dev-keys/dev.key`, the key that signs local test
/// manifests. Its private half is committed with an empty password, so a
/// signature by it proves nothing.
///
/// That is why the constant exists in a development build only: a release
/// build has no way to trust it.
#[cfg(debug_assertions)]
const DEV_KEY: &str = include_str!("../../../dev-keys/dev.key.pub");

/// The keys a normal start trusts: [`RELEASE_KEYS`], and in a development
/// build also the development key.
pub fn trusted_keys() -> Vec<String> {
    let keys = RELEASE_KEYS.iter().copied();
    #[cfg(debug_assertions)]
    let keys = keys.chain([DEV_KEY.trim()]);
    keys.map(str::to_owned).collect()
}

/// Check that `sig_text` is a signature over exactly `bytes` by one of
/// `trusted_keys`.
///
/// `sig_text` is the content of a `.sig` file and each key the content of a
/// `.pub` file, both as the Tauri CLI writes them. Whitespace around them is
/// ignored, because an editor or a copy and paste adds a line end easily.
///
/// # Errors
///
/// [`CoreError::BadSignature`] when the signature cannot be read or no trusted
/// key made it.
pub fn verify<K: AsRef<str>>(bytes: &[u8], sig_text: &str, trusted_keys: &[K]) -> Result<()> {
    matching_key(bytes, sig_text, trusted_keys).map(|_| ())
}

/// Like [`verify`], and says which key made the signature: its position in
/// `trusted_keys`, counted from 0.
///
/// # Errors
///
/// [`CoreError::BadSignature`], as for [`verify`].
pub fn matching_key<K: AsRef<str>>(
    bytes: &[u8],
    sig_text: &str,
    trusted_keys: &[K],
) -> Result<usize> {
    let signature = minisign_text(sig_text)
        .and_then(|text| Signature::decode(&text).ok())
        .ok_or(CoreError::BadSignature)?;

    trusted_keys
        .iter()
        .position(|key| {
            // A key that cannot be read is simply not the key that signed. It must not hide a good key after it.
            minisign_text(key.as_ref())
                .and_then(|text| PublicKey::decode(&text).ok())
                // `false` refuses the old signature kind that is not hashed
                // first. The Tauri CLI has never written it.
                .is_some_and(|key| key.verify(bytes, &signature, false).is_ok())
        })
        .ok_or(CoreError::BadSignature)
}

/// The app version a signature is bound to: the `version:` field of its
/// trusted comment. `tauri build` and `tauri signer sign --app-version` write
/// it, and the updater's `requireSignedVersion` refuses an installer without
/// it. `None` when the signature cannot be read or names no version.
///
/// The trusted comment is covered by the signature, but this function only
/// reads it. The answer means something for a signature that [`verify`] has
/// accepted, and nothing before that.
pub fn signed_version(sig_text: &str) -> Option<String> {
    let signature = Signature::decode(&minisign_text(sig_text)?).ok()?;
    // Whole fields, split on the tab the CLI puts between them: a search for
    // the text `version:0.1.1` would also find it inside `version:0.1.10`.
    signature
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
        .map(str::to_owned)
}

/// Undo the base64 layer the Tauri CLI puts around a minisign key or
/// signature.
fn minisign_text(wrapped: &str) -> Option<String> {
    let bytes = STANDARD.decode(wrapped.trim()).ok()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use minisign::KeyPair;

    use super::*;

    const MANIFEST: &[u8] = br#"{"schema":1,"version":"0.9.0"}"#;

    /// The development public key, read by the tests themselves so that they
    /// also run in a release build, where [`DEV_KEY`] does not exist.
    const DEV_PUB: &str = include_str!("../../../dev-keys/dev.key.pub");

    /// A file that `tauri signer sign -f dev-keys/dev.key` signed, and the
    /// `.sig` it wrote. Made once with the real CLI, because the fixtures of
    /// the other tests are signed by a library and prove only that this
    /// library and the verifier agree.
    const CLI_SIGNED: &[u8] = include_bytes!("../tests/data/signed-by-dev-key.json");
    const CLI_SIGNATURE: &str = include_str!("../tests/data/signed-by-dev-key.json.sig");

    /// A throwaway key pair. Without a password, because the password only
    /// protects the private key on disk and is slow to apply.
    fn new_pair() -> KeyPair {
        KeyPair::generate_unencrypted_keypair().expect("a key pair")
    }

    /// The content of the `.pub` file the Tauri CLI would write for `pair`.
    fn public_text(pair: &KeyPair) -> String {
        let text = pair.pk.to_box().expect("a public key box").to_string();
        STANDARD.encode(text)
    }

    /// The content of the `.sig` file the Tauri CLI would write for `bytes`:
    /// the same call with the same two comments, then base64 of the text.
    fn sign(pair: &KeyPair, bytes: &[u8]) -> String {
        sign_with_comment(pair, bytes, "timestamp:1791280496\tfile:manifest.json")
    }

    fn sign_with_comment(pair: &KeyPair, bytes: &[u8], trusted_comment: &str) -> String {
        let signature = minisign::sign(
            None,
            &pair.sk,
            Cursor::new(bytes),
            Some(trusted_comment),
            Some("signature from tauri secret key"),
        )
        .expect("a signature");
        STANDARD.encode(signature.to_string())
    }

    #[test]
    fn a_signature_by_a_trusted_key_is_accepted() {
        let pair = new_pair();
        let signature = sign(&pair, MANIFEST);

        assert!(verify(MANIFEST, &signature, &[public_text(&pair)]).is_ok());
    }

    #[test]
    fn one_changed_byte_is_refused() {
        let pair = new_pair();
        let signature = sign(&pair, MANIFEST);
        let mut changed = MANIFEST.to_vec();
        changed[20] ^= 1;

        let result = verify(&changed, &signature, &[public_text(&pair)]);

        assert!(matches!(result, Err(CoreError::BadSignature)));
    }

    #[test]
    fn a_signature_by_an_untrusted_key_is_refused() {
        let trusted = new_pair();
        let stranger = new_pair();
        let signature = sign(&stranger, MANIFEST);

        let result = verify(MANIFEST, &signature, &[public_text(&trusted)]);

        assert!(matches!(result, Err(CoreError::BadSignature)));
    }

    #[test]
    fn text_that_is_not_a_signature_is_refused_without_a_panic() {
        let pair = new_pair();
        let keys = [public_text(&pair)];
        let good = sign(&pair, MANIFEST);

        for bad in [
            "",
            "   \n",
            "not base64 at all !!!",
            "aGVsbG8=",
            // Valid base64, but the bytes under it are not text.
            "/////w==",
            // The first half of a real signature.
            &good[..good.len() / 2],
            // A key where a signature belongs.
            keys[0].as_str(),
        ] {
            assert!(
                matches!(verify(MANIFEST, bad, &keys), Err(CoreError::BadSignature)),
                "{bad:?} must be refused"
            );
        }
    }

    #[test]
    fn no_key_or_an_unreadable_key_accepts_nothing() {
        let pair = new_pair();
        let signature = sign(&pair, MANIFEST);

        let no_keys: [&str; 0] = [];
        assert!(verify(MANIFEST, &signature, &no_keys).is_err());
        assert!(verify(MANIFEST, &signature, &["", "garbage", "aGVsbG8="]).is_err());
        assert!(verify(MANIFEST, &signature, &RELEASE_KEYS[..0]).is_err());
    }

    #[test]
    fn any_key_of_the_list_is_enough() {
        let first = new_pair();
        let second = new_pair();
        let keys = [public_text(&first), public_text(&second)];

        let by_first = sign(&first, MANIFEST);
        let by_second = sign(&second, MANIFEST);

        assert_eq!(matching_key(MANIFEST, &by_first, &keys).ok(), Some(0));
        assert_eq!(matching_key(MANIFEST, &by_second, &keys).ok(), Some(1));
        assert!(verify(MANIFEST, &by_second, &keys).is_ok());
    }

    #[test]
    fn an_unreadable_key_does_not_hide_the_good_key_after_it() {
        let pair = new_pair();
        let keys = ["not a key".to_owned(), public_text(&pair)];

        let found = matching_key(MANIFEST, &sign(&pair, MANIFEST), &keys);

        assert_eq!(found.ok(), Some(1));
    }

    #[test]
    fn whitespace_around_the_key_and_the_signature_is_ignored() {
        let pair = new_pair();
        let signature = format!("\r\n  {}\n", sign(&pair, MANIFEST));
        let key = format!("  {}\r\n\n", public_text(&pair));

        assert!(verify(MANIFEST, &signature, &[key]).is_ok());
    }

    #[test]
    fn the_fixtures_have_the_shape_of_the_files_the_tauri_cli_writes() {
        // What `tauri signer` writes was read from the source of the CLI. This
        // pins the same shape for the fixtures above, so a change in the
        // signing crate that would make them differ is seen here.
        let pair = new_pair();
        let key = minisign_text(&public_text(&pair)).expect("the key is base64 of text");
        let signature = minisign_text(&sign(&pair, MANIFEST)).expect("base64 of text");

        let key_lines: Vec<&str> = key.lines().collect();
        assert_eq!(key_lines.len(), 2);
        assert!(key_lines[0].starts_with("untrusted comment: "));

        let lines: Vec<&str> = signature.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[0],
            "untrusted comment: signature from tauri secret key"
        );
        assert!(lines[2].starts_with("trusted comment: timestamp:"));
        let raw = STANDARD.decode(lines[1]).expect("the signature line");
        assert_eq!(&raw[..2], b"ED", "hashed first, the only kind accepted");
    }

    #[test]
    fn the_signed_version_is_the_whole_version_field_of_the_trusted_comment() {
        let pair = new_pair();
        // The comment `tauri signer sign --app-version 0.1.10` writes.
        let bound = sign_with_comment(
            &pair,
            MANIFEST,
            "timestamp:1791390998\tfile:setup.exe\tversion:0.1.10",
        );

        assert_eq!(signed_version(&bound).as_deref(), Some("0.1.10"));
        assert_eq!(signed_version(&sign(&pair, MANIFEST)), None);
        assert_eq!(signed_version("not a signature"), None);
    }

    #[test]
    fn a_signature_written_by_the_tauri_cli_is_accepted() {
        assert_eq!(
            matching_key(CLI_SIGNED, CLI_SIGNATURE, &[DEV_PUB]).ok(),
            Some(0)
        );

        let mut changed = CLI_SIGNED.to_vec();
        changed[2] ^= 1;
        assert!(verify(&changed, CLI_SIGNATURE, &[DEV_PUB]).is_err());
        assert!(
            verify(CLI_SIGNED, CLI_SIGNATURE, &RELEASE_KEYS).is_err(),
            "the development key signs nothing a release build accepts"
        );
    }

    #[test]
    fn release_keys_are_real() {
        for key in RELEASE_KEYS {
            let text = minisign_text(key).expect("a release key is base64 of the minisign text");
            assert!(
                PublicKey::decode(&text).is_ok(),
                "{key} is not a minisign public key"
            );
            assert_ne!(key, DEV_PUB.trim(), "the development key is no release key");
        }
    }

    #[test]
    fn the_development_key_is_a_key_the_tauri_cli_wrote() {
        let text = minisign_text(DEV_PUB).expect("dev.key.pub is base64 of the minisign text");
        assert!(PublicKey::decode(&text).is_ok());
    }

    #[test]
    #[cfg(debug_assertions)]
    fn a_development_build_also_trusts_the_development_key() {
        let keys = trusted_keys();
        assert_eq!(keys[..1], RELEASE_KEYS);
        assert_eq!(keys[1..], [DEV_PUB.trim()]);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn a_release_build_trusts_the_release_keys_only() {
        assert_eq!(trusted_keys(), RELEASE_KEYS);
    }
}
