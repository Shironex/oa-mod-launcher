//! `manifest.json`: the one small file a mod release publishes for the
//! launcher.
//!
//! It is read as untrusted text, after its signature has been accepted. Every
//! value that is used later (addresses, sizes, checksums) is checked here,
//! once, so the rest of the crate can use a [`Manifest`] without looking at it
//! again.
//!
//! No path comes from the manifest. The names of the files the launcher writes
//! into the game folder are constants of this crate; the manifest only says
//! what their content must hash to.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::{CoreError, Result};
use crate::net;
use crate::version::Version;

/// The only manifest format this launcher understands.
pub const SCHEMA: u32 = 1;

/// Where every file a release manifest names must be: a release of the mod's
/// repository, by its permanent address.
pub const DOWNLOAD_PREFIX: &str = "https://github.com/Shironex/oa-mod/releases/download/";

/// The files BepInEx puts into the root of the game folder, in the order they
/// are written. `winhttp.dll` is last: it is the file that makes the game load
/// BepInEx, so a half finished install never loads.
pub const ROOT_FILES: [&str; 4] = [
    "doorstop_config.ini",
    ".doorstop_version",
    "changelog.txt",
    "winhttp.dll",
];

/// `manifest.json`: which mod version is the newest and where its files are.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// Format number, see [`SCHEMA`].
    pub schema: u32,
    /// The mod version to install, `major.minor.patch`.
    pub version: String,
    /// When the release was built. Written for people, not read.
    #[serde(default)]
    pub published: Option<String>,
    /// One line about the release.
    #[serde(default)]
    pub notes: Option<String>,
    /// The mod itself, one DLL.
    #[serde(rename = "mod")]
    pub mod_file: Download,
    /// The BepInEx build the mod needs.
    pub bepinex: BepInEx,
    /// What is known about the game.
    pub game: Game,
    /// What the release needs from the launcher.
    pub launcher: LauncherRequirement,
}

/// One downloadable file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Download {
    /// Where it is.
    pub url: String,
    /// Its length in bytes.
    pub size: u64,
    /// Its SHA-256, 64 lowercase hex characters after parsing.
    pub sha256: String,
}

/// The `bepinex` block: the zip and what a correct copy on disk hashes to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BepInEx {
    /// Display text such as `6.0.0-be.788`. Never compared as a version.
    pub label: String,
    /// The zip.
    pub url: String,
    /// Its length in bytes.
    pub size: u64,
    /// Its SHA-256.
    pub sha256: String,
    /// The SHA-256 of `BepInEx/core/BepInEx.Unity.IL2CPP.dll` inside the zip.
    /// A copy on disk with this hash is this build.
    pub probe_sha256: String,
    /// The SHA-256 of each of the [`ROOT_FILES`]. Exactly those four names.
    pub root_files: BTreeMap<String, String>,
}

/// The `game` block.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Game {
    /// The Steam build ids the mod was tried with. Another build only warns.
    pub builds: Vec<String>,
}

/// The `launcher` block.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LauncherRequirement {
    /// The oldest launcher that may install this release.
    pub min: String,
}

impl Manifest {
    /// Parse and check a release manifest: every address must be under
    /// [`DOWNLOAD_PREFIX`].
    ///
    /// # Errors
    ///
    /// [`CoreError::BadManifest`], as for [`Manifest::parse_from`].
    pub fn parse(text: &str) -> Result<Self> {
        Self::parse_from(text, DOWNLOAD_PREFIX)
    }

    /// Parse and check a manifest whose files must be under `download_prefix`.
    /// A release build passes [`DOWNLOAD_PREFIX`]; a test passes the address
    /// of its loopback server.
    ///
    /// # Errors
    ///
    /// [`CoreError::BadManifest`] for anything that is not JSON of the known
    /// schema with every field in order, including an HTML page from a
    /// captive portal.
    pub fn parse_from(text: &str, download_prefix: &str) -> Result<Self> {
        let bad = CoreError::BadManifest;
        let mut manifest: Self = serde_json::from_str(text)
            .map_err(|error| bad(format!("it is not a manifest ({error})")))?;

        if manifest.schema != SCHEMA {
            return Err(bad(format!(
                "format {} is not known to this launcher",
                manifest.schema
            )));
        }
        if Version::parse(&manifest.version).is_none() {
            return Err(bad(format!("\"{}\" is not a version", manifest.version)));
        }
        if Version::parse(&manifest.launcher.min).is_none() {
            return Err(bad(format!(
                "\"{}\" is not a launcher version",
                manifest.launcher.min
            )));
        }

        let m = &mut manifest.mod_file;
        check_download("the mod", &m.url, m.size, &mut m.sha256, download_prefix).map_err(bad)?;
        let b = &mut manifest.bepinex;
        check_download("BepInEx", &b.url, b.size, &mut b.sha256, download_prefix).map_err(bad)?;

        if b.label.is_empty() || b.label.len() > 64 || b.label.chars().any(char::is_control) {
            return Err(bad("the BepInEx label is not a short line".to_owned()));
        }
        check_hash(&mut b.probe_sha256)
            .map_err(|problem| bad(format!("the BepInEx probe {problem}")))?;
        if b.root_files.len() != ROOT_FILES.len() {
            return Err(bad(format!(
                "BepInEx must name exactly these root files: {}",
                ROOT_FILES.join(", ")
            )));
        }
        for name in ROOT_FILES {
            let hash = b
                .root_files
                .get_mut(name)
                .ok_or_else(|| bad(format!("BepInEx names no checksum for {name}")))?;
            check_hash(hash).map_err(|problem| bad(format!("{name} {problem}")))?;
        }

        for build in &manifest.game.builds {
            if build.is_empty() || build.len() > 20 || !build.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad(format!("\"{build}\" is not a Steam build id")));
            }
        }

        Ok(manifest)
    }

    /// The lowest launcher version this release accepts.
    pub fn minimum_launcher(&self) -> Version {
        // Checked in `parse_from`, so the fallback is never used.
        Version::parse(&self.launcher.min).unwrap_or(Version {
            major: u32::MAX,
            minor: 0,
            patch: 0,
        })
    }
}

