use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::NaiveDate;
use reqwest::{header, Client};

use super::types::*;
use crate::config::Config;

pub struct JiraClient {
    client: Client,
    base_url: String,
    auth_header: String,
}

impl JiraClient {
    pub fn new(config: &Config) -> Result<Self> {
        let token = config.api_token.as_ref()
            .context("API token not configured")?;

        let auth_string = format!("{}:{}", config.email, token);
        let auth_header = format!("Basic {}", STANDARD.encode(auth_string));

        let client = Client::builder()
            .build()?;

        Ok(Self {
            client,
            base_url: config.base_url(),
            auth_header,
        })
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, endpoint: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, endpoint);

        let response = self.client
            .get(&url)
            .header(header::AUTHORIZATION, &self.auth_header)
            .header(header::ACCEPT, "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API request failed: {} - {}", status, body);
        }

        let result = response.json::<T>().await?;
        Ok(result)
    }

    async fn post<T: serde::de::DeserializeOwned, B: serde::Serialize>(
        &self,
        endpoint: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, endpoint);

        let response = self.client
            .post(&url)
            .header(header::AUTHORIZATION, &self.auth_header)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .json(body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API request failed: {} - {}", status, body);
        }

        let result = response.json::<T>().await?;
        Ok(result)
    }

    async fn put<T: serde::de::DeserializeOwned, B: serde::Serialize>(
        &self,
        endpoint: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, endpoint);

        let response = self.client
            .put(&url)
            .header(header::AUTHORIZATION, &self.auth_header)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .json(body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API request failed: {} - {}", status, body);
        }

        let result = response.json::<T>().await?;
        Ok(result)
    }

    /// Get the current user
    pub async fn get_myself(&self) -> Result<User> {
        self.get("/myself").await
    }

    /// Search issues using JQL (using new /search/jql POST endpoint)
    pub async fn search_issues(&self, jql: &str, max_results: i32) -> Result<SearchResponse> {
        let request_body = serde_json::json!({
            "jql": jql,
            "maxResults": max_results,
            "fields": ["summary", "project", "timespent", "timeoriginalestimate"]
        });
        self.post("/search/jql", &request_body).await
    }

    /// Get worklogs for a specific issue
    pub async fn get_issue_worklogs(&self, issue_key: &str) -> Result<Vec<Worklog>> {
        let endpoint = format!("/issue/{}/worklog", issue_key);
        let response: WorklogResponse = self.get(&endpoint).await?;
        Ok(response.worklogs)
    }

    /// Get worklogs for current user within a date range
    /// Returns tuples of (issue_key, issue_summary, issue_type, worklog)
    pub async fn get_my_worklogs(&self, start_date: NaiveDate, end_date: NaiveDate) -> Result<Vec<(String, String, String, Worklog)>> {
        // Search for issues with worklogs by current user in date range
        let jql = format!(
            "worklogAuthor = currentUser() AND worklogDate >= '{}' AND worklogDate <= '{}' ORDER BY updated DESC",
            start_date.format("%Y-%m-%d"),
            end_date.format("%Y-%m-%d")
        );

        let issues = self.search_issues(&jql, 100).await?;
        let mut all_worklogs = Vec::new();

        // Get current user to filter worklogs
        let myself = self.get_myself().await?;

        for issue in issues.issues {
            // Skip issues that fail to fetch (permissions, network, etc.)
            let worklogs = match self.get_issue_worklogs(&issue.key).await {
                Ok(w) => w,
                Err(_) => continue,
            };

            let issue_type = issue.fields.issue_type
                .map(|t| t.name)
                .unwrap_or_else(|| "Task".to_string());

            for worklog in worklogs {
                // Filter to only current user's worklogs
                if worklog.author.account_id == myself.account_id {
                    // Parse worklog date and check if in range
                    if let Ok(worklog_date) = parse_worklog_date(&worklog.started) {
                        if worklog_date >= start_date && worklog_date <= end_date {
                            all_worklogs.push((
                                issue.key.clone(),
                                issue.fields.summary.clone(),
                                issue_type.clone(),
                                worklog,
                            ));
                        }
                    }
                }
            }
        }

        Ok(all_worklogs)
    }

    /// Log time to an issue
    pub async fn log_time(
        &self,
        issue_key: &str,
        seconds: i64,
        date: NaiveDate,
        description: &str,
        start_time: Option<&str>,
    ) -> Result<Worklog> {
        let endpoint = format!("/issue/{}/worklog", issue_key);
        let request = CreateWorklogRequest::from_seconds_with_time(seconds, date, description, start_time);
        self.post(&endpoint, &request).await
    }

    /// Update an existing worklog
    pub async fn update_worklog(
        &self,
        issue_key: &str,
        worklog_id: &str,
        seconds: i64,
        description: &str,
        date: chrono::NaiveDate,
        start_time: Option<&str>,
    ) -> Result<Worklog> {
        use crate::api::CreateWorklogRequest;

        // Build the started timestamp - defaults to 09:00 if empty/None
        let request_helper = CreateWorklogRequest::from_seconds_with_time(seconds, date, description, start_time);

        let endpoint = format!("/issue/{}/worklog/{}", issue_key, worklog_id);
        let request = serde_json::json!({
            "timeSpentSeconds": seconds,
            "started": request_helper.started,
            "comment": {
                "type": "doc",
                "version": 1,
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": description
                    }]
                }]
            }
        });
        self.put(&endpoint, &request).await
    }

    /// Delete a worklog
    pub async fn delete_worklog(&self, issue_key: &str, worklog_id: &str) -> Result<()> {
        let url = format!("{}/issue/{}/worklog/{}", self.base_url, issue_key, worklog_id);

        let response = self.client
            .delete(&url)
            .header(header::AUTHORIZATION, &self.auth_header)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API request failed: {} - {}", status, body);
        }

        Ok(())
    }

    /// Get TIME board bucket issues (Meetings, Support, Admin)
    pub async fn get_time_buckets(&self) -> Result<Vec<Issue>> {
        // Search for TIME board issues - adjust JQL as needed for your setup
        let jql = "project = TIM AND status = 'In Progress' ORDER BY key ASC";
        let response = self.search_issues(jql, 10).await?;
        Ok(response.issues)
    }

    /// Search issues by text (for autocomplete)
    pub async fn search_issues_by_text(&self, text: &str) -> Result<Vec<Issue>> {
        // If it looks like an issue key (e.g., "ABC-123" or "ABC"), search by key first
        let text_upper = text.to_uppercase();
        let jql = if text.contains('-') || text.chars().all(|c| c.is_alphabetic()) {
            // Likely an issue key or project prefix
            format!(
                "key = '{}' OR key ~ '{}' OR summary ~ '{}' ORDER BY lastViewed DESC",
                text_upper, text_upper, text
            )
        } else {
            // General text search
            format!(
                "text ~ '{}' ORDER BY lastViewed DESC",
                text
            )
        };
        let response = self.search_issues(&jql, 10).await?;
        Ok(response.issues)
    }

    /// Get recently viewed issues (for autocomplete suggestions)
    pub async fn get_recent_issues(&self) -> Result<Vec<Issue>> {
        let jql = "ORDER BY lastViewed DESC";
        let response = self.search_issues(jql, 15).await?;
        Ok(response.issues)
    }

    /// Search for a weekly bucket ticket by keyword and week dates
    /// Searches for tickets containing the keyword (e.g., "MEETING") AND a date from the week
    /// Uses Monday and Friday dates in multiple formats for robustness
    pub async fn search_weekly_bucket(&self, keyword: &str, week_start: NaiveDate) -> Result<Option<Issue>> {
        let monday = week_start;
        let friday = week_start + chrono::Duration::days(4);

        // Generate date patterns for both Monday and Friday in multiple formats
        // Examples for Dec 2, 2024: "12/2", "12/02", "Dec 2", "December 2"
        let mon_patterns = generate_date_patterns(monday);
        let fri_patterns = generate_date_patterns(friday);

        // Build JQL with OR conditions for all date patterns
        let date_conditions: Vec<String> = mon_patterns
            .iter()
            .chain(fri_patterns.iter())
            .map(|p| format!("summary ~ \"{}\"", p))
            .collect();

        let jql = format!(
            "summary ~ \"{}\" AND ({}) ORDER BY key DESC",
            keyword,
            date_conditions.join(" OR ")
        );

        let response = self.search_issues(&jql, 1).await?;
        Ok(response.issues.into_iter().next())
    }

    /// Search for all weekly bucket tickets for a given week
    /// Returns a map of category -> Issue for Meeting, Support, Admin
    pub async fn search_all_weekly_buckets(&self, week_start: NaiveDate) -> Result<Vec<(String, Issue)>> {
        let categories = ["MEETING", "SUPPORT", "ADMIN"];
        let mut results = Vec::new();

        for category in categories {
            if let Some(issue) = self.search_weekly_bucket(category, week_start).await? {
                results.push((category.to_string(), issue));
            }
        }

        Ok(results)
    }
}

