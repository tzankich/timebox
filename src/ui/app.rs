use chrono::{Duration, Local, NaiveDate};
use eframe::egui;
use egui::{Color32, RichText};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use crate::api::{JiraClient, TimeEntry, Issue, parse_duration, format_duration_with_format, extract_time, parse_date};
use crate::config::{Config, TimeFormat, ClockFormat, ListViewMode, ViewMode};
use crate::export;
use crate::update::{self, UpdateInfo};
use super::views::{self, week_start, WeekData};

pub struct JiraTimeApp {
    config: Config,
    state: AppState,

    // Current view
    selected_date: NaiveDate,

    // Data - now using week-based caching
    week_data: WeekData,
    time_buckets: Vec<Issue>,

    // Weekly bucket tickets (Meeting, Support, Admin) - cached per week
    weekly_buckets: HashMap<String, (String, String, String)>,  // category -> (issue key, issue summary, issue type)
    weekly_buckets_week: Option<NaiveDate>,   // week start for which buckets are cached
    weekly_buckets_loading: bool,

    // Dialog for add/edit
    show_dialog: bool,
    dialog_mode: DialogMode,
    dialog_hours: String,
    dialog_issue: String,
    dialog_description: String,
    dialog_worklog_id: String,
    dialog_start_time: String,
    dialog_categories: Vec<bool>,  // Multi-select category tags

    // Form validation errors (true = has error)
    error_issue: bool,
    error_hours: bool,

    // Issue autocomplete
    issue_suggestions: Vec<Issue>,
    show_suggestions: bool,
    last_issue_search: String,
    last_search_time: Instant,
    searching_issues: bool,
    validated_issue: Option<(String, String, String)>,  // (issue key, issue summary, issue type)

    // Dialog accent color (for TIM tickets)
    dialog_accent_color: Option<Color32>,

    // Delete confirmation
    pending_delete: Option<TimeEntry>,
    show_delete_confirm: bool,

    // Settings dialog
    show_settings: bool,
    settings_domain: String,
    settings_email: String,
    settings_token: String,
    settings_font_scale: f32,
    settings_tags: String,
    settings_time_format: TimeFormat,
    settings_clock_format: ClockFormat,
    settings_show_start_time: bool,

    // Status
    status_message: Option<(String, bool)>, // (message, is_error)
    loading: bool,
    is_offline: bool,

    // Update state
    update_info: Option<UpdateInfo>,
    update_checking: bool,
    update_applying: bool,
    restart_pending: bool,

    // Progress bar state
    progress: f32,           // Current progress 0.0-1.0
    progress_start: std::time::Instant,
    progress_phase: ProgressPhase,

