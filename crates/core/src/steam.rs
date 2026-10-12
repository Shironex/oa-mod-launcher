//! Finding the game through Steam's own bookkeeping.
//!
//! Steam knows where every game is: `steamapps/libraryfolders.vdf` under its
//! root lists the libraries, and each library holds one `appmanifest_<id>.acf`
//! per installed game. Both are text files in Valve's key and value format,
//! which is read here by a small tokenizer.
//!
//! Everything in this module only reads. Nothing in a Steam folder is written.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

/// The Steam app id of Golf With Your Friends.
pub const APP_ID: &str = "431240";

/// Where Steam installs itself unless told otherwise.
#[cfg(windows)]
const DEFAULT_ROOT: &str = r"C:\Program Files (x86)\Steam";

/// The largest Steam file that is read. A library list with hundreds of games
/// is a few kilobytes.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// How deep blocks may nest. Steam's files use three levels.
const MAX_DEPTH: usize = 32;

/// What Steam says about the installed game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The game folder, `<library>/steamapps/common/<installdir>`. Not checked
    /// here: [`crate::game::validate`] decides whether it really is the game.
    pub dir: PathBuf,
    /// Steam's build id of the installed game, digits as text.
    pub build_id: String,
    /// Steam's `StateFlags` as they are. 4 means fully installed, anything
    /// else that Steam is still downloading, updating or checking.
    pub state_flags: u32,
}

/// The folder Steam is installed in: the current user's registry value, then
/// the machine wide one, then the default path. A candidate counts only when
/// it holds a `steamapps` folder.
///
/// # Errors
///
/// [`CoreError::SteamNotFound`] when no candidate is a Steam folder.
#[cfg(windows)]
pub fn steam_root() -> Result<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let read = |hive, key: &str, value: &str| {
        RegKey::predef(hive)
            .open_subkey(key)
            .and_then(|key| key.get_value::<String, _>(value))
            .ok()
            .map(PathBuf::from)
    };

    [
        read(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
        read(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
        ),
        Some(PathBuf::from(DEFAULT_ROOT)),
    ]
    .into_iter()
    .flatten()
    .find(|candidate| candidate.join("steamapps").is_dir())
    .ok_or(CoreError::SteamNotFound)
}

/// Not on this system: the launcher only looks for Steam on Windows.
///
/// # Errors
///
/// Always [`CoreError::Unsupported`].
#[cfg(not(windows))]
pub fn steam_root() -> Result<PathBuf> {
    Err(CoreError::Unsupported("Finding Steam"))
}

/// Find the game in the libraries of the Steam installation at `root`.
///
/// Libraries whose `apps` list names the game are asked first. Every library
/// is then probed for the game's manifest anyway, and so is `root` itself,
/// because the list can be stale or missing.
///
/// # Errors
///
/// [`CoreError::GameNotFound`] when no library holds the game's manifest,
/// [`CoreError::BadSteamFile`] when a manifest is there but cannot be used and
/// no other library has a usable one.
pub fn find_game(root: &Path) -> Result<Found> {
    let list = root.join("steamapps").join("libraryfolders.vdf");
    // A missing or damaged list is not an error: the root is still a library.
    let listed = read_capped(&list)
        .and_then(|text| libraries(&text).map_err(|reason| bad_file(&list, &reason)))
        .unwrap_or_default();

    let (claimed, others): (Vec<_>, Vec<_>) = listed.into_iter().partition(|library| library.1);
    let candidates = claimed
        .into_iter()
        .chain(others)
        .map(|library| library.0)
        .chain([root.to_path_buf()]);

    // A manifest that cannot be used does not end the search: a leftover file
    // in one library must not hide the game in the next. It is reported only
    // when no library yields the game.
    let mut first_bad = None;
    for library in candidates {
        let steamapps = library.join("steamapps");
        let manifest = steamapps.join(format!("appmanifest_{APP_ID}.acf"));
        if !manifest.is_file() {
            continue;
        }
        match read_capped(&manifest)
            .and_then(|text| app_manifest(&text).map_err(|reason| bad_file(&manifest, &reason)))
        {
            Ok(app) => {
                return Ok(Found {
                    dir: steamapps.join("common").join(app.install_dir),
                    build_id: app.build_id,
                    state_flags: app.state_flags,
                });
            }
            Err(error) => {
                first_bad.get_or_insert(error);
            }
        }
    }
    Err(first_bad.unwrap_or(CoreError::GameNotFound))
}

