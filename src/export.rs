use chrono::{Datelike, Duration, Local};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

use crate::api::TimeEntry;
use crate::ui::WeekData;

#[derive(Serialize)]
pub struct WeeklyLog {
    pub week_start: String,
    pub week_end: String,
    pub exported_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    pub total_seconds: i64,
    pub entries: Vec<ExportEntry>,
}

#[derive(Serialize)]
pub struct ExportEntry {
    pub worklog_id: String,
    pub issue_key: String,
    pub issue_summary: String,
    pub issue_type: String,
    pub seconds: i64,
    pub description: String,
    pub date: String,
    pub start_time: String,
}

impl From<&TimeEntry> for ExportEntry {
    fn from(entry: &TimeEntry) -> Self {
        Self {
            worklog_id: entry.worklog_id.clone(),
            issue_key: entry.issue_key.clone(),
            issue_summary: entry.issue_summary.clone(),
            issue_type: entry.issue_type.clone(),
            seconds: entry.seconds,
            description: entry.description.clone(),
            date: entry.date.format("%Y-%m-%d").to_string(),
            start_time: entry.start_time.clone(),
        }
    }
}

/// Export the current week's data to a JSON file
/// Returns the path of the created file on success
/// If user_name is provided, includes it in the filename and JSON
pub fn export_week(week_data: &WeekData, user_name: Option<&str>) -> Result<PathBuf, String> {
    // Get exe directory
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get exe path: {}", e))?;
    let exe_dir = exe_path.parent()
        .ok_or("Failed to get exe directory")?;

    // Create weekly-logs directory
    let logs_dir = exe_dir.join("weekly-logs");
    fs::create_dir_all(&logs_dir)
        .map_err(|e| format!("Failed to create weekly-logs directory: {}", e))?;

    // Calculate ISO week number
    let week_start = week_data.week_start;
    let week_end = week_start + Duration::days(6);
    let iso_week = week_start.iso_week();

    // Build filename - include user name if provided
    let filename = if let Some(name) = user_name {
        // Sanitize name for filename (replace spaces with dashes, lowercase)
        let safe_name: String = name.chars()
            .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect();
        format!("{}-W{:02}-{}.json", iso_week.year(), iso_week.week(), safe_name)
    } else {
        format!("{}-W{:02}.json", iso_week.year(), iso_week.week())
    };
    let file_path = logs_dir.join(&filename);

    // Build the log structure
    let total_seconds: i64 = week_data.entries.iter().map(|e| e.seconds).sum();
    let log = WeeklyLog {
        week_start: week_start.format("%Y-%m-%d").to_string(),
        week_end: week_end.format("%Y-%m-%d").to_string(),
        exported_at: Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        user_name: user_name.map(String::from),
        total_seconds,
        entries: week_data.entries.iter().map(ExportEntry::from).collect(),
    };

    // Write JSON file
    let json = serde_json::to_string_pretty(&log)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    fs::write(&file_path, json)
        .map_err(|e| format!("Failed to write file: {}", e))?;

    Ok(file_path)
}