    // Async communication
    runtime: tokio::runtime::Runtime,
    result_rx: Receiver<AsyncResult>,
    result_tx: Sender<AsyncResult>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppState {
    Setup,
    Main,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DialogMode {
    Add,
    Edit,
}

enum AsyncResult {
    WorklogsLoaded(Vec<TimeEntry>, Vec<Issue>),
    WorklogSaved(String, TimeEntry, bool),  // (message, entry, is_edit)
    WorklogDeleted(String, String),  // (message, worklog_id)
    IssueSuggestions(Vec<Issue>),
    WeeklyBucketsLoaded(Vec<(String, String, String, String)>),  // (category, issue_key, issue_summary, issue_type)
    UpdateAvailable(UpdateInfo),
    UpdateApplied,
    UpdateError(String),
    Error(String),
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ProgressPhase {
    Idle,
    FastStart,    // 0→17% in 0.25s
    SlowCrawl,    // 17%→50% slowly
    Completing,   // snap to 100%
    FadingOut,    // fade out after completion
    Shrinking,    // shrink back on error
}

impl JiraTimeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config = Config::load().unwrap_or_default();
        super::setup_fonts(&cc.egui_ctx);
        super::setup_theme(&cc.egui_ctx);
        let state = if config.is_configured() {
            AppState::Main
        } else {
            AppState::Setup
        };

        let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let (result_tx, result_rx) = channel();

        let today = Local::now().date_naive();
        let current_week_start = week_start(today);
        let num_tags = config.tags.len();

        let mut app = Self {
            show_settings: false,
            settings_domain: config.jira_domain.trim_end_matches(".atlassian.net").to_string(),
            settings_email: config.email.clone(),
            settings_token: String::new(),
            settings_font_scale: config.font_scale,
            settings_tags: config.tags.join(", "),
            settings_time_format: config.time_format,
            settings_clock_format: config.clock_format,
            settings_show_start_time: config.show_start_time,
            config,
            state,
            selected_date: today,
            week_data: WeekData::new(current_week_start),
            time_buckets: Vec::new(),
            weekly_buckets: HashMap::new(),
            weekly_buckets_week: None,
            weekly_buckets_loading: false,
            show_dialog: false,
            dialog_mode: DialogMode::Add,
            dialog_hours: String::new(),
            dialog_issue: String::new(),
            dialog_description: String::new(),
            dialog_worklog_id: String::new(),
            dialog_start_time: String::new(),
            dialog_categories: vec![false; num_tags],
            error_issue: false,
            error_hours: false,
            issue_suggestions: Vec::new(),
            show_suggestions: false,
            last_issue_search: String::new(),
            last_search_time: Instant::now(),
            searching_issues: false,
            validated_issue: None,
            dialog_accent_color: None,
            pending_delete: None,
            show_delete_confirm: false,
            status_message: None,
            loading: false,
            is_offline: false,
            update_info: None,
            update_checking: false,
            update_applying: false,
            restart_pending: false,
            progress: 0.0,
            progress_start: std::time::Instant::now(),
            progress_phase: ProgressPhase::Idle,
            runtime,
            result_rx,
            result_tx,
        };

        if state == AppState::Main {
            // DEMO MODE: Use fake data for screenshots (comment out for normal use)
            //app.load_demo_data();
            app.refresh_data();
        }

        // Check for updates on startup
        app.check_for_updates();

        app
    }

    fn check_for_updates(&mut self) {
        if self.update_checking {
            return;
        }
        self.update_checking = true;

        let tx = self.result_tx.clone();
        self.runtime.spawn(async move {
            match update::check_for_update() {
                Ok(Some(info)) => {
                    let _ = tx.send(AsyncResult::UpdateAvailable(info));
                }
                Ok(None) => {
                    // No update available, no message needed
                }
                Err(e) => {
                    // Silently fail update checks - not critical
                    eprintln!("Update check failed: {}", e);
                }
            }
        });
    }

    fn apply_update(&mut self) {
        if self.update_applying {
            return;
        }
        self.update_applying = true;
        self.status_message = None;
        // Start progress animation
        self.progress = 0.0;
        self.progress_phase = ProgressPhase::FastStart;
        self.progress_start = std::time::Instant::now();

        let tx = self.result_tx.clone();
        self.runtime.spawn(async move {
            match update::apply_update() {
                Ok(()) => {
                    let _ = tx.send(AsyncResult::UpdateApplied);
                }
                Err(e) => {
                    let _ = tx.send(AsyncResult::UpdateError(format!("Update failed: {}", e)));
                }
            }
        });
    }

    /// Load fake demo data for taking screenshots without personal info
    #[allow(dead_code)]
    fn load_demo_data(&mut self) {
        use crate::api::TimeEntry;

        let today = self.selected_date;
        let descriptions = [
            "Lorem ipsum dolor sit amet",
            "Consectetur adipiscing elit",
            "Sed do eiusmod tempor incididunt",
            "Ut enim ad minim veniam",
            "Quis nostrud exercitation ullamco",
            "Duis aute irure dolor in reprehenderit",
            "Velit esse cillum dolore eu fugiat",
            "Excepteur sint occaecat cupidatat",
        ];

        self.week_data.entries = vec![
            TimeEntry {
                worklog_id: "1".to_string(),
                issue_key: "TIM-42".to_string(),
                issue_summary: "MEETINGS - 2024-12-02 to 2024-12-06".to_string(),
                issue_type: "Task".to_string(),
                seconds: 3600,
                description: descriptions[0].to_string(),
                date: today,
                start_time: "09:00".to_string(),
            },
            TimeEntry {
                worklog_id: "2".to_string(),
                issue_key: "PROJ-123".to_string(),
                issue_summary: "Implement user authentication flow".to_string(),
                issue_type: "Story".to_string(),
                seconds: 5400,
                description: descriptions[1].to_string(),
                date: today,
                start_time: "10:00".to_string(),
            },
            TimeEntry {
                worklog_id: "3".to_string(),
                issue_key: "TIM-43".to_string(),
                issue_summary: "SUPPORT - 2024-12-02 to 2024-12-06".to_string(),
                issue_type: "Task".to_string(),
                seconds: 1800,
                description: descriptions[2].to_string(),
                date: today,
                start_time: "11:30".to_string(),
            },
            TimeEntry {
                worklog_id: "4".to_string(),
                issue_key: "PROJ-456".to_string(),
                issue_summary: "Fix database connection pooling issue".to_string(),
                issue_type: "Bug".to_string(),
                seconds: 7200,
                description: descriptions[3].to_string(),
                date: today,
                start_time: "13:00".to_string(),
            },
            TimeEntry {
                worklog_id: "5".to_string(),
                issue_key: "TIM-44".to_string(),
                issue_summary: "ADMIN - 2024-12-02 to 2024-12-06".to_string(),
                issue_type: "Task".to_string(),
                seconds: 2700,
                description: descriptions[4].to_string(),
                date: today,
                start_time: "15:00".to_string(),
            },
            TimeEntry {
                worklog_id: "6".to_string(),
                issue_key: "PROJ-789".to_string(),
                issue_summary: "Code review and documentation updates".to_string(),
                issue_type: "Task".to_string(),
                seconds: 3600,
                description: descriptions[5].to_string(),
                date: today,
                start_time: "16:00".to_string(),
            },
        ];

        // Add some entries for other days
        let yesterday = today - chrono::Duration::days(1);
        self.week_data.entries.push(TimeEntry {
            worklog_id: "7".to_string(),
            issue_key: "PROJ-101".to_string(),
            issue_summary: "Sprint planning and backlog refinement".to_string(),
            issue_type: "Epic".to_string(),
            seconds: 10800,
            description: descriptions[6].to_string(),
            date: yesterday,
            start_time: "09:00".to_string(),
        });
        self.week_data.entries.push(TimeEntry {
            worklog_id: "8".to_string(),
            issue_key: "TIM-42".to_string(),
            issue_summary: "MEETINGS - 2024-12-02 to 2024-12-06".to_string(),
            issue_type: "Task".to_string(),
            seconds: 5400,
            description: descriptions[7].to_string(),
            date: yesterday,
            start_time: "14:00".to_string(),
        });

        // Fake weekly buckets (key, summary, issue_type)
        self.weekly_buckets.insert("MEETING".to_string(), ("TIM-42".to_string(), "MEETINGS - 2024-12-02 to 2024-12-06".to_string(), "Task".to_string()));
        self.weekly_buckets.insert("SUPPORT".to_string(), ("TIM-43".to_string(), "SUPPORT - 2024-12-02 to 2024-12-06".to_string(), "Task".to_string()));
        self.weekly_buckets.insert("ADMIN".to_string(), ("TIM-44".to_string(), "ADMIN - 2024-12-02 to 2024-12-06".to_string(), "Task".to_string()));
        self.weekly_buckets_week = Some(self.week_data.week_start);
    }

    fn check_async_results(&mut self) {
        while let Ok(result) = self.result_rx.try_recv() {
            match result {
                AsyncResult::WorklogsLoaded(entries, buckets) => {
                    self.week_data.entries = entries;
                    self.time_buckets = buckets;
                    self.loading = false;
                    self.is_offline = false;
                    self.status_message = None;
                    // Trigger completion animation
                    self.progress_phase = ProgressPhase::Completing;
                    self.progress_start = std::time::Instant::now();
                }
                AsyncResult::WorklogSaved(_msg, entry, is_edit) => {
                    self.loading = false;
                    self.show_dialog = false;
                    // Trigger completion animation
                    self.progress_phase = ProgressPhase::Completing;
                    self.progress_start = std::time::Instant::now();
                    // Update local data instead of full refresh
                    if is_edit {
                        // Update existing entry
                        if let Some(existing) = self.week_data.entries.iter_mut()
                            .find(|e| e.worklog_id == entry.worklog_id)
                        {
                            existing.seconds = entry.seconds;
                            existing.description = entry.description;
                            existing.start_time = entry.start_time;
                        }
                        // Re-sort since start time may have changed
                        self.week_data.entries.sort_by(|a, b| {
                            a.date.cmp(&b.date).then_with(|| a.start_time.cmp(&b.start_time))
                        });
                    } else {
                        // Add new entry and sort by start time
                        self.week_data.entries.push(entry);
                        self.week_data.entries.sort_by(|a, b| {
                            a.date.cmp(&b.date).then_with(|| a.start_time.cmp(&b.start_time))
                        });
                    }
                }
                AsyncResult::WorklogDeleted(_msg, worklog_id) => {
                    self.loading = false;
                    // Trigger completion animation
                    self.progress_phase = ProgressPhase::Completing;
                    self.progress_start = std::time::Instant::now();
                    // Remove entry from local data
                    self.week_data.entries.retain(|e| e.worklog_id != worklog_id);
                }
                AsyncResult::IssueSuggestions(issues) => {
                    self.issue_suggestions = issues;
                    self.searching_issues = false;
                    self.show_suggestions = !self.issue_suggestions.is_empty();
                }
                AsyncResult::WeeklyBucketsLoaded(buckets) => {
                    self.weekly_buckets.clear();
                    for (category, key, summary, issue_type) in buckets {
                        self.weekly_buckets.insert(category, (key, summary, issue_type));
                    }
                    self.weekly_buckets_week = Some(self.week_data.week_start);
                    self.weekly_buckets_loading = false;
                }
                AsyncResult::Error(msg) => {
                    self.loading = false;
                    self.searching_issues = false;
                    self.is_offline = false;
                    self.status_message = Some((msg, true));
                    // Trigger shrink animation
                    self.progress_phase = ProgressPhase::Shrinking;
                    self.progress_start = std::time::Instant::now();
                }
                AsyncResult::Offline => {
                    self.loading = false;
                    self.searching_issues = false;
                    self.is_offline = true;
                    self.status_message = None;
                    // Trigger shrink animation
                    self.progress_phase = ProgressPhase::Shrinking;
                    self.progress_start = std::time::Instant::now();
                }
                AsyncResult::UpdateAvailable(info) => {
                    self.update_checking = false;
                    self.update_info = Some(info);
                }
                AsyncResult::UpdateApplied => {
                    self.update_applying = false;
                    #[cfg(target_os = "macos")]
                    {
                        // macOS quarantines downloaded binaries - user must authorize manually
                        self.status_message = Some(("Updated! Run: xattr -d com.apple.quarantine <app-path> then restart".to_string(), false));
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        // Spawn new process and request graceful shutdown
                        if let Ok(exe) = std::env::current_exe() {
                            let _ = std::process::Command::new(exe).spawn();
                            self.restart_pending = true;
                        }
                    }
                }
                AsyncResult::UpdateError(msg) => {
                    self.update_applying = false;
                    self.status_message = Some((msg, true));
                }
            }
        }
    }

    fn refresh_data(&mut self) {
        self.load_week(self.week_data.week_start);
    }

    fn load_week(&mut self, week_start_date: NaiveDate) {
        if !self.config.is_configured() {
            return;
        }

        // If already loading, don't start another request
        if self.loading {
            // But still update UI state immediately
            self.week_data.week_start = week_start_date;
            return;
        }

        // Clear entries immediately for snappy UI
        self.week_data = WeekData::new(week_start_date);

        self.loading = true;
        self.progress = 0.0;
        self.progress_phase = ProgressPhase::FastStart;
        self.progress_start = std::time::Instant::now();

        let config = self.config.clone();
        let tx = self.result_tx.clone();

        // Always load the full week (Mon-Sun)
        let start_date = week_start_date;
        let end_date = week_start_date + Duration::days(6);

        // Also load weekly buckets for quick-add buttons
        self.load_weekly_buckets(week_start_date);

        // Spawn async task
        self.runtime.spawn(async move {
            let result = async {
                let client = JiraClient::new(&config)?;
                let worklogs = client.get_my_worklogs(start_date, end_date).await?;
                let buckets = client.get_time_buckets().await.unwrap_or_default();
                Ok::<_, anyhow::Error>((worklogs, buckets))
            }.await;

            match result {
                Ok((worklogs, buckets)) => {
                    let entries: Vec<TimeEntry> = worklogs
                        .into_iter()
                        .map(|(issue_key, issue_summary, issue_type, worklog)| {
                            let description = worklog.comment_text();
                            let seconds = worklog.time_spent_seconds;
                            let date = parse_date(&worklog.started);
                            let start_time = extract_time(&worklog.started);
                            TimeEntry {
                                worklog_id: worklog.id,
                                issue_key,
                                issue_summary,
                                issue_type,
                                seconds,
                                description,
                                date,
                                start_time,
                            }
                        })
                        .collect();
                    let _ = tx.send(AsyncResult::WorklogsLoaded(entries, buckets));
                }
                Err(e) => {
                    // Check if this is a network connectivity error
                    let err_str = e.to_string().to_lowercase();
                    if err_str.contains("connection") || err_str.contains("network")
                       || err_str.contains("dns") || err_str.contains("resolve")
                       || err_str.contains("timeout") || err_str.contains("unreachable")
                       || err_str.contains("error sending request") || err_str.contains("no route")
                       || err_str.contains("failed to lookup") {
                        let _ = tx.send(AsyncResult::Offline);
                    } else {
                        let _ = tx.send(AsyncResult::Error(format!("Error: {}", e)));
                    }
                }
            }
        });
    }

    fn save_settings(&mut self) {
        // Build full domain from subdomain input
        let full_domain = if self.settings_domain.contains('.') {
            self.settings_domain.clone()
        } else {
            format!("{}.atlassian.net", self.settings_domain)
        };

        // Check if credentials changed (need to reload if so)
        let credentials_changed =
            self.config.jira_domain != full_domain
            || self.config.email != self.settings_email
            || !self.settings_token.is_empty();

        self.config.jira_domain = full_domain;
        self.config.email = self.settings_email.clone();
        self.config.font_scale = self.settings_font_scale;
        self.config.time_format = self.settings_time_format;
        self.config.clock_format = self.settings_clock_format;
        self.config.show_start_time = self.settings_show_start_time;
        // Parse tags from comma-separated string
        self.config.tags = self.settings_tags
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        // Update dialog_categories to match new tag count
        self.dialog_categories = vec![false; self.config.tags.len()];

        if !self.settings_token.is_empty() {
            self.config.api_token = Some(self.settings_token.clone());
        }

        match self.config.save() {
            Ok(_) => {
                self.show_settings = false;
                // Transition from Setup to Main if now configured
                if self.config.is_configured() && self.state == AppState::Setup {
                    self.state = AppState::Main;
                }
                if credentials_changed {
                    self.refresh_data();
                }
            }
            Err(e) => {
                self.status_message = Some((format!("Failed to save: {}", e), true));
            }
        }
    }

    fn load_weekly_buckets(&mut self, week_start_date: NaiveDate) {
        // Skip if already loading or already have buckets for this week
        if self.weekly_buckets_loading {
            return;
        }
        if self.weekly_buckets_week == Some(week_start_date) {
            return;
        }

        self.weekly_buckets_loading = true;

        let config = self.config.clone();
        let tx = self.result_tx.clone();

        self.runtime.spawn(async move {
            let result = async {
                let client = JiraClient::new(&config)?;
                client.search_all_weekly_buckets(week_start_date).await
            }.await;

            match result {
                Ok(buckets) => {
                    let bucket_data: Vec<(String, String, String, String)> = buckets
                        .into_iter()
                        .map(|(cat, issue)| {
                            let issue_type = issue.fields.issue_type
                                .map(|t| t.name)
                                .unwrap_or_else(|| "Task".to_string());
                            (cat, issue.key, issue.fields.summary, issue_type)
                        })
                        .collect();
                    let _ = tx.send(AsyncResult::WeeklyBucketsLoaded(bucket_data));
                }
                Err(_) => {
                    // Silently fail - buckets are optional
                    let _ = tx.send(AsyncResult::WeeklyBucketsLoaded(Vec::new()));
                }
            }
        });
    }

    fn search_issues(&mut self, query: &str) {
        if self.searching_issues {
            return;
        }

        self.searching_issues = true;
        self.last_issue_search = query.to_string();

        let config = self.config.clone();
        let tx = self.result_tx.clone();
        let query = query.to_string();

        self.runtime.spawn(async move {
            let result = async {
                let client = JiraClient::new(&config)?;
                if query.is_empty() {
                    client.get_recent_issues().await
                } else {
                    client.search_issues_by_text(&query).await
                }
            }.await;

            match result {
                Ok(issues) => {
                    let _ = tx.send(AsyncResult::IssueSuggestions(issues));
                }
                Err(_) => {
                    // Silently fail for autocomplete
                    let _ = tx.send(AsyncResult::IssueSuggestions(Vec::new()));
                }
            }
        });
    }

    fn open_add_dialog(&mut self) {
        self.dialog_mode = DialogMode::Add;
        self.dialog_hours = String::new();
        self.dialog_issue = String::new();
        self.dialog_description = String::new();
        self.dialog_worklog_id = String::new();
        self.dialog_start_time = String::new();
        self.dialog_categories = vec![false; self.config.tags.len()];
        self.dialog_accent_color = None;
        self.error_issue = false;
        self.error_hours = false;
        self.issue_suggestions = Vec::new();
        self.show_suggestions = false;
        self.last_issue_search = String::new();
        self.validated_issue = None;
        self.show_dialog = true;
        // Load recent issues immediately
        self.search_issues("");
    }

    fn open_edit_dialog(&mut self, entry: &TimeEntry) {
        self.dialog_mode = DialogMode::Edit;
        self.dialog_hours = format_duration_with_format(entry.seconds, self.config.time_format);
        self.dialog_issue = entry.issue_key.clone();

        // Parse categories from description and extract remaining text
        let (categories, desc) = Self::parse_categories_from_description(&entry.description, &self.config.tags);
        self.dialog_categories = categories;
        self.dialog_description = desc;

        self.dialog_worklog_id = entry.worklog_id.clone();
        self.dialog_start_time = entry.start_time.clone();  // Pre-fill with current start time
        // Set accent color based on ticket type (same logic as entry cards)
        self.dialog_accent_color = if entry.issue_key.starts_with("TIM-") {
            let summary_upper = entry.issue_summary.to_uppercase();
            if summary_upper.contains("MEETING") {
                Some(Color32::from_rgb(0xdc, 0x26, 0x7f))  // Pink
            } else if summary_upper.contains("SUPPORT") {
                Some(Color32::from_rgb(0xfe, 0x61, 0x00))  // Orange
            } else if summary_upper.contains("ADMIN") {
                Some(Color32::from_rgb(0xff, 0xb0, 0x00))  // Yellow
            } else {
                None  // Default blue will be used
            }
        } else {
            None  // Default blue for regular tickets
        };
        self.error_issue = false;
        self.error_hours = false;
        self.issue_suggestions = Vec::new();
        self.show_suggestions = false;
        self.validated_issue = Some((entry.issue_key.clone(), entry.issue_summary.clone(), entry.issue_type.clone()));
        self.show_dialog = true;
    }

    /// Parse category tags like [FE][BE] from the start of a description
    fn parse_categories_from_description(description: &str, tags: &[String]) -> (Vec<bool>, String) {
        let mut categories = vec![false; tags.len()];
        let mut remaining = description.trim();

        // Parse all tags at the start of the description
        loop {
            let trimmed = remaining.trim_start();
            if !trimmed.starts_with('[') {
                remaining = trimmed;
                break;
            }

            if let Some(end) = trimmed.find(']') {
                let tag = &trimmed[1..end];
                // Check if this matches one of our tags (case-insensitive)
                let mut found = false;
                for (i, cat) in tags.iter().enumerate() {
                    if tag.eq_ignore_ascii_case(cat) {
                        categories[i] = true;
                        found = true;
                        break;
                    }
                }
                if found {
                    remaining = &trimmed[end + 1..];
                    // Skip any separator after the tag (space, dash, etc.)
                    remaining = remaining.trim_start_matches(|c: char| c == ' ' || c == '-');
                } else {
                    // Unknown tag - stop parsing
                    break;
                }
            } else {
                break;
            }
        }

        (categories, remaining.to_string())
    }

    fn delete_worklog(&mut self, entry: &TimeEntry) {
        self.loading = true;
        self.progress = 0.0;
        self.progress_phase = ProgressPhase::FastStart;
        self.progress_start = std::time::Instant::now();

        let config = self.config.clone();
        let issue_key = entry.issue_key.clone();
        let worklog_id = entry.worklog_id.clone();
        let tx = self.result_tx.clone();

        self.runtime.spawn(async move {
            let result: Result<String, anyhow::Error> = async {
                let client = JiraClient::new(&config)?;
                client.delete_worklog(&issue_key, &worklog_id).await?;
                Ok(format!("Deleted worklog from {}", issue_key))
            }.await;

            match result {
                Ok(msg) => {
                    let _ = tx.send(AsyncResult::WorklogDeleted(msg, worklog_id));
                }
                Err(e) => {
                    let err_str = e.to_string().to_lowercase();
                    if err_str.contains("connection") || err_str.contains("network")
                       || err_str.contains("error sending request") || err_str.contains("timeout") {
                        let _ = tx.send(AsyncResult::Offline);
                    } else {
                        let _ = tx.send(AsyncResult::Error(format!("Failed to delete: {}", e)));
                    }
                }
            }
        });
    }

    fn save_dialog(&mut self) {
        // Clear previous errors
        self.error_issue = false;
        self.error_hours = false;

        // Validate issue first (it's the first field)
        let is_validated = self.validated_issue.as_ref()
            .map(|(key, _, _)| key == &self.dialog_issue)
            .unwrap_or(false);
        if self.dialog_issue.is_empty() || !is_validated {
            self.error_issue = true;
        }

        // Validate duration
        let seconds = match parse_duration(&self.dialog_hours) {
            Some(s) => s,
            None => {
                self.error_hours = true;
                0
            }
        };

        // If any errors, don't proceed
        if self.error_issue || self.error_hours {
            return;
        }

        self.loading = true;
        self.progress = 0.0;
        self.progress_phase = ProgressPhase::FastStart;
        self.progress_start = std::time::Instant::now();

        // Build category prefix from selected tags
        let mut category_prefix = String::new();
        for (i, selected) in self.dialog_categories.iter().enumerate() {
            if *selected {
                if let Some(tag) = self.config.tags.get(i) {
                    category_prefix.push_str(&format!("[{}]", tag));
                }
            }
        }

        // Combine categories with description
        let description = if category_prefix.is_empty() {
            self.dialog_description.clone()
        } else if self.dialog_description.trim().is_empty() {
            category_prefix
        } else {
            format!("{} {}", category_prefix, self.dialog_description.trim())
        };
        let user_start_time = if self.dialog_start_time.trim().is_empty() {
            None
        } else {
            Some(self.dialog_start_time.clone())
        };

        let config = self.config.clone();
        let issue_key = self.dialog_issue.clone();
        let (issue_summary, issue_type) = self.validated_issue.as_ref()
            .map(|(_, s, t)| (s.clone(), t.clone()))
            .unwrap_or_default();
        let worklog_id = self.dialog_worklog_id.clone();
        let date = self.selected_date;
        let tx = self.result_tx.clone();
        let is_edit = self.dialog_mode == DialogMode::Edit;
        let duration_str = format_duration_with_format(seconds, self.config.time_format);
        let description_clone = description.clone();
        self.runtime.spawn(async move {
            let result: Result<(String, TimeEntry), anyhow::Error> = async {
                let client = JiraClient::new(&config)?;
                if is_edit {
                    let worklog = client.update_worklog(&issue_key, &worklog_id, seconds, &description_clone, date, user_start_time.as_deref()).await?;
                    // Use the actual start time from Jira's response
                    let start_time = extract_time(&worklog.started);
                    let entry = TimeEntry {
                        worklog_id: worklog.id,
                        issue_key: issue_key.clone(),
                        issue_summary: issue_summary.clone(),
                        issue_type: issue_type.clone(),
                        seconds,
                        description: description_clone,
                        date,
                        start_time,
                    };
                    Ok((format!("Updated {} on {}", duration_str, issue_key), entry))
                } else {
                    let worklog = client.log_time(&issue_key, seconds, date, &description_clone, user_start_time.as_deref()).await?;
                    // Use the actual start time from Jira's response
                    let start_time = extract_time(&worklog.started);
                    let entry = TimeEntry {
                        worklog_id: worklog.id,
                        issue_key: issue_key.clone(),
                        issue_summary: issue_summary.clone(),
                        issue_type: issue_type.clone(),
                        seconds,
                        description: description_clone,
                        date,
                        start_time,
                    };
                    Ok((format!("Logged {} to {}", duration_str, issue_key), entry))
                }
            }.await;

            match result {
                Ok((msg, entry)) => {
                    let _ = tx.send(AsyncResult::WorklogSaved(msg, entry, is_edit));
                }
                Err(e) => {
                    let err_str = e.to_string().to_lowercase();
                    if err_str.contains("connection") || err_str.contains("network")
                       || err_str.contains("error sending request") || err_str.contains("timeout") {
                        let _ = tx.send(AsyncResult::Offline);
                    } else {
                        let _ = tx.send(AsyncResult::Error(format!("Failed: {}", e)));
                    }
                }
            }
        });
    }

    fn render_setup(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading("Timebox setup");
            ui.add_space(20.0);

            ui.label("Enter your Jira credentials to get started.");
            ui.add_space(8.0);

            let link = egui::Label::new(
                RichText::new("Create an API token at Atlassian")
                    .color(egui::Color32::from_rgb(0x13, 0x98, 0xf4))
            ).sense(egui::Sense::click());
            let response = ui.add(link);
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                let _ = open::that("https://id.atlassian.com/manage-profile/security/api-tokens");
            }

            ui.add_space(20.0);
        });

        egui::Grid::new("setup_grid")
            .num_columns(2)
            .spacing([20.0, 10.0])
            .show(ui, |ui| {
                ui.label("Jira Domain:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings_domain)
                            .hint_text("company")
                            .desired_width(200.0)
                    );
                    ui.label(".atlassian.net");
                });
                ui.end_row();

                ui.label("Email:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_email)
                        .hint_text("you@company.com")
                        .desired_width(350.0)
                );
                ui.end_row();

                ui.label("API Token:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_token)
                        .password(true)
                        .hint_text("Paste your API token here")
                        .desired_width(350.0)
                );
                ui.end_row();
            });

        ui.add_space(20.0);

        if ui.button("Save and connect").clicked() {
            self.save_settings();
        }
    }

    fn render_main(&mut self, ui: &mut egui::Ui) {
        // Header with week navigation
        ui.horizontal(|ui| {
            // Week navigation styled like a button but pill-shaped
            let (button_bg, button_text) = super::theme::button_colors();

            egui::Frame::none()
                .fill(button_bg)
                .rounding(egui::Rounding::same(12.0))  // Pill-shaped (fully rounded)
                .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Left arrow
                        let left_arrow = ui.add(egui::Label::new(
                            RichText::new(egui_phosphor::regular::CARET_LEFT).size(14.0).color(button_text)
                        ).sense(egui::Sense::click()));
                        if left_arrow.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if left_arrow.clicked() {
                            let new_week = self.week_data.week_start - Duration::days(7);
                            self.selected_date = new_week;
                            self.load_week(new_week);
                        }

                        ui.add_space(4.0);

                        // Date text with min width to prevent resizing
                        let start = self.week_data.week_start;
                        let end = start + Duration::days(4);
                        let date_text = format!("{} - {}", start.format("%b %-d"), end.format("%b %-d, %Y"));
                        // Min width for longest possible text like "Sep 29 - Oct 3, 2025"
                        ui.allocate_ui_with_layout(
                            egui::vec2(115.0, 14.0),
                            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                            |ui| {
                                ui.label(RichText::new(&date_text).size(14.0).color(button_text));
                            }
                        );

                        ui.add_space(4.0);

                        // Right arrow
                        let right_arrow = ui.add(egui::Label::new(
                            RichText::new(egui_phosphor::regular::CARET_RIGHT).size(14.0).color(button_text)
                        ).sense(egui::Sense::click()));
                        if right_arrow.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if right_arrow.clicked() {
                            let new_week = self.week_data.week_start + Duration::days(7);
                            self.selected_date = new_week;
                            self.load_week(new_week);
                        }
                    });
                });

            // Weekly total (only show if > 0) - white bold for times to stand out
            ui.add_space(16.0);
            let week_total: i64 = self.week_data.entries.iter().map(|e| e.seconds).sum();
            if week_total > 0 {
                let week_total_str = crate::api::format_duration_with_format(week_total, self.config.time_format);
                ui.label(RichText::new(week_total_str).size(14.0).color(Color32::WHITE).family(crate::ui::theme::bold_family()));
            }

            // View mode dropdown (icon + chevron)
            ui.add_space(16.0);
            let view_menu_id = ui.make_persistent_id("view_mode_menu");
            let (current_icon, other_icon, other_label, other_mode) = match self.config.view_mode {
                ViewMode::List => (
                    egui_phosphor::regular::LIST,
                    egui_phosphor::regular::SQUARES_FOUR,
                    "Schedule view",
                    ViewMode::Schedule,
                ),
                ViewMode::Schedule => (
                    egui_phosphor::regular::SQUARES_FOUR,
                    egui_phosphor::regular::LIST,
                    "List view",
                    ViewMode::List,
                ),
            };

            let icon_color = Color32::from_rgb(160, 160, 152);
            let hover_color = Color32::WHITE;
            let btn_text = format!("{} {}", current_icon, egui_phosphor::regular::CARET_DOWN);
            let font_id = egui::FontId::proportional(14.0);  // Match snap dropdown size
            let text_size = ui.fonts(|f| f.layout_no_wrap(btn_text.clone(), font_id.clone(), icon_color).size());
            let (btn_rect, btn_response) = ui.allocate_exact_size(text_size + egui::vec2(6.0, 4.0), egui::Sense::click());
            let btn_col = if btn_response.hovered() { hover_color } else { icon_color };
            ui.painter().text(btn_rect.center(), egui::Align2::CENTER_CENTER, &btn_text, font_id.clone(), btn_col);

            if btn_response.clicked() {
                ui.memory_mut(|mem| mem.toggle_popup(view_menu_id));
            }

            egui::popup::popup_below_widget(ui, view_menu_id, &btn_response, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(140.0);
                ui.style_mut().spacing.button_padding = egui::vec2(12.0, 8.0);

                let menu_text = format!("{} {}", other_icon, other_label);
                if ui.add(egui::Button::new(
                    RichText::new(menu_text).size(14.0)
                ).frame(false)).clicked() {
                    self.config.view_mode = other_mode;
                    let _ = self.config.save();
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Icon-only buttons for Settings and Reload - gray, white on hover
                let text_color = Color32::from_rgb(150, 150, 150);
                let hover_color = Color32::WHITE;
                let font_id = egui::FontId::proportional(18.0);

                // Update available indicator (green, clickable)
                if let Some(update_info) = &self.update_info {
                    let update_color = Color32::from_rgb(152, 195, 121);  // Green
                    let update_text = format!("{} v{}", egui_phosphor::regular::ARROW_CIRCLE_UP, update_info.latest_version);
                    let update_font = egui::FontId::proportional(14.0);
                    let text_size = ui.fonts(|f| f.layout_no_wrap(update_text.clone(), update_font.clone(), update_color).size());
                    let (update_rect, update_response) = ui.allocate_exact_size(text_size + egui::vec2(8.0, 4.0), egui::Sense::click());
                    let update_col = if update_response.hovered() { Color32::WHITE } else { update_color };
                    ui.painter().text(update_rect.center(), egui::Align2::CENTER_CENTER, &update_text, update_font, update_col);
                    if update_response.on_hover_text("Click to update").clicked() && !self.update_applying {
                        self.apply_update();
                    }
                    ui.add_space(12.0);
                }

                // Settings button
                let settings_icon = egui_phosphor::regular::FADERS_HORIZONTAL;
                let icon_size = ui.fonts(|f| f.layout_no_wrap(settings_icon.to_string(), font_id.clone(), Color32::WHITE).size());
                let (settings_rect, settings_response) = ui.allocate_exact_size(icon_size + egui::vec2(8.0, 4.0), egui::Sense::click());
                let settings_col = if settings_response.hovered() { hover_color } else { text_color };
                ui.painter().text(settings_rect.center(), egui::Align2::CENTER_CENTER, settings_icon, font_id.clone(), settings_col);
                if settings_response.on_hover_text("Settings").clicked() {
                    // Reset settings to current config values
                    self.settings_domain = self.config.jira_domain.trim_end_matches(".atlassian.net").to_string();
                    self.settings_email = self.config.email.clone();
                    self.settings_token = String::new();
                    self.settings_font_scale = self.config.font_scale;
                    self.settings_tags = self.config.tags.join(", ");
                    self.settings_time_format = self.config.time_format;
                    self.settings_clock_format = self.config.clock_format;
                    self.settings_show_start_time = self.config.show_start_time;
                    self.show_settings = true;
                }

                ui.add_space(12.0);

                // Reload button
                let reload_icon = egui_phosphor::regular::CLOUD_ARROW_DOWN;
                let icon_size = ui.fonts(|f| f.layout_no_wrap(reload_icon.to_string(), font_id.clone(), Color32::WHITE).size());
                let (reload_rect, reload_response) = ui.allocate_exact_size(icon_size + egui::vec2(8.0, 4.0), egui::Sense::click());
                let reload_col = if reload_response.hovered() { hover_color } else { text_color };
                ui.painter().text(reload_rect.center(), egui::Align2::CENTER_CENTER, reload_icon, font_id.clone(), reload_col);
                if reload_response.on_hover_text("Sync with Jira").clicked() {
                    self.refresh_data();
                }

                ui.add_space(12.0);

                // Export button (JSON icon)
                let export_icon = egui_phosphor::regular::BRACKETS_CURLY;
                let icon_size = ui.fonts(|f| f.layout_no_wrap(export_icon.to_string(), font_id.clone(), Color32::WHITE).size());
                let (export_rect, export_response) = ui.allocate_exact_size(icon_size + egui::vec2(8.0, 4.0), egui::Sense::click());
                let export_col = if export_response.hovered() { hover_color } else { text_color };
                ui.painter().text(export_rect.center(), egui::Align2::CENTER_CENTER, export_icon, font_id, export_col);
                if export_response.on_hover_text("Export week to JSON").clicked() {
                    match export::export_week(&self.week_data, None) {
                        Ok(path) => {
                            self.status_message = Some((format!("Exported to {}", path.display()), false));
                        }
                        Err(e) => {
                            self.status_message = Some((format!("Export failed: {}", e), true));
                        }
                    }
                }
            });
        });

        ui.add_space(8.0);

        // Show offline message if we're offline
        if self.is_offline {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(format!("{}", egui_phosphor::regular::WIFI_SLASH))
                        .size(34.0)
                        .color(Color32::from_rgb(224, 108, 117))
                );
                ui.add_space(16.0);
                ui.label(
                    RichText::new("No connection")
                        .size(20.0)
                        .color(Color32::from_rgb(200, 200, 210))
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Check your internet and try again")
                        .size(14.0)
                        .color(Color32::from_rgb(120, 120, 140))
                );
                ui.add_space(24.0);
                let blue = Color32::from_rgb(0x13, 0x98, 0xf4);
                if ui.add(
                    egui::Button::new(
                        RichText::new(format!("{} Retry", egui_phosphor::regular::ARROWS_CLOCKWISE))
                            .size(17.0)
                            .color(Color32::WHITE)
                    )
                    .fill(blue)
                    .rounding(6.0)
                ).clicked() {
                    self.is_offline = false;
                    self.refresh_data();
                }
            });
            return;
        }

        // Render view based on view_mode
        match self.config.view_mode {
            ViewMode::List => {
                // Day tabs with view mode toggle (only in List mode)
                let (clicked_day, view_toggled) = views::render_day_tabs(
                    ui,
                    &self.week_data,
                    self.selected_date,
                    self.config.time_format,
                    self.config.list_view_mode,
                );
                if let Some(day) = clicked_day {
                    self.selected_date = day;
                }
                if view_toggled {
                    self.config.list_view_mode = match self.config.list_view_mode {
                        ListViewMode::Contracted => ListViewMode::Expanded,
                        ListViewMode::Expanded => ListViewMode::Contracted,
                    };
                    let _ = self.config.save();
                }

                ui.add_space(8.0);

                // Entry list for selected day (sorted by start time)
                let mut day_entries: Vec<TimeEntry> = self.week_data.entries_for_day(self.selected_date)
                    .into_iter()
                    .cloned()
                    .collect();
                day_entries.sort_by(|a, b| a.start_time.cmp(&b.start_time));
                let base_url = format!("https://{}", self.config.jira_domain);
                let (edit_idx, delete_idx, add_clicked) = views::render_entry_list(ui, &day_entries, &base_url, self.config.time_format, self.config.clock_format, self.config.show_start_time, self.config.list_view_mode);
                if let Some(idx) = edit_idx {
                    let entry = day_entries[idx].clone();
                    self.open_edit_dialog(&entry);
                }
                if let Some(idx) = delete_idx {
                    let entry = day_entries[idx].clone();
                    self.pending_delete = Some(entry);
                    self.show_delete_confirm = true;
                }
                if add_clicked {
                    self.open_add_dialog();
                }
            }
            ViewMode::Schedule => {
                // Schedule view - render timeline grid
                let base_url = format!("https://{}", self.config.jira_domain);
                let schedule_result = views::render_schedule_view(
                    ui,
                    &self.week_data,
                    &base_url,
                    self.config.time_format,
                    self.config.clock_format,
                    self.config.schedule_start_hour,
                    self.config.schedule_end_hour,
                );
                if let Some(entry) = schedule_result.edit_entry {
                    self.open_edit_dialog(&entry);
                }
                if let Some(entry) = schedule_result.delete_entry {
                    self.pending_delete = Some(entry);
                    self.show_delete_confirm = true;
                }
                if let Some((date, start_time)) = schedule_result.add_at {
                    self.selected_date = date;
                    self.open_add_dialog();
                    self.dialog_start_time = start_time;
                }
            }
        }
    }

    fn render_settings_with_colors(&mut self, ui: &mut egui::Ui, _frame_color: Color32, _frame_text: Color32) {
        let accent = Color32::from_rgb(19, 152, 244);
        let section_color = Color32::from_rgb(140, 140, 160);

        // === Jira Connection ===
        ui.label(RichText::new("Jira Connection").color(section_color).strong());
        ui.add_space(8.0);

        egui::Grid::new("jira_grid")
            .num_columns(2)
            .spacing([20.0, 10.0])
            .show(ui, |ui| {
                ui.label("Domain");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.settings_domain)
                        .hint_text("company")
                        .desired_width(200.0));
                    ui.label(".atlassian.net");
                });
                ui.end_row();

                ui.label("Email");
                ui.add(egui::TextEdit::singleline(&mut self.settings_email)
                    .desired_width(350.0));
                ui.end_row();

                ui.label("API token");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_token)
                        .password(true)
                        .hint_text("Leave blank to keep existing")
                        .desired_width(350.0)
                );
                ui.end_row();

                ui.label("");
                let link = ui.add(egui::Label::new(
                    RichText::new("Generate API token at Atlassian")
                        .size(14.0)
                        .color(accent)
                ).sense(egui::Sense::click()));
                if link.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if link.clicked() {
                    let _ = open::that("https://id.atlassian.com/manage-profile/security/api-tokens");
                }
                ui.end_row();
            });

        ui.add_space(20.0);

        // === Display ===
        ui.label(RichText::new("Display").color(section_color).strong());
        ui.add_space(8.0);

        egui::Grid::new("display_grid")
            .num_columns(2)
            .spacing([20.0, 10.0])
            .show(ui, |ui| {
                ui.label("Font scale");
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.settings_font_scale, 0.75..=2.0).show_value(false));
                    ui.label(format!("{:.0}%", self.settings_font_scale * 100.0));
                });
                ui.end_row();

                ui.label("Duration format");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.settings_time_format, TimeFormat::HoursMinutes, "3h 15m");
                    ui.radio_value(&mut self.settings_time_format, TimeFormat::Decimal, "3.25h");
                });
                ui.end_row();

                ui.label("Clock format");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.settings_clock_format, ClockFormat::Hour24, "14:30");
                    ui.radio_value(&mut self.settings_clock_format, ClockFormat::Hour12, "2:30pm");
                });
                ui.end_row();
            });

        ui.add_space(20.0);

        // === Time Entry ===
        ui.label(RichText::new("Time Entry").color(section_color).strong());
        ui.add_space(8.0);

        egui::Grid::new("entry_grid")
            .num_columns(2)
            .spacing([20.0, 10.0])
            .show(ui, |ui| {
                ui.label("Start time field");
                ui.checkbox(&mut self.settings_show_start_time, "Show in dialogs");
                ui.end_row();

                ui.label("Category tags");
                ui.add(
                    egui::TextEdit::multiline(&mut self.settings_tags)
                        .hint_text("FE, BE, Bugfix, ...")
                        .desired_width(400.0)
                        .desired_rows(3)
                );
                ui.end_row();
            });

        ui.add_space(24.0);

        ui.horizontal(|ui| {
            // Custom buttons with hover effect
            let btn_bg = Color32::from_rgb(0x28, 0x28, 0x26);
            let btn_hover = Color32::from_rgb(0x50, 0x50, 0x4a);
            let text_color = Color32::from_rgb(180, 180, 190);
            let font_id = egui::FontId::proportional(17.0);
            let padding = egui::vec2(18.0, 10.0);
            let rounding = egui::Rounding::same(6.0);

            // Save button
            let save_text = "Save";
            let save_size = ui.fonts(|f| f.layout_no_wrap(save_text.to_string(), font_id.clone(), text_color).size());
            let (save_rect, save_response) = ui.allocate_exact_size(save_size + padding * 2.0, egui::Sense::click());
            let save_bg = if save_response.hovered() { btn_hover } else { btn_bg };
            ui.painter().rect_filled(save_rect, rounding, save_bg);
            ui.painter().text(save_rect.center(), egui::Align2::CENTER_CENTER, save_text, font_id.clone(), text_color);
            if save_response.clicked() {
                self.save_settings();
            }

            // Cancel button
            let cancel_text = "Cancel";
            let cancel_size = ui.fonts(|f| f.layout_no_wrap(cancel_text.to_string(), font_id.clone(), text_color).size());
            let (cancel_rect, cancel_response) = ui.allocate_exact_size(cancel_size + padding * 2.0, egui::Sense::click());
            let cancel_bg = if cancel_response.hovered() { btn_hover } else { btn_bg };
            ui.painter().rect_filled(cancel_rect, rounding, cancel_bg);
            ui.painter().text(cancel_rect.center(), egui::Align2::CENTER_CENTER, cancel_text, font_id, text_color);
            if cancel_response.clicked() {
                self.show_settings = false;
            }
        });
    }
}

