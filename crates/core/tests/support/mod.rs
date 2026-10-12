//! What the integration tests share: a loopback HTTP server, a key that
//! signs what it serves, and a fake Steam installation with the game in it.
//!
//! The server is deliberately not a real HTTP implementation. It reads one
//! request, answers with whatever the test registered for that path, and
//! closes the connection.
//!
//! Nothing is signed ahead of time: every server makes a throwaway key pair
//! and signs what it publishes, so a test trusts exactly that key.

#![allow(dead_code, reason = "each test uses a different subset")]

use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use oa_mod_launcher_core::{game, steam};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// What the server does for one path.
#[derive(Clone)]
pub(crate) enum Reply {
    /// A 200 with these bytes.
    Body(Vec<u8>),
    /// A redirect to `location`.
    Redirect(String),
    /// A status with an empty body.
    Status(u16),
    /// Accept the request and never answer.
    Hang,
}

impl Reply {
    fn render(&self) -> Vec<u8> {
        match self {
            Self::Body(body) => {
                let mut raw = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                raw.extend_from_slice(body);
                raw
            }
            Self::Redirect(location) => format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\
                 Connection: close\r\n\r\n"
            )
            .into_bytes(),
            Self::Status(status) => {
                format!("HTTP/1.1 {status} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .into_bytes()
            }
            Self::Hang => Vec::new(),
        }
    }
}

/// A throwaway signing key, in the formats of the Tauri CLI: what
/// `tauri signer generate` and `tauri signer sign` write is base64 of the
/// minisign text.
pub(crate) struct Signer {
    pair: minisign::KeyPair,
}

impl Signer {
    /// A new key pair. Without a password: the password only protects the
    /// private key on disk, and applying it is slow.
    pub(crate) fn new() -> Self {
        Self {
            pair: minisign::KeyPair::generate_unencrypted_keypair().expect("a key pair"),
        }
    }

    /// The content of the `.pub` file of this key.
    pub(crate) fn public_key(&self) -> String {
        let text = self.pair.pk.to_box().expect("a public key box").to_string();
        STANDARD.encode(text)
    }

    /// The content of the `.sig` file for `bytes`.
    pub(crate) fn sign(&self, bytes: &[u8]) -> String {
        let signature = minisign::sign(
            None,
            &self.pair.sk,
            Cursor::new(bytes),
            Some("timestamp:1791280496\tfile:fixture"),
            Some("signature from tauri secret key"),
        )
        .expect("a signature");
        STANDARD.encode(signature.to_string())
    }
}

type Routes = Arc<Mutex<HashMap<String, Reply>>>;

/// Replies that are given once, in order, before the lasting route of a path.
type FirstReplies = Arc<Mutex<HashMap<String, VecDeque<Reply>>>>;

/// A server on an ephemeral loopback port. It stops when it is dropped.
pub(crate) struct TestServer {
    address: SocketAddr,
    routes: Routes,
    first: FirstReplies,
    signer: Signer,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestServer {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("binding an ephemeral loopback port");
        let address = listener
            .local_addr()
            .expect("the bound address is readable");
        let routes: Routes = Arc::default();
        let first: FirstReplies = Arc::default();
        let requests: Arc<Mutex<Vec<String>>> = Arc::default();

        let (served, once, seen) = (
            Arc::clone(&routes),
            Arc::clone(&first),
            Arc::clone(&requests),
        );
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let (served, once, seen) =
                    (Arc::clone(&served), Arc::clone(&once), Arc::clone(&seen));
                tokio::spawn(async move {
                    let path = request_path(&read_head(&mut stream).await).unwrap_or_default();
                    seen.lock().expect("the request list").push(path.clone());
                    let queued = once
                        .lock()
                        .expect("the first replies")
                        .get_mut(&path)
                        .and_then(VecDeque::pop_front);
                    let reply = queued.unwrap_or_else(|| {
                        served
                            .lock()
                            .expect("the routes")
                            .get(&path)
                            .cloned()
                            .unwrap_or(Reply::Status(404))
                    });
                    if matches!(reply, Reply::Hang) {
                        tokio::time::sleep(Duration::from_secs(600)).await;
                        return;
                    }
                    let _ = stream.write_all(&reply.render()).await;
                    let _ = stream.flush().await;
                    let _ = stream.shutdown().await;
                });
            }
        });

        Self {
            address,
            routes,
            first,
            signer: Signer::new(),
            requests,
            task,
        }
    }

    /// The public key of this server's signing key, as a `.pub` file holds it.
    pub(crate) fn public_key(&self) -> String {
        self.signer.public_key()
    }

    /// What this server's key signs `bytes` with, as a `.sig` file holds it.
    pub(crate) fn sign(&self, bytes: &[u8]) -> String {
        self.signer.sign(bytes)
    }

    /// Answer `path` with `body` and `path.sig` with its signature, from now
    /// on.
    pub(crate) fn route_signed(&self, path: &str, body: &[u8]) {
        self.route(path, Reply::Body(body.to_vec()));
        self.route(
            &format!("{path}.sig"),
            Reply::Body(self.sign(body).into_bytes()),
        );
    }

    /// Answer the next request for `path` with `reply`, and go back to the
    /// lasting route after it. A file that changes between two requests is
    /// what a launcher sees while a release is being published.
    pub(crate) fn route_once(&self, path: &str, reply: Reply) {
        self.first
            .lock()
            .expect("the first replies")
            .entry(path.to_owned())
            .or_default()
            .push_back(reply);
    }

    /// Answer `path` with `reply` from now on.
    pub(crate) fn route(&self, path: &str, reply: Reply) {
        self.routes
            .lock()
            .expect("the routes")
            .insert(path.to_owned(), reply);
    }

    /// The URL of `path` on this server.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /// Every request path received so far.
    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("the request list").clone()
    }
}

