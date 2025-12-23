use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "accountId")]
    pub account_id: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(rename = "emailAddress")]
    pub email_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub key: String,
    #[serde(rename = "self")]
    pub self_url: String,
    pub fields: IssueFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueFields {
    pub summary: String,
    pub project: Option<Project>,
    #[serde(rename = "issuetype")]
    pub issue_type: Option<IssueType>,
    pub timespent: Option<i64>,
    #[serde(rename = "timeoriginalestimate")]
    pub time_original_estimate: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueType {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worklog {
    pub id: String,
    #[serde(rename = "self")]
    pub self_url: String,
    pub author: User,
    #[serde(rename = "timeSpent")]
    pub time_spent: String,
    #[serde(rename = "timeSpentSeconds")]
    pub time_spent_seconds: i64,
    pub started: String,
    pub comment: Option<WorklogComment>,
    #[serde(rename = "issueId")]
    pub issue_id: Option<String>,
}

// WorklogComment uses serde_json::Value to handle Jira's flexible ADF format
// which can contain paragraphs, bullet lists, code blocks, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorklogComment {
    #[serde(rename = "type")]
    pub doc_type: String,
    #[serde(default)]
    pub content: Option<Vec<serde_json::Value>>,
}

impl Worklog {
    /// Extract comment as markdown for editing
    pub fn comment_text(&self) -> String {
        let Some(comment) = &self.comment else {
            return String::new();
        };
        let Some(contents) = &comment.content else {
            return String::new();
        };

        let mut lines: Vec<String> = Vec::new();
        for node in contents {
            extract_adf_to_markdown(node, &mut lines, 0);
        }

        // Clean up: remove leading/trailing empty lines and excessive whitespace
        let result = lines.join("\n");
        result.trim().to_string()
    }
}

// ============================================================================
// ADF → Markdown conversion (for editing existing worklogs)
// ============================================================================

/// Recursively extract ADF nodes to markdown format
fn extract_adf_to_markdown(node: &serde_json::Value, lines: &mut Vec<String>, indent: usize) {
    let Some(obj) = node.as_object() else {
        return;
    };

    let node_type = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match node_type {
        "text" => {
            // Leaf text node - extract text with markdown formatting for marks
            if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                let formatted = apply_marks_to_text(text, obj.get("marks"));
                if lines.is_empty() {
                    lines.push(String::new());
                }
                if let Some(last) = lines.last_mut() {
                    last.push_str(&formatted);
                }
            }
        }
        "paragraph" => {
            // Block that contains inline content - start a new line
            let prefix = "  ".repeat(indent);
            lines.push(prefix);
            if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    extract_adf_to_markdown(child, lines, indent);
                }
            }
        }
        "heading" => {
            // Heading with level
            let level = obj.get("attrs")
                .and_then(|a| a.get("level"))
                .and_then(|l| l.as_u64())
                .unwrap_or(1) as usize;
            let prefix = "#".repeat(level.min(6));
            lines.push(format!("{} ", prefix));
            if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    extract_adf_to_markdown(child, lines, 0);
                }
            }
        }
        "bulletList" => {
            if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    extract_list_item_markdown(child, lines, indent, "- ");
                }
            }
        }
        "orderedList" => {
            if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
                for (i, child) in content.iter().enumerate() {
                    let marker = format!("{}. ", i + 1);
                    extract_list_item_markdown(child, lines, indent, &marker);
                }
            }
        }
        "listItem" => {
            // Should be handled by bulletList/orderedList, but handle standalone
            if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    extract_adf_to_markdown(child, lines, indent);
                }
            }
        }
        "codeBlock" => {
            // Code block with optional language
            let lang = obj.get("attrs")
                .and_then(|a| a.get("language"))
                .and_then(|l| l.as_str())
                .unwrap_or("");
            lines.push(format!("```{}", lang));
            if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    if let Some(text) = child.get("text").and_then(|t| t.as_str()) {
                        for code_line in text.lines() {
                            lines.push(code_line.to_string());
                        }
                    }
                }
            }
            lines.push("```".to_string());
        }
        "blockquote" => {
            // Blockquote - prefix with >
            if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
                let start_idx = lines.len();
                for child in content {
                    extract_adf_to_markdown(child, lines, indent);
                }
                // Add > prefix to lines added by this blockquote
                for line in lines.iter_mut().skip(start_idx) {
                    if !line.is_empty() {
                        *line = format!("> {}", line);
                    } else {
                        *line = ">".to_string();
                    }
                }
            }
        }
        "hardBreak" => {
            // Hard line break within a paragraph
            lines.push(String::new());
        }
        _ => {
            // Unknown node type - try to recurse into content
            if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    extract_adf_to_markdown(child, lines, indent);
                }
            }
        }
    }
}