fn bad_file(path: &Path, reason: &str) -> CoreError {
    CoreError::BadSteamFile {
        path: path.to_path_buf(),
        reason: reason.to_owned(),
    }
}

/// Read a Steam file as text, refusing one that is far too large to be real.
fn read_capped(path: &Path) -> Result<String> {
    let size = std::fs::metadata(path)
        .map_err(CoreError::io("read a Steam file", path))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(bad_file(path, "it is too large"));
    }
    let bytes = std::fs::read(path).map_err(CoreError::io("read a Steam file", path))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The libraries a `libraryfolders.vdf` lists, in file order, each with
/// whether its `apps` list names the game.
fn libraries(text: &str) -> std::result::Result<Vec<(PathBuf, bool)>, String> {
    let document = parse(text)?;
    let folders = document
        .get("libraryfolders")
        .and_then(Value::as_map)
        .ok_or("it has no \"libraryfolders\" block")?;

    let mut found = Vec::new();
    for (key, value) in folders {
        if key.is_empty() || !key.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        match value {
            // The current format: one block per library.
            Value::Map(_) => {
                if let Some(path) = value.get("path").and_then(Value::as_text) {
                    let claimed = value
                        .get("apps")
                        .is_some_and(|apps| apps.get(APP_ID).is_some());
                    found.push((PathBuf::from(path), claimed));
                }
            }
            // Before 2021 a library was only a path.
            Value::Text(path) => found.push((PathBuf::from(path), false)),
        }
    }
    Ok(found)
}

/// The three values the launcher reads from an `appmanifest_<id>.acf`.
#[derive(Debug, PartialEq, Eq)]
struct AppManifest {
    install_dir: String,
    build_id: String,
    state_flags: u32,
}

fn app_manifest(text: &str) -> std::result::Result<AppManifest, String> {
    let document = parse(text)?;
    let state = document
        .get("AppState")
        .ok_or("it has no \"AppState\" block")?;
    let value = |key: &str| {
        state
            .get(key)
            .and_then(Value::as_text)
            .ok_or_else(|| format!("it has no \"{key}\" value"))
    };

    let install_dir = value("installdir")?;
    if !is_bare_name(install_dir) {
        return Err(format!(
            "\"installdir\" is not a plain folder name: {install_dir:?}"
        ));
    }
    let build_id = value("buildid")?;
    if build_id.is_empty() || !build_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("\"buildid\" is not a number: {build_id:?}"));
    }
    let state_flags = value("StateFlags")?;
    let state_flags = state_flags
        .parse()
        .map_err(|_| format!("\"StateFlags\" is not a number: {state_flags:?}"))?;

    Ok(AppManifest {
        install_dir: install_dir.to_owned(),
        build_id: build_id.to_owned(),
        state_flags,
    })
}

/// Whether `name` is one folder name and nothing else. It is joined to a
/// library path, so a separator, a drive letter or a `..` would point the
/// launcher at a folder Steam never named. Both kinds of slash are refused on
/// every system. A name Windows would quietly alter (a trailing dot or space)
/// is refused too, because it would not name the folder it spells, and so is
/// a device name (`CON`, `NUL`, `COM1` and the like, with or without an
/// extension), which Windows never opens as a file.
///
/// The same rule judges every part of an entry name of a downloaded zip.
pub(crate) fn is_bare_name(name: &str) -> bool {
    !name.is_empty()
        && !is_device_name(name)
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name.starts_with(' ')
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
}