async fn read_head(stream: &mut tokio::net::TcpStream) -> String {
    let mut raw = Vec::new();
    let mut scratch = [0_u8; 1024];
    while !raw.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut scratch).await {
            Ok(0) | Err(_) => break,
            Ok(read) => raw.extend_from_slice(&scratch[..read]),
        }
    }
    String::from_utf8_lossy(&raw).into_owned()
}

fn request_path(request: &str) -> Option<String> {
    request
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)
        .map(str::to_owned)
}

/// `path` the way Steam writes it into its files: every backslash doubled.
pub(crate) fn vdf_escaped(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

/// Write a `libraryfolders.vdf` under the Steam folder `root` that lists
/// `libraries`, each with the app ids Steam believes it holds.
pub(crate) fn write_library_list(root: &Path, libraries: &[(&Path, &[&str])]) {
    let mut text = String::from("\"libraryfolders\"\n{\n");
    for (index, (path, apps)) in libraries.iter().enumerate() {
        text.push_str(&format!(
            "\t\"{index}\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"apps\"\n\t\t{{\n",
            vdf_escaped(path)
        ));
        for app in *apps {
            text.push_str(&format!("\t\t\t\"{app}\"\t\t\"1024\"\n"));
        }
        text.push_str("\t\t}\n\t}\n");
    }
    text.push_str("}\n");
    let steamapps = root.join("steamapps");
    std::fs::create_dir_all(&steamapps).expect("the steamapps folder");
    std::fs::write(steamapps.join("libraryfolders.vdf"), text).expect("the library list");
}

/// Write the game's `appmanifest` into `library`, naming `install_dir` as
/// written (so a test can pass a hostile one).
pub(crate) fn write_app_manifest(library: &Path, install_dir: &str) {
    write_app_manifest_in_state(library, install_dir, 4);
}

/// Like [`write_app_manifest`], with Steam's `StateFlags` as given. 4 is
/// "fully installed"; 1026 is what an update in progress looks like.
pub(crate) fn write_app_manifest_in_state(library: &Path, install_dir: &str, state_flags: u32) {
    let steamapps = library.join("steamapps");
    std::fs::create_dir_all(&steamapps).expect("the steamapps folder");
    let text = format!(
        "\"AppState\"\n{{\n\t\"appid\"\t\t\"{}\"\n\t\"StateFlags\"\t\t\"{state_flags}\"\n\
         \t\"installdir\"\t\t\"{install_dir}\"\n\t\"buildid\"\t\t\"{BUILD_ID}\"\n}}\n",
        steam::APP_ID
    );
    std::fs::write(
        steamapps.join(format!("appmanifest_{}.acf", steam::APP_ID)),
        text,
    )
    .expect("the app manifest");
}

/// A fake installed game in `library`: its manifest, and a game folder with
/// the two files that make it the game. Returns the game folder.
pub(crate) fn fake_game_folder(library: &Path) -> PathBuf {
    const INSTALL_DIR: &str = "Golf With Your Friends";
    write_app_manifest(library, INSTALL_DIR);
    let dir = library.join("steamapps").join("common").join(INSTALL_DIR);
    std::fs::create_dir_all(&dir).expect("the game folder");
    std::fs::write(dir.join(game::EXE), b"MZ").expect("the game exe");
    std::fs::write(dir.join(game::ASSEMBLY), b"MZ").expect("the game assembly");
    dir
}

/// The Steam build id every fake app manifest names.
pub(crate) const BUILD_ID: &str = "20236533";

/// The install folder name every fake app manifest names.
pub(crate) const INSTALL_DIR: &str = "Golf With Your Friends";

/// Lowercase hex SHA-256 of `bytes`.
pub(crate) fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A zip holding `entries`, as bytes. A name that ends in `/` becomes a
/// folder entry without content, as the real BepInEx zip has them.
pub(crate) fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in entries {
        if name.ends_with('/') {
            writer
                .add_directory(*name, options)
                .expect("a folder entry");
            continue;
        }
        writer.start_file(*name, options).expect("start an entry");
        writer.write_all(body).expect("write an entry");
    }
    writer.finish().expect("finish the zip").into_inner()
}