/// Apply ADF marks to text, converting to markdown syntax
fn apply_marks_to_text(text: &str, marks: Option<&serde_json::Value>) -> String {
    let Some(marks_array) = marks.and_then(|m| m.as_array()) else {
        return text.to_string();
    };

    let mut result = text.to_string();

    // Collect mark types
    let mut has_strong = false;
    let mut has_em = false;
    let mut has_code = false;
    let mut has_strike = false;

    for mark in marks_array {
        if let Some(mark_type) = mark.get("type").and_then(|t| t.as_str()) {
            match mark_type {
                "strong" => has_strong = true,
                "em" => has_em = true,
                "code" => has_code = true,
                "strike" => has_strike = true,
                _ => {}
            }
        }
    }

    // Apply marks in order (code innermost, then em, then strong, then strike)
    if has_code {
        result = format!("`{}`", result);
    }
    if has_em {
        result = format!("*{}*", result);
    }
    if has_strong {
        result = format!("**{}**", result);
    }
    if has_strike {
        result = format!("~~{}~~", result);
    }

    result
}

/// Extract text from a list item with proper indentation and marker (markdown format)
fn extract_list_item_markdown(node: &serde_json::Value, lines: &mut Vec<String>, indent: usize, marker: &str) {
    let Some(obj) = node.as_object() else {
        return;
    };

    if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
        let mut first = true;
        for child in content {
            let child_type = child.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if child_type == "paragraph" {
                let prefix = if first {
                    first = false;
                    format!("{}{}", "  ".repeat(indent), marker)
                } else {
                    format!("{}  ", "  ".repeat(indent))
                };
                lines.push(prefix);
                if let Some(para_content) = child.get("content").and_then(|c| c.as_array()) {
                    for para_child in para_content {
                        extract_adf_to_markdown(para_child, lines, indent + 1);
                    }
                }
            } else if child_type == "bulletList" || child_type == "orderedList" {
                // Nested list
                extract_adf_to_markdown(child, lines, indent + 1);
            } else {
                extract_adf_to_markdown(child, lines, indent + 1);
            }
        }
    }
}

/// Parse time strings like "1h 30m", "2h", "45m", "1.5h", "90" (minutes), "4" (hours)
/// Bare integers 1-8 are treated as hours, 9+ as minutes
/// Returns seconds
pub fn parse_duration(input: &str) -> Option<i64> {
    let input = input.trim().to_lowercase();

    if input.is_empty() {
        return None;
    }

    let mut total_seconds: i64 = 0;
    let mut current_num = String::new();
    let mut has_unit = false;

    for c in input.chars() {
        if c.is_ascii_digit() || c == '.' {
            current_num.push(c);
        } else if c == 'h' {
            if let Ok(hours) = current_num.parse::<f32>() {
                total_seconds += (hours * 3600.0) as i64;
                has_unit = true;
            }
            current_num.clear();
        } else if c == 'm' {
            if let Ok(mins) = current_num.parse::<f32>() {
                total_seconds += (mins * 60.0) as i64;
                has_unit = true;
            }
            current_num.clear();
        } else if c == 's' {
            if let Ok(secs) = current_num.parse::<i64>() {
                total_seconds += secs;
                has_unit = true;
            }
            current_num.clear();
        } else if c.is_whitespace() {
            // ignore whitespace
        } else {
            return None; // invalid character
        }
    }

    // Handle trailing number without unit
    if !current_num.is_empty() {
        if let Ok(num) = current_num.parse::<f32>() {
            if has_unit {
                // If we already had units, trailing number is invalid
                return None;
            }
            // Bare number: if it has decimal, treat as hours
            // If integer 1-8, treat as hours (nobody logs 3 minutes)
            // If integer 9+, treat as minutes
            if current_num.contains('.') {
                total_seconds = (num * 3600.0) as i64;
            } else {
                let int_val = num as i64;
                if int_val >= 1 && int_val <= 8 {
                    total_seconds = int_val * 3600;
                } else {
                    total_seconds = (num * 60.0) as i64;
                }
            }
        }
    }

    if total_seconds > 0 {
        Some(total_seconds)
    } else {
        None
    }
}

