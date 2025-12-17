//! Time parsing and formatting utilities for Jira datetime strings

use chrono::{DateTime, Local, NaiveDate};

/// Debug logging helper - only logs in debug builds
#[cfg(debug_assertions)]
fn debug_log(message: &str) {
    use std::io::Write;
    let log_path = std::env::temp_dir().join("time-tracker-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "{}", message);
    }
}

#[cfg(not(debug_assertions))]
fn debug_log(_message: &str) {}

/// Extract time as "HH:MM" from a Jira datetime string like "2025-12-02T09:00:00.000+0000"
/// Converts from the stored timezone to local time
pub fn extract_time(started: &str) -> String {
    debug_log(&format!("\n--- extract_time ---"));
    debug_log(&format!("Input: {}", started));

    // Normalize timezone offset: convert "+0800" to "+08:00" format for parsing
    let normalized = normalize_timezone_offset(started);
    debug_log(&format!("Normalized: {}", normalized));

    // Try parsing with milliseconds
    if let Ok(dt) = DateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M:%S%.3f%:z") {
        let local_time = dt.with_timezone(&Local);
        let result = local_time.format("%H:%M").to_string();
        debug_log(&format!("Parsed OK (with ms): {} -> local: {}", dt, result));
        return result;
    }

    // Fallback: try without milliseconds
    if let Ok(dt) = DateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M:%S%:z") {
        let local_time = dt.with_timezone(&Local);
        let result = local_time.format("%H:%M").to_string();
        debug_log(&format!("Parsed OK (no ms): {} -> local: {}", dt, result));
        return result;
    }

    debug_log("Parse FAILED, using fallback");

    // Last resort fallback - just extract raw time (no timezone conversion)
    if let Some(time_part) = started.split('T').nth(1) {
        let time_only = time_part.split('.').next().unwrap_or(time_part);
        let parts: Vec<&str> = time_only.split(':').collect();
        if parts.len() >= 2 {
            let result = format!("{}:{}", parts[0], parts[1]);
            debug_log(&format!("Fallback result: {}", result));
            return result;
        }
    }
    "99:99".to_string()
}

/// Parse date from a Jira datetime string like "2025-12-02T09:00:00.000+0000"
pub fn parse_date(started: &str) -> NaiveDate {
    let date_part = started.split('T').next().unwrap_or(started);
    NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
        .unwrap_or_else(|_| Local::now().date_naive())
}

/// Parse a user-entered start time string (e.g., "9:00am", "14:30", "2pm") to "HH:MM:SS" format
pub fn parse_start_time(input: &str) -> Option<String> {
    let input = input.trim().to_lowercase();
    if input.is_empty() {
        return None;
    }

    // Check for am/pm suffix
    let (time_part, is_pm) = if input.ends_with("pm") {
        (&input[..input.len() - 2], true)
    } else if input.ends_with("am") {
        (&input[..input.len() - 2], false)
    } else if input.ends_with("p") {
        (&input[..input.len() - 1], true)
    } else if input.ends_with("a") {
        (&input[..input.len() - 1], false)
    } else {
        (input.as_str(), false) // 24-hour format assumed
    };

    let time_part = time_part.trim();

    // Parse hour and optional minute
    let (hour, minute) = if time_part.contains(':') {
        let parts: Vec<&str> = time_part.split(':').collect();
        let h = parts[0].parse::<u32>().ok()?;
        let m = parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        (h, m)
    } else {
        let h = time_part.parse::<u32>().ok()?;
        (h, 0)
    };

    // Convert to 24-hour format if am/pm was specified
    let hour_24 = if is_pm && hour < 12 {
        hour + 12
    } else if !is_pm && hour == 12 && input.contains('a') {
        0 // 12am = midnight
    } else {
        hour
    };

    if hour_24 > 23 || minute > 59 {
        return None;
    }

    Some(format!("{:02}:{:02}:00", hour_24, minute))
}

/// Build a Jira-compatible "started" timestamp from date and optional start time
/// Format: "2025-12-02T09:00:00.000-0800"
pub fn build_jira_timestamp(date: NaiveDate, start_time: Option<&str>) -> String {
    // Parse start time or default to 09:00
    let time_str = start_time
        .and_then(parse_start_time)
        .unwrap_or_else(|| "09:00:00".to_string());

    // Get local timezone offset - Jira requires format: -0800 (no colon, zero-padded)
    let local_offset = Local::now().offset().local_minus_utc();
    let offset_hours = local_offset / 3600;
    let offset_mins = (local_offset.abs() % 3600) / 60;
    let sign = if local_offset >= 0 { '+' } else { '-' };
    let offset_str = format!("{}{:02}{:02}", sign, offset_hours.abs(), offset_mins);

    let started = format!("{}T{}.000{}", date.format("%Y-%m-%d"), time_str, offset_str);

    debug_log(&format!("\n--- build_jira_timestamp ---"));
    debug_log(&format!("Input start_time: {:?}", start_time));
    debug_log(&format!("Parsed time_str: {}", time_str));
    debug_log(&format!("Local offset (seconds): {}", local_offset));
    debug_log(&format!("Offset string: {}", offset_str));
    debug_log(&format!("Final 'started': {}", started));

    started
}

/// Normalize timezone offset from "+0800" to "+08:00" format for chrono parsing
fn normalize_timezone_offset(started: &str) -> String {
    if started.len() > 5 {
        let bytes = started.as_bytes();
        let len = bytes.len();
        // Check if it ends with a 4-digit offset (no colon)
        if (bytes[len - 5] == b'+' || bytes[len - 5] == b'-')
            && bytes[len - 4].is_ascii_digit()
            && bytes[len - 3].is_ascii_digit()
            && bytes[len - 2].is_ascii_digit()
            && bytes[len - 1].is_ascii_digit()
        {
            // Insert colon: +0800 -> +08:00
            return format!("{}:{}", &started[..len - 2], &started[len - 2..]);
        }
    }
    started.to_string()
}