/// A small stand-in for the BepInEx zip: the same top level as the real one
/// (two folders and four root files), with a few files in each folder.
pub(crate) const BEPINEX_FILES: [(&str, &[u8]); 9] = [
    ("BepInEx/core/BepInEx.Unity.IL2CPP.dll", b"MZ il2cpp 788"),
    ("BepInEx/core/BepInEx.Core.dll", b"MZ core 788"),
    ("BepInEx/core/0Harmony.dll", b"MZ harmony"),
    ("dotnet/coreclr.dll", b"MZ coreclr"),
    ("dotnet/shared/System.Runtime.dll", b"MZ runtime"),
    ("winhttp.dll", b"MZ proxy"),
    ("doorstop_config.ini", b"[General]\nenabled = true\n"),
    (".doorstop_version", b"4.3.0"),
    ("changelog.txt", b"be.788\n"),
];

/// The bytes of the file `name` in [`BEPINEX_FILES`].
pub(crate) fn bepinex_file(name: &str) -> &'static [u8] {
    BEPINEX_FILES
        .iter()
        .find(|file| file.0 == name)
        .expect("a file of the fake BepInEx")
        .1
}

/// Write `body` to `relative` under `root`, with the folders it needs.
pub(crate) fn put(root: &Path, relative: &str, body: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
    std::fs::write(path, body).expect("a file");
}

/// Put the fake BepInEx into `game` by hand, as if somebody had unpacked the
/// zip there before the launcher was ever started.
pub(crate) fn unpack_bepinex_by_hand(game: &Path) {
    for (name, body) in BEPINEX_FILES {
        put(game, name, body);
    }
}

/// Every file below `dir`, as paths relative to it with `/` between the
/// parts, sorted. What a test compares a folder before and after with.
pub(crate) fn listing(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, found: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, found);
            } else {
                let relative = path.strip_prefix(base).expect("below the base");
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut found = Vec::new();
    walk(dir, dir, &mut found);
    found.sort();
    found
}

/// One mod release as a server publishes it.
pub(crate) struct Release {
    pub(crate) version: String,
    pub(crate) mod_bytes: Vec<u8>,
    pub(crate) zip: Vec<u8>,
    pub(crate) builds: Vec<String>,
    pub(crate) launcher_min: String,
}

impl Release {
    /// A release of `version` with the fake BepInEx and a mod DLL whose bytes
    /// name the version.
    pub(crate) fn new(version: &str) -> Self {
        Self {
            version: version.to_owned(),
            mod_bytes: format!("MZ OaMod {version}").into_bytes(),
            zip: zip_of(&BEPINEX_FILES),
            builds: vec![BUILD_ID.to_owned()],
            launcher_min: "0.1.0".to_owned(),
        }
    }

    /// The manifest of this release for files served by `server`. The
    /// checksums of the files inside the zip are always the ones of the fake
    /// BepInEx, whatever zip a test put in `zip`.
    pub(crate) fn manifest(&self, server: &TestServer) -> serde_json::Value {
        let root_file = |name: &str| sha256(bepinex_file(name));
        serde_json::json!({
            "schema": 1,
            "version": self.version,
            "published": "2026-10-20T18:00:00Z",
            "notes": format!("About {}", self.version),
            "mod": {
                "url": server.url(&format!("/files/OaMod-{}.dll", self.version)),
                "size": self.mod_bytes.len(),
                "sha256": sha256(&self.mod_bytes),
            },
            "bepinex": {
                "label": "6.0.0-be.788",
                "url": server.url("/files/BepInEx.zip"),
                "size": self.zip.len(),
                "sha256": sha256(&self.zip),
                "probe_sha256": sha256(bepinex_file("BepInEx/core/BepInEx.Unity.IL2CPP.dll")),
                "root_files": {
                    "winhttp.dll": root_file("winhttp.dll"),
                    "doorstop_config.ini": root_file("doorstop_config.ini"),
                    ".doorstop_version": root_file(".doorstop_version"),
                    "changelog.txt": root_file("changelog.txt"),
                },
            },
            "game": { "builds": self.builds },
            "launcher": { "min": self.launcher_min },
        })
    }
}