use crate::config::TimeFormat;

/// Format seconds as "Xh Ym" string
pub fn format_duration(seconds: i64) -> String {
    let hours = seconds / 3600;
    let mins = (seconds % 3600) / 60;

    if hours > 0 && mins > 0 {
        format!("{}h {}m", hours, mins)
    } else if hours > 0 {
        format!("{}h", hours)
    } else if mins > 0 {
        format!("{}m", mins)
    } else {
        "0m".to_string()
    }
}

/// Format seconds based on user's preferred time format
pub fn format_duration_with_format(seconds: i64, time_format: TimeFormat) -> String {
    match time_format {
        TimeFormat::HoursMinutes => format_duration(seconds),
        TimeFormat::Decimal => {
            let hours = seconds as f32 / 3600.0;
            if hours == 0.0 {
                "0h".to_string()
            } else if hours == hours.floor() {
                format!("{}h", hours as i32)
            } else {
                // Format with 2 decimals, then trim trailing zeros
                let s = format!("{:.2}", hours);
                let trimmed = s.trim_end_matches('0').trim_end_matches('.');
                format!("{}h", trimmed)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorklogResponse {
    pub worklogs: Vec<Worklog>,
    pub total: i32,
    #[serde(rename = "maxResults")]
    pub max_results: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub issues: Vec<Issue>,
    #[serde(default)]
    pub total: Option<i32>,
    #[serde(rename = "maxResults", default)]
    pub max_results: Option<i32>,
    #[serde(rename = "nextPageToken", default)]
    pub next_page_token: Option<String>,
}

// ============================================================================
// Markdown → ADF conversion (for saving worklogs)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct CreateWorklogRequest {
    #[serde(rename = "timeSpentSeconds")]
    pub time_spent_seconds: i64,
    pub started: String,
    pub comment: serde_json::Value,
}

impl CreateWorklogRequest {
    pub fn from_seconds_with_time(seconds: i64, date: NaiveDate, description: &str, start_time: Option<&str>) -> Self {
        let started = super::time::build_jira_timestamp(date, start_time);
        let comment = markdown_to_adf(description);

        Self {
            time_spent_seconds: seconds,
            started,
            comment,
        }
    }
}

/// Convert markdown text to ADF (Atlassian Document Format)
pub fn markdown_to_adf(markdown: &str) -> serde_json::Value {
    let blocks = parse_markdown_blocks(markdown);

    serde_json::json!({
        "type": "doc",
        "version": 1,
        "content": blocks
    })
}

/// Parse markdown into ADF block nodes
fn parse_markdown_blocks(markdown: &str) -> Vec<serde_json::Value> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = markdown.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Code block (```)
        if line.trim_start().starts_with("```") {
            let lang = line.trim_start().trim_start_matches('`').trim();
            let mut code_lines = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code_lines.push(lines[i]);
                i += 1;
            }
            blocks.push(create_code_block(&code_lines.join("\n"), lang));
            i += 1; // skip closing ```
            continue;
        }

        // Blockquote (>)
        if line.trim_start().starts_with("> ") || line.trim_start() == ">" {
            let mut quote_lines = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim_start();
                if l.starts_with("> ") {
                    quote_lines.push(&l[2..]);
                    i += 1;
                } else if l == ">" {
                    quote_lines.push("");
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(create_blockquote(&quote_lines.join("\n")));
            continue;
        }

        // Heading (#, ##, etc.)
        if line.trim_start().starts_with('#') {
            let trimmed = line.trim_start();
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if level <= 6 {
                let text = trimmed[level..].trim_start();
                blocks.push(create_heading(text, level));
                i += 1;
                continue;
            }
        }

        // Bullet list (- or *)
        if line.trim_start().starts_with("- ") || line.trim_start().starts_with("* ") {
            let (list_block, consumed) = parse_bullet_list(&lines[i..]);
            blocks.push(list_block);
            i += consumed;
            continue;
        }

        // Ordered list (1. 2. etc.)
        if is_ordered_list_item(line.trim_start()) {
            let (list_block, consumed) = parse_ordered_list(&lines[i..]);
            blocks.push(list_block);
            i += consumed;
            continue;
        }

        // Regular paragraph (or empty line)
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            blocks.push(create_paragraph(trimmed));
        }
        i += 1;
    }

    // Ensure at least one empty paragraph
    if blocks.is_empty() {
        blocks.push(create_paragraph(""));
    }

    blocks
}