/// Whether Windows reads `name` as a device: the part before the first dot,
/// without trailing spaces and in any case, is one of the reserved names.
fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let stem = stem.to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|rest| {
            let mut rest = rest.chars();
            matches!(
                (rest.next(), rest.next()),
                (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
            )
        })
    };
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT")
}

/// One value of Valve's key and value text format.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    /// A block. A list and not a map: the files are small, and the order and
    /// spelling of the keys stay as written.
    Map(Vec<(String, Value)>),
}

impl Value {
    /// The value of `key` in this block. Steam does not care about the case of
    /// a key, and the first match wins.
    fn get(&self, key: &str) -> Option<&Value> {
        self.as_map()?
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    }

    fn as_map(&self) -> Option<&[(String, Value)]> {
        match self {
            Self::Map(pairs) => Some(pairs),
            Self::Text(_) => None,
        }
    }

    fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Map(_) => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Text(String),
    Open,
    Close,
}

/// Split `text` into strings and braces. A string is quoted (with `\\`, `\"`,
/// `\n` and `\t` escapes) or a bare run of characters. `//` starts a comment
/// that runs to the end of the line.
fn tokens(text: &str) -> std::result::Result<Vec<Token>, String> {
    let mut found = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '{' => found.push(Token::Open),
            '}' => found.push(Token::Close),
            '/' if chars.peek() == Some(&'/') => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut value = String::new();
                loop {
                    match chars.next() {
                        None => return Err("a quoted text never ends".to_owned()),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            None => return Err("a quoted text never ends".to_owned()),
                            Some('n') => value.push('\n'),
                            Some('t') => value.push('\t'),
                            Some(escaped @ ('\\' | '"')) => value.push(escaped),
                            // Not an escape Steam writes: keep it as it is.
                            Some(other) => {
                                value.push('\\');
                                value.push(other);
                            }
                        },
                        Some(other) => value.push(other),
                    }
                }
                found.push(Token::Text(value));
            }
            first => {
                let mut value = String::from(first);
                while let Some(next) =
                    chars.next_if(|next| !next.is_whitespace() && !matches!(next, '{' | '}' | '"'))
                {
                    value.push(next);
                }
                found.push(Token::Text(value));
            }
        }
    }
    Ok(found)
}

/// Parse a whole file into one block of its top level keys.
fn parse(text: &str) -> std::result::Result<Value, String> {
    let tokens = tokens(text)?;
    let mut position = 0;
    let pairs = block(&tokens, &mut position, 0)?;
    if position < tokens.len() {
        return Err("a block is closed that was never opened".to_owned());
    }
    Ok(Value::Map(pairs))
}

