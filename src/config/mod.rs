use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimeFormat {
    #[default]
    HoursMinutes,  // "3h 15m"
    Decimal,       // "3.25h"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ClockFormat {
    #[default]
    Hour24,      // "14:30"
    Hour12,      // "2:30pm"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ListViewMode {
    #[default]
    Contracted,  // Compact 2-line cards, description truncated
    Expanded,    // Cards grow to fit full wrapped description
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewMode {
    #[default]
    List,        // Traditional list of time entries
    Schedule,    // Multi-day schedule/timeline view
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub jira_domain: String,
    pub email: String,
    #[serde(default)]
    pub api_token: Option<String>,
    #[serde(default = "default_font_scale")]
    pub font_scale: f32,
    #[serde(default)]
    pub time_format: TimeFormat,
    #[serde(default)]
    pub clock_format: ClockFormat,
    #[serde(default = "default_true")]
    pub show_start_time: bool,
    #[serde(default = "default_tags")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub list_view_mode: ListViewMode,
    #[serde(default)]
    pub view_mode: ViewMode,
    #[serde(default = "default_schedule_start_hour")]
    pub schedule_start_hour: u8,
    #[serde(default = "default_schedule_end_hour")]
    pub schedule_end_hour: u8,
}

fn default_schedule_start_hour() -> u8 {
    5  // 5am
}

fn default_schedule_end_hour() -> u8 {
    20  // 8pm
}

fn default_true() -> bool {
    true
}

fn default_font_scale() -> f32 {
    1.0
}

fn default_tags() -> Vec<String> {
    vec![
        "FE".to_string(),
        "BE".to_string(),
        "Bugfix".to_string(),
        "CR".to_string(),
        "Support".to_string(),
        "Meetings".to_string(),
        "Refactor".to_string(),
        "Admin".to_string(),
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            jira_domain: String::new(),
            email: String::new(),
            api_token: None,
            font_scale: 1.0,
            time_format: TimeFormat::HoursMinutes,
            clock_format: ClockFormat::Hour24,
            show_start_time: true,
            tags: default_tags(),
            list_view_mode: ListViewMode::Contracted,
            view_mode: ViewMode::List,
            schedule_start_hour: 5,
            schedule_end_hour: 20,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;

        if config_path.exists() {
            let contents = fs::read_to_string(&config_path)
                .context("Failed to read config file")?;
            serde_json::from_str(&contents)
                .context("Failed to parse config file")
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;

        // Ensure directory exists
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&config_path, contents)?;

        Ok(())
    }

    pub fn is_configured(&self) -> bool {
        !self.jira_domain.is_empty()
            && !self.email.is_empty()
            && self.api_token.is_some()
    }

    fn config_path() -> Result<PathBuf> {
        let proj_dirs = ProjectDirs::from("com", "tzankich", "timebox")
            .context("Could not determine config directory")?;
        Ok(proj_dirs.config_dir().join("config.json"))
    }

    pub fn base_url(&self) -> String {
        // Clean up the domain - remove protocol, trailing slashes, paths
        let domain = self.jira_domain
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .split('/')
            .next()
            .unwrap_or(&self.jira_domain);

        format!("https://{}/rest/api/3", domain)
    }
}