/// Check if line starts with ordered list marker (1. 2. etc.)
fn is_ordered_list_item(line: &str) -> bool {
    let mut chars = line.chars().peekable();
    // Must start with digit
    if !chars.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        return false;
    }
    // Consume digits
    while chars.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        chars.next();
    }
    // Must be followed by ". "
    chars.next() == Some('.') && chars.next() == Some(' ')
}

/// Parse a bullet list, returns (ADF node, lines consumed)
fn parse_bullet_list(lines: &[&str]) -> (serde_json::Value, usize) {
    let mut items = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim_start();
        if line.starts_with("- ") || line.starts_with("* ") {
            let text = &line[2..];
            items.push(create_list_item(text));
            i += 1;
        } else if line.starts_with("  ") && !items.is_empty() {
            // Continuation of previous item (indented) - skip for simplicity
            i += 1;
        } else {
            break;
        }
    }

    (serde_json::json!({
        "type": "bulletList",
        "content": items
    }), i)
}

/// Parse an ordered list, returns (ADF node, lines consumed)
fn parse_ordered_list(lines: &[&str]) -> (serde_json::Value, usize) {
    let mut items = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim_start();
        if is_ordered_list_item(line) {
            // Find the text after "N. "
            if let Some(pos) = line.find(". ") {
                let text = &line[pos + 2..];
                items.push(create_list_item(text));
            }
            i += 1;
        } else if line.starts_with("  ") && !items.is_empty() {
            // Continuation of previous item - skip for simplicity
            i += 1;
        } else {
            break;
        }
    }

    (serde_json::json!({
        "type": "orderedList",
        "content": items
    }), i)
}

/// Create an ADF paragraph node with inline formatting
fn create_paragraph(text: &str) -> serde_json::Value {
    let inline_content = parse_inline_formatting(text);
    serde_json::json!({
        "type": "paragraph",
        "content": inline_content
    })
}

/// Create an ADF heading node
fn create_heading(text: &str, level: usize) -> serde_json::Value {
    let inline_content = parse_inline_formatting(text);
    serde_json::json!({
        "type": "heading",
        "attrs": { "level": level },
        "content": inline_content
    })
}

/// Create an ADF code block node
fn create_code_block(code: &str, language: &str) -> serde_json::Value {
    let mut node = serde_json::json!({
        "type": "codeBlock",
        "content": [{
            "type": "text",
            "text": code
        }]
    });
    if !language.is_empty() {
        node["attrs"] = serde_json::json!({ "language": language });
    }
    node
}

