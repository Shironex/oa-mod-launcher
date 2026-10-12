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
    let steamapps = library.join("steamapps");
    std::fs::create_dir_all(&steamapps).expect("the steamapps folder");
    let text = format!(
        "\"AppState\"\n{{\n\t\"appid\"\t\t\"{}\"\n\t\"StateFlags\"\t\t\"4\"\n\
         \t\"installdir\"\t\t\"{install_dir}\"\n\t\"buildid\"\t\t\"20236533\"\n}}\n",
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