impl TestServer {
    /// Publish `release`: its two files, and the manifest with a signature by
    /// this server's key. Returns the manifest as it was served.
    pub(crate) fn publish(&self, release: &Release) -> Vec<u8> {
        self.route(
            &format!("/files/OaMod-{}.dll", release.version),
            Reply::Body(release.mod_bytes.clone()),
        );
        self.route("/files/BepInEx.zip", Reply::Body(release.zip.clone()));
        let manifest = serde_json::to_vec_pretty(&release.manifest(self)).expect("JSON");
        self.route_signed("/manifest.json", &manifest);
        manifest
    }
}

/// Everything one flow test needs on disk, in one temporary folder: the
/// launcher's data root, a fake Steam with the fake game in it, a switch for
/// "the game is running" and a record of what the launcher asked to open.
///
/// Nothing here is the real game folder or the real Steam, and no test built
/// on it can reach them: the configuration names the fake Steam folder, so
/// the launcher never asks the system where Steam is.
pub(crate) struct Sandbox {
    _temp: tempfile::TempDir,
    /// The launcher's data root.
    pub(crate) data: PathBuf,
    /// The fake Steam folder, which is also its only library.
    pub(crate) steam: PathBuf,
    /// The fake game folder, canonical.
    pub(crate) game: PathBuf,
    /// Set it to make the launcher believe the game is running.
    pub(crate) running: Arc<std::sync::atomic::AtomicBool>,
    /// The addresses the launcher asked the system to open.
    pub(crate) opened: Arc<Mutex<Vec<String>>>,
}

impl Sandbox {
    pub(crate) fn new() -> Self {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let base = std::fs::canonicalize(temp.path()).expect("a real path");
        let steam = base.join("Steam");
        let game = std::fs::canonicalize(fake_game_folder(&steam)).expect("the game folder");
        Self {
            data: base.join("data"),
            steam,
            game,
            running: Arc::default(),
            opened: Arc::default(),
            _temp: temp,
        }
    }

    /// A configuration that reads the manifest of `server` and trusts its
    /// key.
    pub(crate) fn config(&self, server: &TestServer) -> oa_mod_launcher_core::Config {
        self.config_for(
            &server.url("/manifest.json"),
            &server.url("/files/"),
            vec![server.public_key()],
        )
    }

    /// A configuration whose manifest address answers nothing: port 9 on
    /// loopback, which no test listens on.
    pub(crate) fn config_offline(&self) -> oa_mod_launcher_core::Config {
        self.config_for(
            "http://127.0.0.1:9/manifest.json",
            "http://127.0.0.1:9/files/",
            Vec::new(),
        )
    }

    fn config_for(
        &self,
        manifest_url: &str,
        download_prefix: &str,
        trusted_keys: Vec<String>,
    ) -> oa_mod_launcher_core::Config {
        let running = Arc::clone(&self.running);
        let opened = Arc::clone(&self.opened);
        oa_mod_launcher_core::Config {
            layout: oa_mod_launcher_core::Layout::new(&self.data),
            manifest_url: manifest_url.to_owned(),
            download_prefix: download_prefix.to_owned(),
            launcher_version: oa_mod_launcher_core::Version::parse("0.1.0").expect("a version"),
            trusted_keys,
            steam_root: Some(self.steam.clone()),
            running: Arc::new(move |_| running.load(std::sync::atomic::Ordering::SeqCst)),
            open: Arc::new(move |address| {
                opened
                    .lock()
                    .expect("the opened list")
                    .push(address.to_owned());
                Ok(())
            }),
        }
    }

    /// The state file as it is on disk.
    pub(crate) fn saved_state(&self) -> oa_mod_launcher_core::State {
        oa_mod_launcher_core::State::load(&self.data.join("state.json")).expect("the state file")
    }
}

/// A junction on Windows, a symbolic link elsewhere: `link` leads to the
/// folder `target`. Neither needs special rights.
pub(crate) fn link_dir(target: &Path, link: &Path) {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("cmd runs");
        assert!(status.success(), "the junction is made");
    }
    #[cfg(not(windows))]
    std::os::unix::fs::symlink(target, link).expect("the link is made");
}