fn check_download(
    what: &str,
    url: &str,
    size: u64,
    sha256: &mut str,
    prefix: &str,
) -> std::result::Result<(), String> {
    // Parsed first, so `..` and the like are resolved before the comparison,
    // and the https rule of every request is applied here already.
    let parsed =
        net::checked_url(url).map_err(|_| format!("{what} has an address that is not https"))?;
    if !parsed.as_str().starts_with(prefix) || parsed.query().is_some() {
        return Err(format!("{what} is not at the release address ({prefix})"));
    }
    if size == 0 {
        return Err(format!("{what} has no size"));
    }
    check_hash(sha256).map_err(|problem| format!("{what} {problem}"))
}

/// Accept 64 hex characters and make them lowercase, which is how every
/// digest in this crate is compared.
fn check_hash(sha256: &mut str) -> std::result::Result<(), String> {
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("has a checksum that is not 64 hex characters".to_owned());
    }
    sha256.make_ascii_lowercase();
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A valid manifest as a JSON value, for a test to break one thing in.
    pub(crate) fn sample() -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "version": "0.1.0",
            "published": "2026-10-20T18:00:00Z",
            "notes": "One line.",
            "mod": { "url": format!("{DOWNLOAD_PREFIX}v0.1.0/OaMod-0.1.0.dll"), "size": 10, "sha256": HASH },
            "bepinex": {
                "label": "6.0.0-be.788",
                "url": format!("{DOWNLOAD_PREFIX}bepinex-6.0.0-be.788/BepInEx.zip"),
                "size": 20,
                "sha256": HASH,
                "probe_sha256": HASH,
                "root_files": {
                    "winhttp.dll": HASH, "doorstop_config.ini": HASH,
                    ".doorstop_version": HASH, "changelog.txt": HASH
                }
            },
            "game": { "builds": ["20236533"] },
            "launcher": { "min": "0.1.0" }
        })
    }

    fn refused(value: &serde_json::Value) -> String {
        match Manifest::parse(&value.to_string()) {
            Err(CoreError::BadManifest(reason)) => reason,
            other => panic!("expected a refused manifest, got {other:?}"),
        }
    }

    #[test]
    fn a_complete_manifest_parses() {
        let manifest = Manifest::parse(&sample().to_string()).expect("the manifest is valid");

        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.mod_file.size, 10);
        assert_eq!(manifest.bepinex.label, "6.0.0-be.788");
        assert_eq!(manifest.bepinex.root_files.len(), 4);
        assert_eq!(manifest.game.builds, ["20236533"]);
        assert_eq!(manifest.minimum_launcher().to_string(), "0.1.0");
        assert_eq!(manifest.notes.as_deref(), Some("One line."));
    }

    #[test]
    fn an_html_page_is_not_a_manifest() {
        let error = Manifest::parse("<html><body>Sign in to the hotel Wi-Fi</body></html>");
        assert!(matches!(error, Err(CoreError::BadManifest(_))));
    }

    #[test]
    fn an_unknown_schema_or_a_bad_version_is_refused() {
        let mut value = sample();
        value["schema"] = 2.into();
        assert!(refused(&value).contains("format 2"));

        for version in ["../../evil", "0.9", "v0.9.0", ""] {
            let mut value = sample();
            value["version"] = version.into();
            assert!(refused(&value).contains("not a version"), "{version:?}");
        }
        let mut value = sample();
        value["launcher"]["min"] = "soon".into();
        assert!(refused(&value).contains("launcher version"));
    }

    #[test]
    fn every_block_is_required() {
        for key in ["version", "mod", "bepinex", "game", "launcher"] {
            let mut value = sample();
            value.as_object_mut().expect("an object").remove(key);
            assert!(refused(&value).contains("not a manifest"), "{key}");
        }
        for key in [
            "url",
            "size",
            "sha256",
            "probe_sha256",
            "root_files",
            "label",
        ] {
            let mut value = sample();
            value["bepinex"]
                .as_object_mut()
                .expect("an object")
                .remove(key);
            assert!(refused(&value).contains("not a manifest"), "{key}");
        }
    }

    #[test]
    fn an_address_that_is_not_https_on_the_release_host_is_refused() {
        for url in [
            "http://github.com/Shironex/oa-mod/releases/download/v0.1.0/OaMod.dll",
            "https://example.com/Shironex/oa-mod/releases/download/v0.1.0/OaMod.dll",
            "https://github.com/Somebody/oa-mod/releases/download/v0.1.0/OaMod.dll",
            "https://github.com/Shironex/oa-mod/releases/download/../../../../evil/OaMod.dll",
            "https://github.com.evil.example/Shironex/oa-mod/releases/download/a.dll",
            "https://user@github.com/Shironex/oa-mod/releases/download/a.dll",
            "file:///C:/Windows/OaMod.dll",
            "OaMod.dll",
            "",
        ] {
            for block in ["mod", "bepinex"] {
                let mut value = sample();
                value[block]["url"] = url.into();
                let reason = refused(&value);
                assert!(reason.contains("address"), "{url:?}: {reason}");
            }
        }
    }

    #[test]
    fn a_loopback_address_is_accepted_only_when_the_caller_names_it() {
        let text = sample()
            .to_string()
            .replace(DOWNLOAD_PREFIX, "http://127.0.0.1:8123/files/");

        assert!(Manifest::parse(&text).is_err());
        assert!(Manifest::parse_from(&text, "http://127.0.0.1:8123/files/").is_ok());
        assert!(Manifest::parse_from(&text, "http://127.0.0.1:9999/").is_err());
    }

    #[test]
    fn a_size_or_a_checksum_of_the_wrong_shape_is_refused() {
        for block in ["mod", "bepinex"] {
            let mut value = sample();
            value[block]["size"] = 0.into();
            assert!(refused(&value).contains("no size"));

            let mut value = sample();
            value[block]["size"] = (-1).into();
            assert!(refused(&value).contains("not a manifest"));

            for sha256 in ["abc", &HASH[1..], &format!("{}g", &HASH[1..])] {
                let mut value = sample();
                value[block]["sha256"] = sha256.into();
                assert!(refused(&value).contains("64 hex"));
            }
        }
        let mut value = sample();
        value["bepinex"]["probe_sha256"] = "abc".into();
        assert!(refused(&value).contains("probe"));
    }

    #[test]
    fn checksums_are_lowercase_after_parsing() {
        let mut value = sample();
        value["mod"]["sha256"] = HASH.to_ascii_uppercase().into();
        value["bepinex"]["root_files"]["winhttp.dll"] = HASH.to_ascii_uppercase().into();

        let manifest = Manifest::parse(&value.to_string()).expect("upper case is accepted");

        assert_eq!(manifest.mod_file.sha256, HASH);
        assert_eq!(manifest.bepinex.root_files["winhttp.dll"], HASH);
    }

    #[test]
    fn the_root_files_are_exactly_the_four_known_names() {
        let mut value = sample();
        value["bepinex"]["root_files"]
            .as_object_mut()
            .expect("an object")
            .remove("changelog.txt");
        assert!(refused(&value).contains("exactly"));

        // A fifth name, and a name that is a path: neither is ever joined to
        // the game folder, because the manifest is refused.
        for extra in ["version.dll", "..\\..\\Windows\\evil.dll"] {
            let mut value = sample();
            value["bepinex"]["root_files"][extra] = HASH.into();
            assert!(refused(&value).contains("exactly"), "{extra}");
        }

        let mut value = sample();
        let files = value["bepinex"]["root_files"]
            .as_object_mut()
            .expect("an object");
        files.remove("winhttp.dll");
        files.insert("WINHTTP.DLL".to_owned(), HASH.into());
        assert!(refused(&value).contains("winhttp.dll"));
    }

    #[test]
    fn a_label_or_a_build_id_of_the_wrong_shape_is_refused() {
        for label in ["", "two\nlines", &"x".repeat(65)] {
            let mut value = sample();
            value["bepinex"]["label"] = label.into();
            assert!(refused(&value).contains("label"));
        }
        for build in ["", "latest", "12 34"] {
            let mut value = sample();
            value["game"]["builds"] = serde_json::json!([build]);
            assert!(refused(&value).contains("build id"));
        }
        let mut value = sample();
        value["game"]["builds"] = serde_json::json!([]);
        assert!(Manifest::parse(&value.to_string()).is_ok());
    }
}
