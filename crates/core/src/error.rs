//! Everything that can go wrong in the launcher, as one type.

use std::path::PathBuf;

/// The result type of this crate.
pub type Result<T> = std::result::Result<T, CoreError>;

/// A failure of one launcher step. The text of each variant is shown to the
/// player as it is, so it is written as a full sentence without jargon.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// A file or directory operation failed.
    #[error("Could not {operation} ({}): {source}", crate::files::display(.path))]
    Io {
        /// What was being done, as a verb phrase ("write the state file").
        operation: &'static str,
        /// The path it was done to.
        path: PathBuf,
        /// The error of the operating system.
        #[source]
        source: std::io::Error,
    },

    /// A request failed, timed out or was answered with an error status.
    #[error("Could not {operation}: {message}")]
    Http {
        /// What was being fetched ("read the manifest").
        operation: &'static str,
        /// Why it failed.
        message: String,
    },

    /// A URL that is not https (plain http is accepted for loopback only).
    #[error("Refused a URL that is not https: {0}")]
    InsecureUrl(String),

    /// The manifest is not what this launcher understands.
    #[error("The update information is not valid: {0}")]
    BadManifest(String),

    /// The manifest has no signature by a trusted key, or the signature is
    /// over other bytes than the ones that arrived.
    #[error("The update information is not signed with an OA Mod key, so it was ignored")]
    BadSignature,

    /// The manifest asks for a newer launcher than this one.
    #[error("This launcher is too old for the newest OA Mod version (needs launcher {required})")]
    LauncherTooOld {
        /// The lowest launcher version the manifest accepts.
        required: String,
    },

    /// The download has a different length than the manifest says.
    #[error("The download is {actual} bytes, the release says {expected}")]
    SizeMismatch {
        /// Bytes the manifest names.
        expected: u64,
        /// Bytes that arrived.
        actual: u64,
    },

    /// The download has a different SHA-256 than the manifest says.
    #[error("The download does not match its checksum, so it was not installed")]
    HashMismatch {
        /// The digest the manifest names.
        expected: String,
        /// The digest of the bytes that arrived.
        actual: String,
    },

    /// The zip file is damaged or contains an entry that must not be unpacked.
    #[error("The downloaded archive was refused: {0}")]
    Archive(String),

    /// Another step is already in progress.
    #[error("The launcher is busy with another step")]
    Busy,

    /// No Steam installation was found on this computer.
    #[error("Steam was not found on this computer")]
    SteamNotFound,

    /// Steam is there, but none of its libraries holds the game.
    #[error("Golf With Your Friends was not found in a Steam library")]
    GameNotFound,

    /// A file of Steam's own bookkeeping could not be understood.
    #[error("A Steam file could not be read ({}): {reason}", crate::files::display(.path))]
    BadSteamFile {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },

    /// A folder that is not an installed copy of the game.
    #[error("This is not a Golf With Your Friends folder ({}): {reason}", crate::files::display(.path))]
    BadGameFolder {
        /// The folder that was offered.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },

    /// The game is running, so its folder must not be changed.
    #[error("Golf With Your Friends is running. Close the game first")]
    GameRunning,

    /// Steam is downloading, updating or checking the game.
    #[error("Steam is busy with Golf With Your Friends. Wait until it has finished")]
    SteamBusy,

    /// The game folder refuses a write. The launcher never asks for more
    /// rights than the player has.
    #[error("The game folder is not writable ({})", crate::files::display(.path))]
    NotWritable {
        /// The file or folder that could not be written.
        path: PathBuf,
    },

    /// A path that could lead out of the folder it belongs to. Nothing is
    /// written to it and nothing is deleted through it.
    #[error("Refused to touch {}: {reason}", crate::files::display(.path))]
    UnsafePath {
        /// The path.
        path: PathBuf,
        /// Why it is refused.
        reason: String,
    },

    /// A file BepInEx would be written to is already there, so nothing was
    /// written.
    #[error("A file is in the way, so nothing was installed ({})", crate::files::display(.path))]
    Conflict {
        /// The file that exists.
        path: PathBuf,
    },

    /// The game folder holds a mod loader the launcher cannot build on.
    #[error("The mod loader in the game folder cannot be used: {0}")]
    UnsupportedBepInEx(String),

    /// A copy made before deleting does not match what it was made from, so
    /// nothing was deleted.
    #[error("The backup is not complete, so nothing was removed ({})", crate::files::display(.path))]
    BackupIncomplete {
        /// The backup folder.
        path: PathBuf,
    },

    /// The launcher's own settings file cannot be read.
    #[error("The launcher's settings could not be opened: {0}")]
    State(String),

    /// Steam could not be asked to start the game.
    #[error("Could not ask Steam to start the game: {0}")]
    Start(String),

    /// Something this build cannot do on the system it runs on.
    #[error("{0} is not supported on this system")]
    Unsupported(&'static str),
}

impl CoreError {
    /// Shorthand for the `Io` variant.
    pub(crate) fn io(
        operation: &'static str,
        path: &std::path::Path,
    ) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.to_path_buf();
        move |source| Self::Io {
            operation,
            path,
            source,
        }
    }
}