impl eframe::App for JiraTimeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle pinch-to-zoom (trackpad pinch or Ctrl+scroll)
        let zoom_delta = ctx.input(|i| i.zoom_delta());
        if zoom_delta != 1.0 {
            // Apply zoom to font scale, clamped to reasonable range
            self.config.font_scale = (self.config.font_scale * zoom_delta).clamp(0.75, 2.5);
            // Save config on zoom change (debounced by only saving when delta is significant)
            if (zoom_delta - 1.0).abs() > 0.01 {
                let _ = self.config.save();
            }
        }

        // Apply font scale
        ctx.set_pixels_per_point(self.config.font_scale);

        // Check for async results
        self.check_async_results();

        // Handle graceful restart after update
        if self.restart_pending {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }


        // Update progress bar animation
        let elapsed = self.progress_start.elapsed().as_secs_f32();
        match self.progress_phase {
            ProgressPhase::Idle => {}
            ProgressPhase::FastStart => {
                // 0→17% in 0.25 seconds
                self.progress = (elapsed / 0.25 * 0.17).min(0.17);
                if elapsed >= 0.25 {
                    self.progress_phase = ProgressPhase::SlowCrawl;
                    self.progress_start = std::time::Instant::now();
                }
                ctx.request_repaint();
            }
            ProgressPhase::SlowCrawl => {
                // 17%→50% over ~7.5 seconds
                self.progress = 0.17 + (elapsed / 7.5 * 0.33).min(0.33);
                ctx.request_repaint();
            }
            ProgressPhase::Completing => {
                // Snap to 100% fast (0.15 seconds)
                let t = (elapsed / 0.15).min(1.0);
                self.progress = self.progress + (1.0 - self.progress) * t;
                if elapsed >= 0.15 {
                    self.progress = 1.0;
                    self.progress_phase = ProgressPhase::FadingOut;
                    self.progress_start = std::time::Instant::now();
                }
                ctx.request_repaint();
            }
            ProgressPhase::FadingOut => {
                // Fade out over 0.3 seconds
                if elapsed >= 0.3 {
                    self.progress_phase = ProgressPhase::Idle;
                    self.progress = 0.0;
                }
                ctx.request_repaint();
            }
            ProgressPhase::Shrinking => {
                // Shrink to 0 fast (0.2 seconds)
                let t = (elapsed / 0.2).min(1.0);
                self.progress = self.progress * (1.0 - t);
                if elapsed >= 0.2 {
                    self.progress_phase = ProgressPhase::Idle;
                    self.progress = 0.0;
                }
                ctx.request_repaint();
            }
        }

        // Render the dialog window if open
        if self.show_dialog {
            let title = match self.dialog_mode {
                DialogMode::Add => "Log time",
                DialogMode::Edit => "Edit log",
            };

            let mut selected_issue: Option<(String, String, String)> = None;
            let mut close_requested = false;

            let (content_bg, frame_color, _) = super::theme::dialog_colors();
            let dialog_frame = egui::Frame::none()
                .fill(content_bg)
                .stroke(egui::Stroke::new(2.0, frame_color))
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::same(20.0));

            egui::Window::new(title)
                .collapsible(false)
                .resizable(true)
                .default_width(600.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .title_bar(true)
                .frame(dialog_frame)
                .show(ctx, |ui| {
                    ui.set_min_width(550.0);

                    // Quick-add buttons at top of Add dialog (only when issue not yet selected)
                    if matches!(self.dialog_mode, DialogMode::Add) && self.validated_issue.is_none() && !self.weekly_buckets.is_empty() {
                        ui.horizontal(|ui| {
                            let btn_bg = Color32::from_rgb(0x2a, 0x2a, 0x32);
                            let btn_hover = Color32::from_rgb(0x45, 0x45, 0x50);

                            // (category, label, accent_color)
                            let button_config = [
                                ("MEETING", "Meeting", Color32::from_rgb(0xdc, 0x26, 0x7f)),
                                ("SUPPORT", "Support", Color32::from_rgb(0xfe, 0x61, 0x00)),
                                ("ADMIN", "Admin", Color32::from_rgb(0xff, 0xb0, 0x00)),
                            ];

                            for (cat, label, accent_color) in button_config {
                                if let Some((issue_key, issue_summary, issue_type)) = self.weekly_buckets.get(cat) {
                                    let btn_text = format!("{} {}", egui_phosphor::regular::PLUS, label);
                                    let font_id = egui::FontId::proportional(14.0);
                                    let text_size = ui.fonts(|f| f.layout_no_wrap(btn_text.clone(), font_id.clone(), Color32::WHITE).size());
                                    let padding = egui::vec2(14.0, 8.0);
                                    let button_size = text_size + padding * 2.0;

                                    let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());
                                    let bg = if response.hovered() { btn_hover } else { btn_bg };
                                    ui.painter().rect_filled(rect, 6.0, bg);
                                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &btn_text, font_id, accent_color);

                                    if response.on_hover_text(format!("[{}] {}", issue_key, issue_summary)).clicked() {
                                        self.dialog_issue = issue_key.clone();
                                        self.validated_issue = Some((issue_key.clone(), issue_summary.clone(), issue_type.clone()));
                                        self.dialog_accent_color = Some(accent_color);
                                    }
                                }
                            }
                        });
                        ui.add_space(12.0);
                    }

                    // Use Grid for proper label/input alignment
                    let mut issue_response_opt: Option<egui::Response> = None;

                    // Check if issue is validated (either pre-selected from quick buttons or manually selected)
                    let is_validated = self.validated_issue.as_ref()
                        .map(|(key, _, _)| key == &self.dialog_issue)
                        .unwrap_or(false);

                    egui::Grid::new("log_time_grid")
                        .num_columns(2)
                        .spacing([12.0, 10.0])
                        .show(ui, |ui| {
                            // Issue row: show colored KEY: Summary if validated, otherwise show text field
                            ui.label("Issue");
                            if is_validated {
                                // Show colored issue summary
                                if let Some((key, summary, _)) = &self.validated_issue {
                                    let accent = self.dialog_accent_color.unwrap_or(Color32::from_rgb(0x13, 0x98, 0xf4));
                                    ui.add(egui::Label::new(
                                        RichText::new(format!("[{}] {}", key, summary)).size(14.0).color(accent)
                                    ).truncate());
                                }
                            } else {
                                // Show text field with autocomplete
                                ui.horizontal(|ui| {
                                    let error_color = Color32::from_rgb(0xff, 0x44, 0x44);
                                    let issue_frame = if self.error_issue {
                                        egui::Frame::none()
                                            .stroke(egui::Stroke::new(2.0, error_color))
                                            .rounding(4.0)
                                            .inner_margin(2.0)
                                    } else {
                                        egui::Frame::none()
                                    };

                                    let issue_response = issue_frame.show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(&mut self.dialog_issue)
                                                .desired_width(350.0)
                                                .hint_text("Type to search issues...")
                                        )
                                    }).inner;

                                    if self.searching_issues {
                                        ui.spinner();
                                    }

                                    // Handle focus and text changes for autocomplete
                                    if issue_response.gained_focus() {
                                        self.show_suggestions = !self.issue_suggestions.is_empty();
                                    }

                                    if issue_response.changed() {
                                        // Clear error when user types
                                        self.error_issue = false;
                                        // Invalidate validation when text changes
                                        self.validated_issue = None;
                                        self.last_search_time = Instant::now();

                                        // Check if the typed text matches a suggestion exactly
                                        let typed_upper = self.dialog_issue.to_uppercase();
                                        if let Some(issue) = self.issue_suggestions.iter().find(|i| i.key == typed_upper) {
                                            let issue_type = issue.fields.issue_type.as_ref()
                                                .map(|t| t.name.clone())
                                                .unwrap_or_else(|| "Task".to_string());
                                            self.validated_issue = Some((issue.key.clone(), issue.fields.summary.clone(), issue_type));
                                            self.dialog_issue = issue.key.clone();
                                        }
                                    }

                                    // Debounced search (300ms after last keystroke)
                                    if issue_response.has_focus() && !self.searching_issues {
                                        let elapsed = self.last_search_time.elapsed().as_millis();
                                        if elapsed > 300 && self.last_issue_search != self.dialog_issue {
                                            self.search_issues(&self.dialog_issue.clone());
                                        }
                                    }

                                    issue_response_opt = Some(issue_response);
                                });
                            }
                            ui.end_row();

                            // Start time field (optional, controlled by settings)
                            if self.config.show_start_time {
                                ui.label("Start");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.dialog_start_time)
                                        .desired_width(150.0)
                                        .hint_text("9am")
                                );
                                ui.end_row();
                            }

                            // Duration field
                            ui.label("Duration");
                            let error_color = Color32::from_rgb(0xff, 0x44, 0x44);
                            let hours_frame = if self.error_hours {
                                egui::Frame::none()
                                    .stroke(egui::Stroke::new(2.0, error_color))
                                    .rounding(4.0)
                                    .inner_margin(2.0)
                            } else {
                                egui::Frame::none()
                            };
                            let hours_response = hours_frame.show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.dialog_hours)
                                        .desired_width(150.0)
                                        .hint_text("1h 30m")
                                )
                            }).inner;
                            if hours_response.changed() {
                                self.error_hours = false;
                            }
                            ui.end_row();
                        });

                    // Dropdown suggestions (outside grid, full width)
                    if self.show_suggestions && !self.issue_suggestions.is_empty() {
                        let dropdown_bg = ui.visuals().widgets.noninteractive.bg_fill;
                        egui::Frame::none()
                            .fill(dropdown_bg)
                            .rounding(egui::Rounding::same(4.0))
                            .inner_margin(egui::Margin::same(4.0))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(200.0)
                                    .show(ui, |ui| {
                                        for issue in &self.issue_suggestions {
                                            let text = format!("{} - {}", issue.key, issue.fields.summary);
                                            let display_text = if text.len() > 70 {
                                                format!("{}...", &text[..67])
                                            } else {
                                                text
                                            };

                                            let response = ui.selectable_label(
                                                false,
                                                RichText::new(&display_text).size(14.0)
                                            );

                                            if response.clicked() {
                                                let issue_type = issue.fields.issue_type.as_ref()
                                                    .map(|t| t.name.clone())
                                                    .unwrap_or_else(|| "Task".to_string());
                                                selected_issue = Some((issue.key.clone(), issue.fields.summary.clone(), issue_type));
                                            }
                                        }
                                    });
                            });
                    }

                    // Description outside the grid for more room
                    ui.add_space(15.0);
                    ui.label("Description");

                    // Category tags as small, minimal chips - dark by default, bright when selected
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for (i, tag) in self.config.tags.iter().enumerate() {
                            let selected = self.dialog_categories.get(i).copied().unwrap_or(false);
                            let font_id = egui::FontId::proportional(18.0);
                            let text_size = ui.fonts(|f| f.layout_no_wrap(tag.to_string(), font_id.clone(), Color32::WHITE).size());
                            let padding = egui::vec2(8.0, 4.0);
                            let button_size = text_size + padding * 2.0;

                            let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());

                            // Draw tag - dark by default, bright blue when selected
                            let (text_color, bg_color) = if selected {
                                (Color32::WHITE, Color32::from_rgb(19, 152, 244))
                            } else {
                                (Color32::from_rgb(120, 120, 130), Color32::TRANSPARENT)
                            };

                            if selected {
                                ui.painter().rect_filled(rect, egui::Rounding::same(3.0), bg_color);
                            }
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, tag.as_str(), font_id, text_color);

                            // Set pointer cursor
                            if response.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }

                            if response.clicked() {
                                if let Some(cat) = self.dialog_categories.get_mut(i) {
                                    *cat = !selected;
                                }
                            }
                        }
                    });

                    // Calculate max height for description - leave room for buttons below
                    // Use a reasonable max that keeps dialog within typical window bounds
                    let max_desc_height = (ctx.screen_rect().height() - 400.0).max(100.0).min(300.0);

                    egui::ScrollArea::vertical()
                        .max_height(max_desc_height)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.dialog_description)
                                    .desired_width(ui.available_width())
                                    .desired_rows(5)
                                    .hint_text("What did you work on?")
                            );
                        });

                    ui.add_space(14.0);

                    // Progress bar for saving
                    if self.progress_phase != ProgressPhase::Idle {
                        let bar_height = 4.0;
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), bar_height), egui::Sense::hover());
                        if ui.is_rect_visible(rect) && self.progress > 0.0 {
                            let painter = ui.painter();
                            let fill_width = rect.width() * self.progress;
                            let bar_rect = egui::Rect::from_min_size(
                                rect.min,
                                egui::vec2(fill_width, bar_height)
                            );
                            // Fade out during FadingOut phase - use blue accent color
                            let alpha = if self.progress_phase == ProgressPhase::FadingOut {
                                let t = self.progress_start.elapsed().as_secs_f32() / 0.3;
                                ((1.0 - t) * 255.0) as u8
                            } else {
                                255
                            };
                            painter.rect_filled(bar_rect, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, alpha));
                        }
                        ui.add_space(10.0);
                    }

                    ui.add_enabled_ui(!self.loading, |ui| {
                        ui.horizontal(|ui| {
                            // Subdued buttons - dark gray bg, lighter on hover (like quick-add buttons)
                            let btn_bg = Color32::from_rgb(0x2a, 0x2a, 0x32);
                            let btn_hover = Color32::from_rgb(0x45, 0x45, 0x50);
                            let text_color = Color32::from_rgb(180, 180, 190);
                            let font_id = egui::FontId::proportional(17.0);
                            let padding = egui::vec2(18.0, 10.0);
                            let rounding = egui::Rounding::same(6.0);

                            // Save button - custom rendered for hover effect
                            let save_text = "Save";
                            let save_size = ui.fonts(|f| f.layout_no_wrap(save_text.to_string(), font_id.clone(), text_color).size());
                            let (save_rect, save_response) = ui.allocate_exact_size(save_size + padding * 2.0, egui::Sense::click());
                            let save_bg = if save_response.hovered() { btn_hover } else { btn_bg };
                            ui.painter().rect_filled(save_rect, rounding, save_bg);
                            ui.painter().text(save_rect.center(), egui::Align2::CENTER_CENTER, save_text, font_id.clone(), text_color);
                            if save_response.clicked() {
                                self.save_dialog();
                            }

                            // Cancel button - custom rendered for hover effect
                            let cancel_text = "Cancel";
                            let cancel_size = ui.fonts(|f| f.layout_no_wrap(cancel_text.to_string(), font_id.clone(), text_color).size());
                            let (cancel_rect, cancel_response) = ui.allocate_exact_size(cancel_size + padding * 2.0, egui::Sense::click());
                            let cancel_bg = if cancel_response.hovered() { btn_hover } else { btn_bg };
                            ui.painter().rect_filled(cancel_rect, rounding, cancel_bg);
                            ui.painter().text(cancel_rect.center(), egui::Align2::CENTER_CENTER, cancel_text, font_id, text_color);
                            if cancel_response.clicked() {
                                close_requested = true;
                            }
                        });
                    });
                });

            // Handle issue selection (after window closure for borrow checker)
            if let Some((key, summary, issue_type)) = selected_issue {
                self.dialog_issue = key.clone();
                self.validated_issue = Some((key, summary, issue_type));
                self.show_suggestions = false;
            }
            if close_requested {
                self.show_dialog = false;
            }
        }

        // Render settings dialog if open
        if self.show_settings {
            let (content_bg, frame_color, frame_text) = super::theme::dialog_colors();
            let dialog_frame = egui::Frame::none()
                .fill(content_bg)
                .stroke(egui::Stroke::new(2.0, frame_color))
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::same(20.0));

            egui::Window::new("Settings")
                .collapsible(false)
                .resizable(false)
                .default_width(750.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .frame(dialog_frame)
                .show(ctx, |ui| {
                    self.render_settings_with_colors(ui, frame_color, frame_text);
                });
        }

        // Render delete confirmation dialog
        if self.show_delete_confirm {
            let mut do_delete = false;
            let mut cancel_delete = false;

            let (content_bg, frame_color, _) = super::theme::dialog_colors();
            let dialog_frame = egui::Frame::none()
                .fill(content_bg)
                .stroke(egui::Stroke::new(2.0, frame_color))
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::same(20.0));

            egui::Window::new("Confirm Delete")
                .collapsible(false)
                .resizable(false)
                .default_width(400.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .frame(dialog_frame)
                .show(ctx, |ui| {
                    ui.add_space(10.0);

                    if let Some(entry) = &self.pending_delete {
                        ui.label(RichText::new("Delete this time entry?").size(14.0));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&entry.issue_key).strong());
                            ui.label("-");
                            ui.add(egui::Label::new(&entry.issue_summary).truncate());
                        });
                        ui.label(format_duration_with_format(entry.seconds, self.config.time_format));
                    }

                    ui.add_space(20.0);

                    ui.horizontal(|ui| {
                        // Custom buttons with hover effect
                        let btn_bg = Color32::from_rgb(0x28, 0x28, 0x26);
                        let btn_hover = Color32::from_rgb(0x50, 0x50, 0x4a);
                        let text_color = Color32::from_rgb(180, 180, 190);
                        let delete_color = Color32::from_rgb(224, 108, 117);
                        let font_id = egui::FontId::proportional(17.0);
                        let padding = egui::vec2(18.0, 10.0);
                        let rounding = egui::Rounding::same(6.0);

                        // Delete button - red text for emphasis
                        let delete_text = "Delete";
                        let delete_size = ui.fonts(|f| f.layout_no_wrap(delete_text.to_string(), font_id.clone(), delete_color).size());
                        let (delete_rect, delete_response) = ui.allocate_exact_size(delete_size + padding * 2.0, egui::Sense::click());
                        let delete_bg = if delete_response.hovered() { btn_hover } else { btn_bg };
                        ui.painter().rect_filled(delete_rect, rounding, delete_bg);
                        ui.painter().text(delete_rect.center(), egui::Align2::CENTER_CENTER, delete_text, font_id.clone(), delete_color);
                        if delete_response.clicked() {
                            do_delete = true;
                        }

                        // Cancel button
                        let cancel_text = "Cancel";
                        let cancel_size = ui.fonts(|f| f.layout_no_wrap(cancel_text.to_string(), font_id.clone(), text_color).size());
                        let (cancel_rect, cancel_response) = ui.allocate_exact_size(cancel_size + padding * 2.0, egui::Sense::click());
                        let cancel_bg = if cancel_response.hovered() { btn_hover } else { btn_bg };
                        ui.painter().rect_filled(cancel_rect, rounding, cancel_bg);
                        ui.painter().text(cancel_rect.center(), egui::Align2::CENTER_CENTER, cancel_text, font_id, text_color);
                        if cancel_response.clicked() {
                            cancel_delete = true;
                        }
                    });
                });

            if do_delete {
                if let Some(entry) = self.pending_delete.take() {
                    self.delete_worklog(&entry);
                }
                self.show_delete_confirm = false;
            }
            if cancel_delete {
                self.pending_delete = None;
                self.show_delete_confirm = false;
            }
        }

        // Update overlay - blocks interaction while downloading
        if self.update_applying {
            egui::Area::new(egui::Id::new("update_overlay"))
                .fixed_pos(egui::Pos2::ZERO)
                .show(ctx, |ui| {
                    let screen = ctx.screen_rect();
                    ui.allocate_exact_size(screen.size(), egui::Sense::click()); // Block clicks
                    let painter = ui.painter();
                    // Semi-transparent background
                    painter.rect_filled(screen, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 200));

                    // Centered content
                    let center = screen.center();
                    let box_width = 300.0;
                    let box_height = 80.0;
                    let box_rect = egui::Rect::from_center_size(center, egui::vec2(box_width, box_height));

                    // Background box
                    painter.rect_filled(box_rect, 8.0, Color32::from_rgb(0x1e, 0x1e, 0x1e));

                    // "Updating..." text
                    let text_pos = egui::pos2(center.x, center.y - 15.0);
                    painter.text(text_pos, egui::Align2::CENTER_CENTER, "Updating...", egui::FontId::proportional(18.0), Color32::WHITE);

                    // Progress bar
                    let bar_width = box_width - 40.0;
                    let bar_height = 6.0;
                    let bar_y = center.y + 15.0;
                    let bar_bg = egui::Rect::from_center_size(egui::pos2(center.x, bar_y), egui::vec2(bar_width, bar_height));
                    painter.rect_filled(bar_bg, 3.0, Color32::from_rgb(0x3a, 0x3a, 0x3a));

                    // Progress fill
                    let fill_width = bar_width * self.progress;
                    let fill_rect = egui::Rect::from_min_size(
                        egui::pos2(bar_bg.min.x, bar_bg.min.y),
                        egui::vec2(fill_width, bar_height)
                    );
                    painter.rect_filled(fill_rect, 3.0, Color32::from_rgb(0x13, 0x98, 0xf4));
                });
        }

        egui::CentralPanel::default().frame(
            egui::Frame::none().inner_margin(egui::Margin::symmetric(12.0, 0.0))
        ).show(ctx, |ui| {
            // Progress bar at top (fixed height, no layout shift)
            let bar_height = 4.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), bar_height), egui::Sense::hover());

            if self.progress_phase != ProgressPhase::Idle && ui.is_rect_visible(rect) && self.progress > 0.0 {
                let painter = ui.painter();
                let fill_width = rect.width() * self.progress;
                let bar_rect = egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(fill_width, bar_height)
                );
                // Fade out during FadingOut phase - use blue accent color
                let alpha = if self.progress_phase == ProgressPhase::FadingOut {
                    let t = self.progress_start.elapsed().as_secs_f32() / 0.3;
                    ((1.0 - t) * 255.0) as u8
                } else {
                    255
                };
                painter.rect_filled(bar_rect, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, alpha));
            }

            // Status message (errors only) - selectable with copy and close buttons
            let mut dismiss_message = false;
            let mut copy_message: Option<String> = None;
            if !self.loading {
                if let Some((msg, is_error)) = &self.status_message {
                    let color = if *is_error {
                        Color32::from_rgb(224, 108, 117)
                    } else {
                        Color32::from_rgb(152, 195, 121)
                    };
                    let dim_color = Color32::from_rgb(120, 120, 130);
                    ui.horizontal(|ui| {
                        // Selectable text (can copy manually)
                        ui.add(egui::Label::new(RichText::new(msg).color(color)));

                        ui.add_space(8.0);

                        // Copy button
                        let copy_btn = ui.add(egui::Label::new(
                            RichText::new(egui_phosphor::regular::COPY).size(14.0).color(dim_color)
                        ).sense(egui::Sense::click()));
                        if copy_btn.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if copy_btn.clicked() {
                            copy_message = Some(msg.clone());
                        }

                        // Close button
                        let close_btn = ui.add(egui::Label::new(
                            RichText::new(egui_phosphor::regular::X).size(14.0).color(dim_color)
                        ).sense(egui::Sense::click()));
                        if close_btn.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if close_btn.clicked() {
                            dismiss_message = true;
                        }
                    });
                    ui.add_space(8.0);
                }
            }
            if let Some(text) = copy_message {
                ui.ctx().copy_text(text);
            }
            if dismiss_message {
                self.status_message = None;
            }

            match self.state {
                AppState::Setup => self.render_setup(ui),
                AppState::Main => self.render_main(ui),
            }
        });
    }
}
