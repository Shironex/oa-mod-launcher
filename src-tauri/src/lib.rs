//! The window around the launcher rules.
//!
//! This crate decides nothing. For now it only opens the window: the commands
//! and events that connect the page to the core come later.

/// Build and run the application.
///
/// # Panics
///
/// When the window system cannot be started at all. There is nothing to show
/// an error in at that point.
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("the window system starts");
}