/// Generate multiple date pattern strings for searching
fn generate_date_patterns(date: NaiveDate) -> Vec<String> {
    let year = date.format("%Y").to_string();             // "2025"
    let month = date.format("%m").to_string().trim_start_matches('0').to_string(); // "12" or "1"
    let day = date.format("%d").to_string().trim_start_matches('0').to_string();   // "2" or "31"
    let month_padded = date.format("%m").to_string();     // "12" or "01"
    let day_padded = date.format("%d").to_string();       // "02" or "31"
    let month_abbrev = date.format("%b").to_string();     // "Dec"
    let month_full = date.format("%B").to_string();       // "December"

    vec![
        // ISO format (most likely based on user feedback)
        format!("{}-{}-{}", year, month_padded, day_padded), // "2025-12-02"
        format!("{}-{}-{}", year, month, day),               // "2025-12-2"
        // US format with slashes
        format!("{}/{}", month, day),                        // "12/2"
        format!("{}/{}", month_padded, day_padded),          // "12/02"
        // Dash format without year
        format!("{}-{}", month_padded, day_padded),          // "12-02"
        format!("{}-{}", month, day),                        // "12-2"
        // Readable formats
        format!("{} {}", month_abbrev, day),                 // "Dec 2"
        format!("{} {}", month_full, day),                   // "December 2"
    ]
}

fn parse_worklog_date(started: &str) -> Result<NaiveDate> {
    // Format: "2025-12-02T09:00:00.000+0000"
    let date_part = started.split('T').next().unwrap_or(started);
    NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
        .context("Failed to parse worklog date")
}