/// Create an ADF blockquote node
fn create_blockquote(text: &str) -> serde_json::Value {
    // Parse the blockquote content as paragraphs
    let paragraphs: Vec<serde_json::Value> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| create_paragraph(line))
        .collect();

    let content = if paragraphs.is_empty() {
        vec![create_paragraph("")]
    } else {
        paragraphs
    };

    serde_json::json!({
        "type": "blockquote",
        "content": content
    })
}

/// Create an ADF list item node
fn create_list_item(text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "listItem",
        "content": [{
            "type": "paragraph",
            "content": parse_inline_formatting(text)
        }]
    })
}

/// Parse inline formatting (bold, italic, code, strikethrough)
fn parse_inline_formatting(text: &str) -> Vec<serde_json::Value> {
    if text.is_empty() {
        return vec![serde_json::json!({ "type": "text", "text": "" })];
    }

    let mut result = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Try to match inline patterns
        if let Some((before, content, marks, after)) = try_match_inline(remaining) {
            // Add any text before the match
            if !before.is_empty() {
                result.push(serde_json::json!({ "type": "text", "text": before }));
            }
            // Add the formatted text
            let mut node = serde_json::json!({ "type": "text", "text": content });
            if !marks.is_empty() {
                node["marks"] = serde_json::json!(marks);
            }
            result.push(node);
            remaining = after;
        } else {
            // No match found, add rest as plain text
            result.push(serde_json::json!({ "type": "text", "text": remaining }));
            break;
        }
    }

    if result.is_empty() {
        result.push(serde_json::json!({ "type": "text", "text": "" }));
    }

    result
}

/// Try to match an inline formatting pattern
/// Returns (text_before, matched_content, marks, text_after) or None
fn try_match_inline(text: &str) -> Option<(&str, &str, Vec<serde_json::Value>, &str)> {
    // Order matters: check longer patterns first

    // Bold + Italic (***text***)
    if let Some(m) = find_pattern(text, "***", "***") {
        return Some((m.0, m.1, vec![
            serde_json::json!({"type": "strong"}),
            serde_json::json!({"type": "em"})
        ], m.2));
    }

    // Bold (**text**)
    if let Some(m) = find_pattern(text, "**", "**") {
        return Some((m.0, m.1, vec![serde_json::json!({"type": "strong"})], m.2));
    }

    // Strikethrough (~~text~~)
    if let Some(m) = find_pattern(text, "~~", "~~") {
        return Some((m.0, m.1, vec![serde_json::json!({"type": "strike"})], m.2));
    }

    // Inline code (`text`)
    if let Some(m) = find_pattern(text, "`", "`") {
        return Some((m.0, m.1, vec![serde_json::json!({"type": "code"})], m.2));
    }

    // Italic (*text* or _text_)
    if let Some(m) = find_pattern(text, "*", "*") {
        return Some((m.0, m.1, vec![serde_json::json!({"type": "em"})], m.2));
    }
    if let Some(m) = find_pattern(text, "_", "_") {
        return Some((m.0, m.1, vec![serde_json::json!({"type": "em"})], m.2));
    }

    None
}

/// Find a pattern with start and end delimiters
/// Returns (before, content, after) or None
fn find_pattern<'a>(text: &'a str, start: &str, end: &str) -> Option<(&'a str, &'a str, &'a str)> {
    let start_pos = text.find(start)?;
    let content_start = start_pos + start.len();
    let rest = &text[content_start..];

    // Find end delimiter (must not be immediately after start)
    let end_pos = rest.find(end)?;
    if end_pos == 0 {
        return None; // Empty content not allowed
    }

    let content = &rest[..end_pos];
    let after = &rest[end_pos + end.len()..];
    let before = &text[..start_pos];

    Some((before, content, after))
}

// Time entry display for the UI
#[derive(Debug, Clone)]
pub struct TimeEntry {
    pub worklog_id: String,
    pub issue_key: String,
    pub issue_summary: String,
    pub issue_type: String,  // "Task", "Bug", "Story", "Epic", etc.
    pub seconds: i64,
    pub description: String,
    pub date: NaiveDate,
    pub start_time: String,  // "HH:MM" format for sorting
}
