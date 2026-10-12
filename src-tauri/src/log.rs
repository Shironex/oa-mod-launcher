//! `launcher.log`: one line per thing the launcher did.
//!
//! Plain text, appended, never sent anywhere. It is the file to ask a friend
//! for when an install did not work.

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use oa_mod_launcher_core::Layout;

/// The log is started again once it grows past this.
const MAX_BYTES: u64 = 512 * 1024;

/// Append `message` with a UTC time. Failures are ignored: a launcher that
/// cannot write its log must still work.
pub(crate) fn write(layout: &Layout, message: &str) {
    let path = layout.launcher_log();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_BYTES) {
        let _ = std::fs::remove_file(&path);
    }
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{} {message}", utc_text(seconds));
    }
}

/// `seconds` since 1970 as `YYYY-MM-DD HH:MM:SSZ`.
///
/// The date part is the usual days-to-civil conversion: shift the epoch to
/// 0000-03-01 so that a leap day is the last day of a year, then split the day
/// count into 400 year eras.
fn utc_text(seconds: u64) -> String {
    let days = seconds / 86_400;
    let rest = seconds % 86_400;

    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);

    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_moments_are_formatted_correctly() {
        assert_eq!(utc_text(0), "1970-01-01 00:00:00Z");
        assert_eq!(utc_text(951_782_400), "2000-02-29 00:00:00Z");
        assert_eq!(utc_text(1_709_251_199), "2024-02-29 23:59:59Z");
        assert_eq!(utc_text(1_709_251_200), "2024-03-01 00:00:00Z");
        assert_eq!(utc_text(1_791_280_496), "2026-10-06 09:54:56Z");
    }

    #[test]
    fn lines_are_appended() {
        let temp = std::env::temp_dir().join(format!("oa-launcher-log-{}", std::process::id()));
        let layout = Layout::new(&temp);
        write(&layout, "first");
        write(&layout, "second");
        let text = std::fs::read_to_string(layout.launcher_log()).expect("the log exists");
        let _ = std::fs::remove_dir_all(&temp);

        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("Z first"));
        assert!(lines[1].ends_with("Z second"));
    }
}