/// The pairs of one block, up to its closing brace or the end of the file.
fn block(
    tokens: &[Token],
    position: &mut usize,
    depth: usize,
) -> std::result::Result<Vec<(String, Value)>, String> {
    if depth > MAX_DEPTH {
        return Err("its blocks are nested too deep".to_owned());
    }
    let mut pairs = Vec::new();
    loop {
        let key = match tokens.get(*position) {
            None if depth == 0 => return Ok(pairs),
            None => return Err("a block is never closed".to_owned()),
            // At the top level this stops the walk and `parse` reports it.
            Some(Token::Close) => {
                if depth > 0 {
                    *position += 1;
                }
                return Ok(pairs);
            }
            Some(Token::Open) => return Err("a block has no name".to_owned()),
            Some(Token::Text(key)) => key.clone(),
        };
        *position += 1;
        let value = match tokens.get(*position) {
            Some(Token::Text(text)) => {
                *position += 1;
                Value::Text(text.clone())
            }
            Some(Token::Open) => {
                *position += 1;
                Value::Map(block(tokens, position, depth + 1)?)
            }
            Some(Token::Close) | None => return Err(format!("\"{key}\" has no value")),
        };
        pairs.push((key, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A library list in the shape Steam writes: tabs, one block per library,
    /// Windows paths with doubled backslashes. The game is in the second one.
    const TWO_LIBRARIES: &str = "\"libraryfolders\"\r\n{\r\n\
        \t\"0\"\r\n\t{\r\n\
        \t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\r\n\
        \t\t\"label\"\t\t\"\"\r\n\
        \t\t\"contentid\"\t\t\"1111111111111111111\"\r\n\
        \t\t\"totalsize\"\t\t\"0\"\r\n\
        \t\t\"apps\"\r\n\t\t{\r\n\
        \t\t\t\"228980\"\t\t\"559537066\"\r\n\
        \t\t\t\"4312400\"\t\t\"1\"\r\n\
        \t\t}\r\n\t}\r\n\
        \t\"1\"\r\n\t{\r\n\
        \t\t\"path\"\t\t\"D:\\\\Games\\\\Steam Library\"\r\n\
        \t\t\"label\"\t\t\"\"\r\n\
        \t\t\"contentid\"\t\t\"2222222222222222222\"\r\n\
        \t\t\"totalsize\"\t\t\"5802520735744\"\r\n\
        \t\t\"apps\"\r\n\t\t{\r\n\
        \t\t\t\"730\"\t\t\"73931747945\"\r\n\
        \t\t\t\"431240\"\t\t\"6018004668\"\r\n\
        \t\t}\r\n\t}\r\n\
        }\r\n";

    /// The game's manifest in the shape Steam writes, with invented ids.
    fn manifest_text(install_dir: &str) -> String {
        format!(
            "\"AppState\"\n{{\n\
             \t\"appid\"\t\t\"431240\"\n\
             \t\"Universe\"\t\t\"1\"\n\
             \t\"LauncherPath\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\\\\steam.exe\"\n\
             \t\"name\"\t\t\"Golf With Your Friends\"\n\
             \t\"StateFlags\"\t\t\"4\"\n\
             \t\"installdir\"\t\t\"{install_dir}\"\n\
             \t\"LastUpdated\"\t\t\"1786966337\"\n\
             \t\"SizeOnDisk\"\t\t\"6018004668\"\n\
             \t\"buildid\"\t\t\"20236533\"\n\
             \t\"LastOwner\"\t\t\"12345678901234567\"\n\
             \t\"InstalledDepots\"\n\t{{\n\
             \t\t\"431241\"\n\t\t{{\n\
             \t\t\t\"manifest\"\t\t\"1234567890123456789\"\n\
             \t\t\t\"size\"\t\t\"6018004668\"\n\
             \t\t}}\n\t}}\n\
             \t\"UserConfig\"\n\t{{\n\
             \t\t\"language\"\t\t\"english\"\n\
             \t}}\n\
             }}\n"
        )
    }

    #[test]
    fn a_library_list_gives_every_path_and_who_has_the_game() {
        assert_eq!(
            libraries(TWO_LIBRARIES),
            Ok(vec![
                (PathBuf::from(r"C:\Program Files (x86)\Steam"), false),
                (PathBuf::from(r"D:\Games\Steam Library"), true),
            ]),
            "the doubled backslashes are single again, and app 4312400 is not app 431240"
        );
    }

    #[test]
    fn a_library_list_without_the_game_claims_nothing() {
        let text = TWO_LIBRARIES.replace("\"431240\"", "\"440\"");

        let found = libraries(&text).expect("the list parses");

        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|library| !library.1));
    }

    #[test]
    fn the_old_library_list_format_is_read_too() {
        let text = "\"LibraryFolders\"\n{\n\
            \t\"TimeNextStatsReport\"\t\t\"1600000000\"\n\
            \t\"ContentStatsID\"\t\t\"-1\"\n\
            \t\"1\"\t\t\"D:\\\\SteamLibrary\"\n\
            }\n";

        assert_eq!(
            libraries(text),
            Ok(vec![(PathBuf::from(r"D:\SteamLibrary"), false)])
        );
    }

    #[test]
    fn a_file_that_is_not_a_library_list_is_refused() {
        for text in [
            "",
            "\"AppState\" { }",
            "\"libraryfolders\" \"text\"",
            "\"libraryfolders\" {",
            "\"libraryfolders\" { \"0\" { \"path\" } }",
            "\"libraryfolders\" { } }",
            "{ }",
            "\"libraryfolders\" { \"0\" { \"path\" \"C:\\\\Steam",
        ] {
            assert!(libraries(text).is_err(), "{text:?} must be refused");
        }
    }

    #[test]
    fn the_manifest_gives_the_folder_the_build_and_the_state() {
        assert_eq!(
            app_manifest(&manifest_text("Golf With Your Friends")),
            Ok(AppManifest {
                install_dir: "Golf With Your Friends".to_owned(),
                build_id: "20236533".to_owned(),
                state_flags: 4,
            })
        );
    }

    #[test]
    fn a_hostile_installdir_is_refused() {
        for hostile in [
            "",
            ".",
            "..",
            "..\\\\..\\\\Windows",
            "../../Windows",
            "common/Other Game",
            "Golf\\\\..\\\\Other",
            "C:\\\\Windows",
            "C:",
            "Golf.",
            "Golf ",
            " Golf",
            "Golf\\nWith",
            "Golf|With",
            "CON",
            "nul.d",
            "COM1",
            "lpt9",
        ] {
            let result = app_manifest(&manifest_text(hostile));
            assert!(
                result
                    .as_ref()
                    .is_err_and(|reason| reason.contains("installdir")),
                "{hostile:?} must be refused, got {result:?}"
            );
        }
    }

    #[test]
    fn a_manifest_with_a_missing_or_odd_value_is_refused() {
        let good = manifest_text("Golf With Your Friends");
        for (from, to) in [
            ("\"buildid\"", "\"build\""),
            ("\"20236533\"", "\"20236533a\""),
            ("\"StateFlags\"\t\t\"4\"", "\"StateFlags\"\t\t\"-4\""),
            ("\"StateFlags\"", "\"Flags\""),
            ("\"installdir\"", "\"dir\""),
            ("\"AppState\"", "\"State\""),
        ] {
            assert!(good.contains(from));
            assert!(
                app_manifest(&good.replace(from, to)).is_err(),
                "{from} as {to} must be refused"
            );
        }
    }

    #[test]
    fn keys_match_in_any_case_and_other_state_flags_pass_through() {
        let text = manifest_text("Golf With Your Friends")
            .replace("\"StateFlags\"\t\t\"4\"", "\"stateflags\"\t\t\"1026\"")
            .replace("\"buildid\"", "\"BuildID\"");

        let app = app_manifest(&text).expect("the manifest parses");

        assert_eq!(app.state_flags, 1026);
        assert_eq!(app.build_id, "20236533");
    }

    #[test]
    fn the_tokenizer_reads_quotes_escapes_bare_words_and_comments() {
        let text = "// a comment\n\"a b\"\t{ bare \"x\\\\y \\\"z\\\" \\n\\t \\q\" } // tail";

        assert_eq!(
            tokens(text),
            Ok(vec![
                Token::Text("a b".to_owned()),
                Token::Open,
                Token::Text("bare".to_owned()),
                Token::Text("x\\y \"z\" \n\t \\q".to_owned()),
                Token::Close,
            ])
        );
        assert!(tokens("\"open").is_err());
        assert!(tokens("\"open\\").is_err());
    }

    #[test]
    fn blocks_nested_too_deep_are_refused_without_a_crash() {
        let deep = "\"a\" { ".repeat(100_000);

        assert!(parse(&deep).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn the_steam_root_is_a_steam_folder_or_nothing() {
        // Reads the real registry: a PC with Steam answers with its folder, a
        // build runner without Steam with the error.
        match steam_root() {
            Ok(root) => assert!(root.join("steamapps").is_dir()),
            Err(error) => assert!(matches!(error, CoreError::SteamNotFound)),
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn finding_steam_is_not_supported_here() {
        assert!(matches!(steam_root(), Err(CoreError::Unsupported(_))));
    }
}
