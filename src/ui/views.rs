use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use egui::{Color32, RichText, Ui};

use crate::api::{TimeEntry, format_duration_with_format};
use crate::config::{TimeFormat, ClockFormat, ListViewMode};
use super::theme::{day_tab_colors, day_tab_text_colors, entry_colors};

/// Result from schedule view interactions
#[derive(Default)]
pub struct ScheduleResult {
    pub edit_entry: Option<TimeEntry>,
    pub delete_entry: Option<TimeEntry>,
    pub add_at: Option<(NaiveDate, String)>,  // (date, start_time "HH:MM")
    // Drag move completed - entry moved to new time (optimistic update)
    pub drag_move: Option<(TimeEntry, String)>,  // (entry, new_start_time "HH:MM")
    // Drag resize completed - entry start/duration changed (optimistic update)
    pub drag_resize: Option<(TimeEntry, String, i64)>,  // (entry, new_start_time, new_seconds)
    // Ghost preview for new entries
    pub ghost_position: Option<(NaiveDate, String)>,  // (date, time) - where ghost should appear
    pub ghost_clicked: bool,  // User clicked on the ghost
}

/// Issue type icon style
enum IssueTypeIcon {
    /// White icon on colored square background (like Jira's Task icon)
    OnSquare(&'static str, Color32),
    /// Black icon on colored square background (for high-contrast items like bugs)
    OnSquareBlack(&'static str, Color32),
}

/// Get the icon style for an issue type
fn issue_type_icon(issue_type: &str) -> IssueTypeIcon {
    match issue_type.to_lowercase().as_str() {
        "bug" => IssueTypeIcon::OnSquareBlack(egui_phosphor::fill::BUG, Color32::from_rgb(0xe5, 0x4d, 0x42)),  // Black bug on red square
        "story" => IssueTypeIcon::OnSquare(egui_phosphor::fill::BOOKMARK_SIMPLE, Color32::from_rgb(0x65, 0xba, 0x43)),  // White bookmark on green square
        "epic" => IssueTypeIcon::OnSquare(egui_phosphor::fill::LIGHTNING, Color32::from_rgb(0x90, 0x4e, 0xe2)),  // White lightning on purple square
        _ => IssueTypeIcon::OnSquare(egui_phosphor::fill::CHECK_FAT, Color32::from_rgb(0x42, 0x9c, 0xd6)),  // White check on blue square (Task)
    }
}

/// Render an issue type icon
fn render_issue_type_icon(ui: &mut Ui, icon_style: IssueTypeIcon, size: f32) {
    // All icons are now rendered as colored squares with icon inside
    let (icon, bg_color, icon_color) = match icon_style {
        IssueTypeIcon::OnSquare(icon, bg_color) => (icon, bg_color, Color32::WHITE),
        IssueTypeIcon::OnSquareBlack(icon, bg_color) => (icon, bg_color, Color32::BLACK),
    };

    // Allocate space for the composite icon
    let icon_size = size + 2.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(icon_size, icon_size), egui::Sense::hover());

    // Draw rounded square background
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, bg_color);

    // Draw filled icon centered (uses phosphor-fill family)
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::new(size * 0.75, super::theme::phosphor_fill_family()),
        icon_color,
    );
}

/// Format a time string "HH:MM" according to clock format
fn format_clock_time(time_24: &str, clock_format: ClockFormat) -> String {
    match clock_format {
        ClockFormat::Hour24 => time_24.to_string(),
        ClockFormat::Hour12 => {
            // Parse "HH:MM" and convert to 12-hour format
            let parts: Vec<&str> = time_24.split(':').collect();
            if parts.len() >= 2 {
                if let (Ok(hour), Ok(min)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                    let (h12, ampm) = if hour == 0 {
                        (12, "am")
                    } else if hour < 12 {
                        (hour, "am")
                    } else if hour == 12 {
                        (12, "pm")
                    } else {
                        (hour - 12, "pm")
                    };
                    return format!("{}:{:02}{}", h12, min, ampm);
                }
            }
            time_24.to_string()
        }
    }
}

/// Represents a cached week of data
#[derive(Debug, Clone)]
pub struct WeekData {
    pub week_start: NaiveDate,
    pub entries: Vec<TimeEntry>,
}

impl WeekData {
    pub fn new(week_start: NaiveDate) -> Self {
        Self {
            week_start,
            entries: Vec::new(),
        }
    }

    /// Get entries for a specific day
    pub fn entries_for_day(&self, date: NaiveDate) -> Vec<&TimeEntry> {
        self.entries.iter().filter(|e| e.date == date).collect()
    }

    /// Get total seconds logged for a specific day
    pub fn seconds_for_day(&self, date: NaiveDate) -> i64 {
        self.entries.iter()
            .filter(|e| e.date == date)
            .map(|e| e.seconds)
            .sum()
    }

    /// Get day dates (Mon-Sun)
    pub fn all_days(&self) -> Vec<NaiveDate> {
        (0..7).map(|i| self.week_start + Duration::days(i)).collect()
    }
}

/// Returns (edit_index, delete_index, add_clicked) if Edit/Delete/Add was clicked
pub fn render_entry_list(
    ui: &mut Ui,
    entries: &[TimeEntry],
    jira_base_url: &str,
    time_format: TimeFormat,
    clock_format: ClockFormat,
    show_start_time: bool,
    list_view_mode: ListViewMode,
) -> (Option<usize>, Option<usize>, bool) {
    let mut edit_index = None;
    let mut delete_index = None;
    let mut add_clicked = false;

    egui::ScrollArea::vertical().show(ui, |ui| {
        // No extra spacing - cards handle their own gaps
        ui.spacing_mut().item_spacing.y = 0.0;

        for (idx, entry) in entries.iter().enumerate() {
            let (edit, delete) = match list_view_mode {
                ListViewMode::Contracted => render_entry_row_contracted(ui, entry, jira_base_url, time_format, clock_format, show_start_time),
                ListViewMode::Expanded => render_entry_row_expanded(ui, entry, jira_base_url, time_format, clock_format, show_start_time),
            };
            if edit {
                edit_index = Some(idx);
            }
            if delete {
                delete_index = Some(idx);
            }
        }

        // Add button at the end of the list
        if render_add_button(ui, entries.is_empty()) {
            add_clicked = true;
        }
    });

    (edit_index, delete_index, add_clicked)
}

/// Render the [+] add button at the end of the list
fn render_add_button(ui: &mut Ui, is_empty: bool) -> bool {
    let card_gap = 8.0;
    let button_height = if is_empty { 80.0 } else { 50.0 };
    let available_width = ui.available_width();

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available_width, button_height + card_gap),
        egui::Sense::click()
    );

    let button_rect = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(available_width, button_height)
    );

    let painter = ui.painter();
    let is_hovered = response.hovered();

    // Blue border, brighter on hover
    let border_color = if is_hovered {
        Color32::from_rgb(0x61, 0xAF, 0xEF)  // Bright blue
    } else {
        Color32::from_rgb(0x3A, 0x6E, 0x99)  // Muted blue
    };

    painter.rect_stroke(button_rect, 8.0, egui::Stroke::new(2.0, border_color));

    // Blue plus icon, white on hover
    let icon_color = if is_hovered { Color32::WHITE } else { Color32::from_rgb(0x61, 0xAF, 0xEF) };
    let icon_size = if is_empty { 32.0 } else { 24.0 };

    painter.text(
        button_rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::PLUS,
        egui::FontId::proportional(icon_size),
        icon_color
    );

    if is_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    response.clicked()
}

/// Returns (edit_clicked, delete_clicked) - Contracted view with single line
fn render_entry_row_contracted(ui: &mut Ui, entry: &TimeEntry, jira_base_url: &str, time_format: TimeFormat, _clock_format: ClockFormat, _show_start_time: bool) -> (bool, bool) {
    let mut edit_clicked = false;
    let mut delete_clicked = false;
    let (_bg_color, text_color, secondary_color) = entry_colors();

    // Accent color based on ticket type
    let accent_color = if entry.issue_key.starts_with("TIM-") {
        let summary_upper = entry.issue_summary.to_uppercase();
        if summary_upper.contains("MEETING") {
            Color32::from_rgb(0xe8, 0x28, 0x71)  // Pink/magenta
        } else if summary_upper.contains("SUPPORT") {
            Color32::from_rgb(0xec, 0x71, 0x1b)  // Orange
        } else if summary_upper.contains("ADMIN") {
            Color32::from_rgb(0xe5, 0xaa, 0x00)  // Yellow/gold
        } else {
            Color32::from_rgb(0x13, 0x98, 0xf4)  // Blue
        }
    } else {
        Color32::from_rgb(0x13, 0x98, 0xf4)  // Blue for regular tickets
    };

    // Card styling
    let card_bg = Color32::from_rgb(0x1c, 0x1c, 0x1a);
    let card_border = Color32::from_rgb(0x28, 0x28, 0x26);
    let accent_width = 4.0;
    let corner_radius = 6.0;
    let card_padding = 10.0;  // More padding left/right
    let card_gap = 6.0;

    // Calculate height (single line in contracted mode)
    let has_description = !entry.description.is_empty();
    let line_height = 24.0;
    let content_height = line_height;
    let total_height = content_height + card_padding * 2.0;

    // Create menu_id early so we can use it for card right-clicks
    let menu_id = ui.make_persistent_id(format!("entry_menu_{}", entry.worklog_id));

    // Allocate card space with gap - right-clickable for context menu
    let available_width = ui.available_width();
    let (full_rect, response) = ui.allocate_exact_size(
        egui::vec2(available_width, total_height + card_gap),
        egui::Sense::click()
    );

    // Handle right-click to open context menu
    if response.secondary_clicked() {
        ui.memory_mut(|mem| mem.toggle_popup(menu_id));
    }

    // Handle double-click to edit
    if response.double_clicked() {
        edit_clicked = true;
    }

    // Actual card rect (without gap)
    let card_rect = egui::Rect::from_min_size(
        full_rect.min,
        egui::vec2(available_width, total_height)
    );

    let is_hovered = response.hovered();
    let bg_color = card_bg;  // No hover state change

    let painter = ui.painter();

    // Draw card background with rounded corners
    painter.rect(
        card_rect,
        corner_radius,
        bg_color,
        egui::Stroke::new(1.0, card_border)
    );

    // Draw left accent stripe (rounded on left side only)
    let accent_rect = egui::Rect::from_min_size(
        card_rect.min,
        egui::vec2(accent_width + corner_radius, card_rect.height())
    );
    painter.rect(
        accent_rect,
        egui::Rounding {
            nw: corner_radius,
            sw: corner_radius,
            ne: 0.0,
            se: 0.0,
        },
        accent_color,
        egui::Stroke::NONE
    );
    // Cover the rounded right edge of accent with card background
    let cover_rect = egui::Rect::from_min_size(
        egui::pos2(card_rect.min.x + accent_width, card_rect.min.y),
        egui::vec2(corner_radius, card_rect.height())
    );
    painter.rect_filled(cover_rect, 0.0, bg_color);

    // Content area (after accent stripe)
    let content_left = card_rect.min.x + accent_width + card_padding;
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(content_left, card_rect.min.y + card_padding),
        egui::pos2(card_rect.max.x - card_padding, card_rect.max.y - card_padding)
    );
    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));

    // Single line: Icon + Issue key + Duration pill + Description + Menu
    let issue_url = format!("{}/browse/{}", jira_base_url, entry.issue_key);
    let icon_style = issue_type_icon(&entry.issue_type);
    let duration_text = format_duration_with_format(entry.seconds, time_format);

    // Issue key color - bright gray since we have colored icons now
    let issue_key_color = Color32::from_rgb(200, 200, 192);

    child_ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;  // Comfortable spacing between elements
        ui.set_height(24.0);

        // Issue type icon
        render_issue_type_icon(ui, icon_style, 12.0);  // Smaller to match text height

        // Issue key (clickable link) - bright gray
        let link_response = ui.add(egui::Label::new(
            RichText::new(&entry.issue_key)
                .size(14.0)
                .color(issue_key_color)
        ).sense(egui::Sense::click()));

        if link_response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if link_response.clicked() {
            let _ = open::that(&issue_url);
        }

        // Duration - white bold for times to stand out
        ui.add(egui::Label::new(
            RichText::new(&duration_text)
                .size(14.0)
                .family(super::theme::bold_family())
                .color(Color32::WHITE)
        ));

        // Description
        if has_description {
            ui.add(egui::Label::new(
                RichText::new(&entry.description)
                    .size(14.0)
                    .color(text_color)
            ).truncate());
        }

        // Actions menu (right-aligned)
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let menu_response = ui.add(egui::Label::new(
                RichText::new(egui_phosphor::regular::DOTS_THREE_VERTICAL)
                    .size(14.0)
                    .color(if is_hovered { text_color } else { secondary_color })
            ).sense(egui::Sense::click()));

            if menu_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            if menu_response.clicked() {
                ui.memory_mut(|mem| mem.toggle_popup(menu_id));
            }

            egui::popup::popup_below_widget(ui, menu_id, &menu_response, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(140.0);
                ui.style_mut().spacing.button_padding = egui::vec2(12.0, 8.0);

                if ui.add(egui::Button::new(
                    RichText::new(format!("{}  Edit log", egui_phosphor::regular::PENCIL_SIMPLE))
                        .size(14.0)
                ).frame(false)).clicked() {
                    edit_clicked = true;
                }

                if ui.add(egui::Button::new(
                    RichText::new(format!("{}  Delete log", egui_phosphor::regular::TRASH))
                        .size(14.0)
                ).frame(false)).clicked() {
                    delete_clicked = true;
                }
            });
        });
    });

    (edit_clicked, delete_clicked)
}

/// Returns (edit_clicked, delete_clicked) - Expanded view with wrapped description
fn render_entry_row_expanded(ui: &mut Ui, entry: &TimeEntry, jira_base_url: &str, time_format: TimeFormat, clock_format: ClockFormat, show_start_time: bool) -> (bool, bool) {
    let mut edit_clicked = false;
    let mut delete_clicked = false;
    let (_bg_color, text_color, secondary_color) = entry_colors();

    // Accent color based on ticket type
    let accent_color = if entry.issue_key.starts_with("TIM-") {
        let summary_upper = entry.issue_summary.to_uppercase();
        if summary_upper.contains("MEETING") {
            Color32::from_rgb(0xe8, 0x28, 0x71)  // Pink/magenta
        } else if summary_upper.contains("SUPPORT") {
            Color32::from_rgb(0xec, 0x71, 0x1b)  // Orange
        } else if summary_upper.contains("ADMIN") {
            Color32::from_rgb(0xe5, 0xaa, 0x00)  // Yellow/gold
        } else {
            Color32::from_rgb(0x13, 0x98, 0xf4)  // Blue
        }
    } else {
        Color32::from_rgb(0x13, 0x98, 0xf4)  // Blue for regular tickets
    };

    // Card styling
    let card_bg = Color32::from_rgb(0x1c, 0x1c, 0x1a);
    let card_border = Color32::from_rgb(0x28, 0x28, 0x26);
    let accent_width = 4.0;
    let corner_radius = 6.0;
    let card_padding = 10.0;
    let card_gap = 6.0;

    // Create menu_id early so we can use it for card right-clicks
    let menu_id = ui.make_persistent_id(format!("entry_menu_{}", entry.worklog_id));

    // Calculate dynamic height based on content
    let available_width = ui.available_width();
    let content_width = available_width - accent_width - card_padding * 2.0;
    let line_height = 24.0;
    let has_description = !entry.description.is_empty();
    let has_summary = !entry.issue_summary.is_empty();

    // Calculate description height if present (wrapped text)
    let description_height = if has_description {
        // Account for indent (icon width + spacing)
        let indent = 20.0;
        let desc_galley = ui.fonts(|f| {
            f.layout(
                entry.description.clone(),
                egui::FontId::new(14.0, egui::FontFamily::Proportional),
                text_color,
                content_width - indent,
            )
        });
        desc_galley.rect.height().max(line_height)
    } else {
        0.0
    };

    // Layout:
    // Line 1: Icon + Issue key + Duration (bold white) + Start time (optional) + Menu dots
    // Line 2: Summary/issue title (context)
    // Line 3+: Description (what you did - detail, dimmer)
    let line_spacing = 4.0;
    let mut content_height = line_height;  // Line 1 always present

    if has_summary {
        content_height += line_spacing + line_height;  // Line 2: summary/issue title
    }

    if has_description {
        content_height += line_spacing + description_height;  // Line 3+: description
    }

    let total_height = content_height + card_padding * 2.0;

    // Allocate card space with gap - right-clickable for context menu
    let (full_rect, response) = ui.allocate_exact_size(
        egui::vec2(available_width, total_height + card_gap),
        egui::Sense::click()
    );

    // Handle right-click to open context menu
    if response.secondary_clicked() {
        ui.memory_mut(|mem| mem.toggle_popup(menu_id));
    }

    // Handle double-click to edit
    if response.double_clicked() {
        edit_clicked = true;
    }

    // Actual card rect (without gap)
    let card_rect = egui::Rect::from_min_size(
        full_rect.min,
        egui::vec2(available_width, total_height)
    );

    let is_hovered = response.hovered();
    let bg_color = card_bg;

    let painter = ui.painter();

    // Draw card background with rounded corners
    painter.rect(
        card_rect,
        corner_radius,
        bg_color,
        egui::Stroke::new(1.0, card_border)
    );

    // Draw left accent stripe (rounded on left side only)
    let accent_rect = egui::Rect::from_min_size(
        card_rect.min,
        egui::vec2(accent_width + corner_radius, card_rect.height())
    );
    painter.rect(
        accent_rect,
        egui::Rounding {
            nw: corner_radius,
            sw: corner_radius,
            ne: 0.0,
            se: 0.0,
        },
        accent_color,
        egui::Stroke::NONE
    );
    // Cover the rounded right edge of accent with card background
    let cover_rect = egui::Rect::from_min_size(
        egui::pos2(card_rect.min.x + accent_width, card_rect.min.y),
        egui::vec2(corner_radius, card_rect.height())
    );
    painter.rect_filled(cover_rect, 0.0, bg_color);

    // Content area (after accent stripe)
    let content_left = card_rect.min.x + accent_width + card_padding;
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(content_left, card_rect.min.y + card_padding),
        egui::pos2(card_rect.max.x - card_padding, card_rect.max.y - card_padding)
    );
    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));

    // Line 1: Icon + Issue key + Duration (bold white) + Start time (optional) + Menu dots
    // Matches the contracted layout for visual consistency
    let issue_url = format!("{}/browse/{}", jira_base_url, entry.issue_key);
    let icon_style = issue_type_icon(&entry.issue_type);
    let duration_text = format_duration_with_format(entry.seconds, time_format);
    let issue_key_color = Color32::from_rgb(200, 200, 192);

    child_ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.set_height(24.0);

        // Issue type icon
        render_issue_type_icon(ui, icon_style, 12.0);

        // Issue key (clickable link)
        let link_response = ui.add(egui::Label::new(
            RichText::new(&entry.issue_key)
                .size(14.0)
                .color(issue_key_color)
        ).sense(egui::Sense::click()));

        if link_response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if link_response.clicked() {
            let _ = open::that(&issue_url);
        }

        // Duration - white bold (matching contracted style)
        ui.add(egui::Label::new(
            RichText::new(&duration_text)
                .size(14.0)
                .family(super::theme::bold_family())
                .color(Color32::WHITE)
        ));

        // Start time (optional, in secondary color)
        if show_start_time {
            let time_text = format_clock_time(&entry.start_time, clock_format);
            ui.add(egui::Label::new(
                RichText::new(&time_text)
                    .size(14.0)
                    .color(secondary_color)
            ));
        }

        // Actions menu (right-aligned)
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let menu_response = ui.add(egui::Label::new(
                RichText::new(egui_phosphor::regular::DOTS_THREE_VERTICAL)
                    .size(14.0)
                    .color(if is_hovered { text_color } else { secondary_color })
            ).sense(egui::Sense::click()));

            if menu_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            if menu_response.clicked() {
                ui.memory_mut(|mem| mem.toggle_popup(menu_id));
            }

            egui::popup::popup_below_widget(ui, menu_id, &menu_response, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(140.0);
                ui.style_mut().spacing.button_padding = egui::vec2(12.0, 8.0);

                if ui.add(egui::Button::new(
                    RichText::new(format!("{}  Edit log", egui_phosphor::regular::PENCIL_SIMPLE))
                        .size(14.0)
                ).frame(false)).clicked() {
                    edit_clicked = true;
                }

                if ui.add(egui::Button::new(
                    RichText::new(format!("{}  Delete log", egui_phosphor::regular::TRASH))
                        .size(14.0)
                ).frame(false)).clicked() {
                    delete_clicked = true;
                }
            });
        });
    });

    // Line 2: Summary/issue title (context, indented)
    if has_summary {
        child_ui.add_space(line_spacing);
        child_ui.horizontal(|ui| {
            ui.add_space(20.0);  // Indent: icon width (12) + spacing (8)
            ui.add(egui::Label::new(
                RichText::new(&entry.issue_summary)
                    .size(14.0)
                    .color(text_color)
            ).wrap());
        });
    }

    // Line 3+: Description (what you did - detail, dimmer color)
    if has_description {
        child_ui.add_space(line_spacing);
        child_ui.horizontal(|ui| {
            ui.add_space(20.0);  // Same indent as summary
            ui.add(egui::Label::new(
                RichText::new(&entry.description)
                    .size(14.0)
                    .color(secondary_color)
            ).wrap());
        });
    }

    (edit_clicked, delete_clicked)
}

pub fn week_start(date: NaiveDate) -> NaiveDate {
    let weekday = date.weekday();
    let days_from_monday = weekday.num_days_from_monday();
    date - Duration::days(days_from_monday as i64)
}

/// Determine if weekends should be shown based on:
/// - Today is Saturday or Sunday, OR
/// - Any entry in the week falls on Saturday or Sunday
pub fn should_show_weekends(week_data: &WeekData) -> bool {
    let today = Local::now().date_naive();

    // Show if today is a weekend
    if matches!(today.weekday(), Weekday::Sat | Weekday::Sun) {
        return true;
    }

    // Show if any entry is on a weekend
    for entry in &week_data.entries {
        if matches!(entry.date.weekday(), Weekday::Sat | Weekday::Sun) {
            return true;
        }
    }

    false
}

/// Render the day tabs with hours status and view mode toggle
/// Returns (clicked_day, view_mode_toggled)
pub fn render_day_tabs(
    ui: &mut Ui,
    week_data: &WeekData,
    selected_day: NaiveDate,
    time_format: TimeFormat,
    list_view_mode: ListViewMode,
) -> (Option<NaiveDate>, bool) {
    let today = Local::now().date_naive();
    let mut clicked_day = None;
    let mut view_mode_toggled = false;
    let show_weekends = should_show_weekends(week_data);

    let (bg_color, border_color, _accent) = day_tab_colors();

    ui.horizontal(|ui| {
        // Filter days based on whether weekends should be shown
        let days: Vec<NaiveDate> = week_data.all_days()
            .into_iter()
            .filter(|day| {
                if show_weekends {
                    true
                } else {
                    !matches!(day.weekday(), Weekday::Sat | Weekday::Sun)
                }
            })
            .collect();

        for day in days {
            let is_today = day == today;
            let is_selected = day == selected_day;

            // Show "Today" instead of day name if it's today
            let day_name = if is_today {
                "Today"
            } else {
                match day.weekday() {
                    Weekday::Mon => "Mon",
                    Weekday::Tue => "Tue",
                    Weekday::Wed => "Wed",
                    Weekday::Thu => "Thu",
                    Weekday::Fri => "Fri",
                    Weekday::Sat => "Sat",
                    Weekday::Sun => "Sun",
                }
            };

            let seconds = week_data.seconds_for_day(day);
            let is_future = day > today;

            // Show "0" for zero duration on past/current days, nothing for future days
            let hours_text = if seconds > 0 {
                format_duration_with_format(seconds, time_format)
            } else if is_future {
                String::new()  // Hide "0" on future days
            } else {
                "0".to_string()
            };

            // Determine styling - selected day gets brighter outline (white border for dark mode)
            let (fill_color, stroke, day_color, hours_color) = if is_selected {
                let (day_c, hours_c) = day_tab_text_colors(false);
                (bg_color, egui::Stroke::new(1.0, Color32::WHITE), day_c, hours_c)
            } else {
                let (day_c, hours_c) = day_tab_text_colors(false);
                (bg_color, egui::Stroke::new(1.0, border_color), day_c, hours_c)
            };

            // Use a fixed-size allocation for each tab
            let tab_size = egui::vec2(64.0, 64.0);
            let (rect, response) = ui.allocate_exact_size(tab_size, egui::Sense::click());

            if ui.is_rect_visible(rect) {
                let painter = ui.painter();

                // Background with optional border - rounded corners
                painter.rect(rect, 8.0, fill_color, stroke);

                // Day name - positioned higher (dimmer)
                painter.text(
                    egui::pos2(rect.center().x, rect.min.y + 24.0),
                    egui::Align2::CENTER_CENTER,
                    day_name,
                    egui::FontId::proportional(14.0),
                    day_color,
                );

                // Hours text - bold, positioned lower (brighter)
                painter.text(
                    egui::pos2(rect.center().x, rect.min.y + 44.0),
                    egui::Align2::CENTER_CENTER,
                    &hours_text,
                    egui::FontId::new(14.0, super::theme::bold_family()),
                    hours_color,
                );
            }

            if response.clicked() {
                clicked_day = Some(day);
            }

            // Add spacing between tabs
            ui.add_space(2.0);
        }

        // View mode toggle (right-aligned, vertically centered with tabs)
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (icon, tooltip) = match list_view_mode {
                ListViewMode::Contracted => (egui_phosphor::regular::ARROWS_OUT_SIMPLE, "Expand cards"),
                ListViewMode::Expanded => (egui_phosphor::regular::ARROWS_IN_SIMPLE, "Collapse cards"),
            };

            let icon_color = Color32::from_rgb(0x90, 0x90, 0x88);
            let icon_hover = Color32::WHITE;

            let response = ui.add(
                egui::Button::new(egui::RichText::new(icon).size(20.0).color(icon_color))
                    .fill(Color32::TRANSPARENT)
                    .stroke(egui::Stroke::NONE)
                    .min_size(egui::vec2(48.0, 48.0))
            );

            // Brighter icon on hover
            if response.hovered() {
                ui.painter().text(
                    response.rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icon,
                    egui::FontId::proportional(20.0),
                    icon_hover
                );
            }

            if response.on_hover_text(tooltip).clicked() {
                view_mode_toggled = true;
            }
        });
    });

    (clicked_day, view_mode_toggled)
}

/// Render the schedule/timeline view
pub fn render_schedule_view(
    ui: &mut Ui,
    week_data: &WeekData,
    _jira_base_url: &str,  // Reserved for future use (context menu links)
    time_format: TimeFormat,
    clock_format: ClockFormat,
    schedule_start_hour: u8,
    schedule_end_hour: u8,
) -> ScheduleResult {
    let mut result = ScheduleResult::default();
    let show_weekends = should_show_weekends(week_data);

    // Filter days based on whether weekends should be shown
    let days: Vec<NaiveDate> = week_data.all_days()
        .into_iter()
        .filter(|day| {
            if show_weekends {
                true
            } else {
                !matches!(day.weekday(), Weekday::Sat | Weekday::Sun)
            }
        })
        .collect();

    let today = Local::now().date_naive();

    // Calculate actual time range needed based on entries
    let mut actual_start_hour = schedule_start_hour;
    let mut actual_end_hour = schedule_end_hour;

    for day in &days {
        for entry in week_data.entries_for_day(*day) {
            let entry_start_minutes = parse_time_to_minutes(&entry.start_time);
            let entry_end_minutes = entry_start_minutes + (entry.seconds / 60) as i32;

            let entry_start_hour = (entry_start_minutes / 60) as u8;
            let entry_end_hour = ((entry_end_minutes + 59) / 60) as u8; // Round up

            if entry_start_hour < actual_start_hour {
                actual_start_hour = entry_start_hour;
            }
            if entry_end_hour > actual_end_hour {
                actual_end_hour = entry_end_hour.min(24);
            }
        }
    }

    // Use actual range (expanded if needed)
    let schedule_start_hour = actual_start_hour;
    let schedule_end_hour = actual_end_hour;

    // Layout constants
    let hour_label_width = 60.0;
    let header_height = 32.0;
    let hour_height = 60.0;  // Height per hour
    let grid_line_color = Color32::from_rgb(0x40, 0x40, 0x3c);
    let hour_line_color = Color32::from_rgb(0x50, 0x50, 0x4a);

    let num_hours = (schedule_end_hour - schedule_start_hour) as usize;
    let total_grid_height = num_hours as f32 * hour_height;

    let available_width = ui.available_width();
    let num_days = days.len();
    let day_width = (available_width - hour_label_width) / num_days as f32;

    // Fixed day headers (outside ScrollArea)
    let (header_rect, _) = ui.allocate_exact_size(
        egui::vec2(available_width, header_height),
        egui::Sense::hover()
    );

    let painter = ui.painter();

    for (i, day) in days.iter().enumerate() {
        let x = header_rect.min.x + hour_label_width + i as f32 * day_width;
        let col_header_rect = egui::Rect::from_min_size(
            egui::pos2(x, header_rect.min.y),
            egui::vec2(day_width, header_height)
        );

        let is_today = *day == today;

        // Day name
        let day_name = if is_today {
            "Today"
        } else {
            match day.weekday() {
                Weekday::Mon => "Mon",
                Weekday::Tue => "Tue",
                Weekday::Wed => "Wed",
                Weekday::Thu => "Thu",
                Weekday::Fri => "Fri",
                Weekday::Sat => "Sat",
                Weekday::Sun => "Sun",
            }
        };

        // Daily total - hide "0" on future days
        let seconds = week_data.seconds_for_day(*day);
        let is_future = *day > today;
        let hours_text = if seconds > 0 {
            crate::api::format_duration_with_format(seconds, time_format)
        } else if is_future {
            String::new()
        } else {
            "0".to_string()
        };

        // Combined: "Mon 6h 30m" left-justified
        let day_color = Color32::from_rgb(0xb0, 0xb0, 0xa8);
        let hours_color = Color32::WHITE;  // Bright white for times to stand out

        let text_left = col_header_rect.min.x + 8.0;
        let text_y = col_header_rect.center().y;

        // Day name
        let day_galley = painter.layout_no_wrap(
            day_name.to_string(),
            egui::FontId::proportional(14.0),
            day_color
        );
        let day_width_px = day_galley.rect.width();
        painter.galley(egui::pos2(text_left, text_y - day_galley.rect.height() / 2.0), day_galley, Color32::WHITE);

        // Hours (after day name with space) - bold white for times to stand out
        painter.text(
            egui::pos2(text_left + day_width_px + 8.0, text_y),
            egui::Align2::LEFT_CENTER,
            &hours_text,
            egui::FontId::new(14.0, super::theme::bold_family()),
            hours_color,
        );

        // Vertical separator line between columns
        if i > 0 {
            painter.line_segment(
                [
                    egui::pos2(x, col_header_rect.min.y + 4.0),
                    egui::pos2(x, col_header_rect.max.y - 4.0),
                ],
                egui::Stroke::new(1.0, grid_line_color),
            );
        }
    }

    // Scrollable grid area
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Allocate the grid area (without header)
        let (grid_rect, _) = ui.allocate_exact_size(
            egui::vec2(available_width, total_grid_height),
            egui::Sense::hover()
        );

        let painter = ui.painter();

        // Highlight current day column with dim background
        for (i, day) in days.iter().enumerate() {
            if *day == today {
                let col_x = grid_rect.min.x + hour_label_width + i as f32 * day_width;
                let col_rect = egui::Rect::from_min_size(
                    egui::pos2(col_x, grid_rect.min.y),
                    egui::vec2(day_width, total_grid_height)
                );
                painter.rect_filled(col_rect, 0.0, Color32::from_rgb(0x11, 0x11, 0x10));
                break;
            }
        }

        // Vertical grid lines for columns
        for (i, _day) in days.iter().enumerate() {
            let x = grid_rect.min.x + hour_label_width + i as f32 * day_width;
            painter.line_segment(
                [
                    egui::pos2(x, grid_rect.min.y),
                    egui::pos2(x, grid_rect.max.y),
                ],
                egui::Stroke::new(1.0, grid_line_color),
            );
        }

        // Right edge line
        painter.line_segment(
            [
                egui::pos2(grid_rect.max.x, grid_rect.min.y),
                egui::pos2(grid_rect.max.x, grid_rect.max.y),
            ],
            egui::Stroke::new(1.0, grid_line_color),
        );

        // Hour labels and horizontal grid lines
        for hour_idx in 0..=num_hours {
            let hour = schedule_start_hour + hour_idx as u8;
            let y = grid_rect.min.y + hour_idx as f32 * hour_height;

            // Hour label - aligned with the hour line, smaller and darker
            if hour_idx < num_hours {
                let hour_text = format_clock_time(&format!("{:02}:00", hour), clock_format);
                painter.text(
                    egui::pos2(grid_rect.min.x + hour_label_width - 8.0, y),
                    egui::Align2::RIGHT_TOP,
                    &hour_text,
                    egui::FontId::proportional(11.0),  // Smaller font for axis labels
                    Color32::from_rgb(0x70, 0x70, 0x68),  // Darker gray for less prominence
                );
            }

            // Horizontal line for full hours
            painter.line_segment(
                [
                    egui::pos2(grid_rect.min.x + hour_label_width, y),
                    egui::pos2(grid_rect.max.x, y),
                ],
                egui::Stroke::new(1.0, if hour_idx == 0 { hour_line_color } else { grid_line_color }),
            );

            // Draw 15-minute subdivision lines (solid, darker than hour lines)
            if hour_idx < num_hours {
                let quarter_color = Color32::from_rgb(0x24, 0x24, 0x22);
                let quarter_height = hour_height / 4.0;

                for quarter in 1..4 {
                    let quarter_y = y + quarter as f32 * quarter_height;
                    painter.line_segment(
                        [
                            egui::pos2(grid_rect.min.x + hour_label_width, quarter_y),
                            egui::pos2(grid_rect.max.x, quarter_y),
                        ],
                        egui::Stroke::new(1.0, quarter_color),
                    );
                }
            }
        }

        // Render entries as blocks
        let pixels_per_minute = hour_height / 60.0;
        let start_minutes = schedule_start_hour as i32 * 60;
        let end_minutes = schedule_end_hour as i32 * 60;

        // First pass: collect all entry rects and render them
        let mut all_entry_rects: Vec<egui::Rect> = Vec::new();

        // Drag state: (entry, original_start_minutes, original_end_minutes, press_time, original_col_x, drag_mode)
        // drag_mode: 0=move, 1=resize-top (change start), 2=resize-bottom (change duration)
        // Use egui memory to persist across frames
        let drag_id = ui.id().with("schedule_drag");
        type DragState = (TimeEntry, i32, i32, f64, f32, u8);
        let grabbed_state: Option<DragState> = ui.ctx().memory(|mem| {
            mem.data.get_temp::<DragState>(drag_id).clone()
        });

        let current_time = ui.ctx().input(|i| i.time);
        let long_press_threshold = 0.2; // 200ms for long-press to initiate drag
        let edge_threshold = 8.0; // pixels from edge to trigger resize mode

        // Check if we're in drag mode (past threshold)
        let in_drag_mode = grabbed_state.as_ref().map(|(_, _, _, press_time, _, _)| {
            current_time - press_time > long_press_threshold
        }).unwrap_or(false);

        // Get the dragged entry's worklog_id to skip rendering it at original position
        let dragged_worklog_id = if in_drag_mode {
            grabbed_state.as_ref().map(|(e, _, _, _, _, _)| e.worklog_id.clone())
        } else {
            None
        };

        for (day_idx, day) in days.iter().enumerate() {
            let day_entries = week_data.entries_for_day(*day);
            let col_x = grid_rect.min.x + hour_label_width + day_idx as f32 * day_width;

            for entry in day_entries {
                // Parse start time
                let entry_start_minutes = parse_time_to_minutes(&entry.start_time);
                let entry_end_minutes = entry_start_minutes + (entry.seconds / 60) as i32;

                // Skip if completely outside visible range
                if entry_end_minutes <= start_minutes || entry_start_minutes >= end_minutes {
                    continue;
                }

                // Clamp to visible range
                let visible_start = entry_start_minutes.max(start_minutes);
                let visible_end = entry_end_minutes.min(end_minutes);

                // Calculate Y position and height
                let y_start = grid_rect.min.y
                    + (visible_start - start_minutes) as f32 * pixels_per_minute;
                let height = (visible_end - visible_start) as f32 * pixels_per_minute;

                let block_margin = 2.0;
                // Subtract 2 pixels from height to create visual gap between adjacent blocks
                let block_rect = egui::Rect::from_min_size(
                    egui::pos2(col_x + block_margin, y_start),
                    egui::vec2(day_width - block_margin * 2.0, (height - 2.0).max(20.0))
                );

                all_entry_rects.push(block_rect);

                // Skip rendering if this entry is being dragged (we'll render it at mouse position)
                let is_being_dragged = dragged_worklog_id.as_ref() == Some(&entry.worklog_id);
                if !is_being_dragged {
                    // Render the entry (paint only)
                    render_schedule_entry_paint(ui, block_rect, entry, time_format);
                }

                // Check if pointer is over this entry manually
                let pointer_pos = ui.ctx().pointer_hover_pos();
                let pointer_over_entry = pointer_pos
                    .map(|pos| block_rect.contains(pos))
                    .unwrap_or(false);

                // Detect edge proximity for resize cursor
                let (near_top_edge, near_bottom_edge) = if let Some(pos) = pointer_pos {
                    if block_rect.contains(pos) {
                        let dist_from_top = pos.y - block_rect.min.y;
                        let dist_from_bottom = block_rect.max.y - pos.y;
                        (dist_from_top < edge_threshold, dist_from_bottom < edge_threshold)
                    } else {
                        (false, false)
                    }
                } else {
                    (false, false)
                };

                // Check if primary button JUST went down this frame
                let button_just_pressed = ui.ctx().input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));

                // Capture entry when click starts on it - store in memory with press time and drag mode
                if pointer_over_entry && button_just_pressed && grabbed_state.is_none() {
                    let drag_mode: u8 = if near_top_edge {
                        1 // resize-top
                    } else if near_bottom_edge {
                        2 // resize-bottom
                    } else {
                        0 // move
                    };
                    ui.ctx().memory_mut(|mem| {
                        mem.data.insert_temp(drag_id, (entry.clone(), entry_start_minutes, entry_end_minutes, current_time, col_x, drag_mode));
                    });
                }

                // Show appropriate cursor when hovering over entry (not during drag)
                if pointer_over_entry && !in_drag_mode {
                    if near_top_edge || near_bottom_edge {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    } else {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                    }
                }
            }
        }

        // Handle grabbed entry (click, drag, or resize)
        if let Some((entry, original_start_minutes, original_end_minutes, press_time, original_col_x, drag_mode)) = grabbed_state {
            let primary_down = ui.ctx().input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let primary_released = ui.ctx().input(|i| i.pointer.button_released(egui::PointerButton::Primary));
            let right_clicked = ui.ctx().input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary));
            let esc_pressed = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));

            let held_duration = current_time - press_time;
            let is_long_press = held_duration > long_press_threshold;

            // Get current pointer position
            let current_pos = ui.ctx().input(|i| i.pointer.latest_pos()).unwrap_or(egui::Pos2::ZERO);

            // Calculate snapped time at current position (5-minute intervals)
            let relative_y = (current_pos.y - grid_rect.min.y).max(0.0);
            let hover_minutes = start_minutes + (relative_y / pixels_per_minute) as i32;
            let snapped_minutes = ((hover_minutes + 2) / 5) * 5; // Round to nearest 5 min
            let snapped_minutes = snapped_minutes.max(start_minutes).min(end_minutes - 5);

            // Calculate new values based on drag mode
            let min_duration_minutes = 15; // Minimum 15-minute duration
            let (new_start_minutes, new_end_minutes) = match drag_mode {
                1 => {
                    // Resize-top: change start time, keep end fixed
                    let clamped_start = snapped_minutes.min(original_end_minutes - min_duration_minutes);
                    (clamped_start, original_end_minutes)
                }
                2 => {
                    // Resize-bottom: keep start fixed, change end time
                    let clamped_end = snapped_minutes.max(original_start_minutes + min_duration_minutes);
                    (original_start_minutes, clamped_end)
                }
                _ => {
                    // Move: shift both start and end by same amount
                    let duration = original_end_minutes - original_start_minutes;
                    (snapped_minutes, snapped_minutes + duration)
                }
            };

            let new_hour = new_start_minutes / 60;
            let new_minute = new_start_minutes % 60;
            let new_start_time = format!("{:02}:{:02}", new_hour, new_minute);
            let new_duration_seconds = ((new_end_minutes - new_start_minutes) * 60) as i64;

            // Right-click or Esc cancels drag
            if right_clicked || esc_pressed {
                ui.ctx().memory_mut(|mem| {
                    mem.data.remove::<DragState>(drag_id);
                });
            }
            // Primary released
            else if primary_released {
                // Clear state
                ui.ctx().memory_mut(|mem| {
                    mem.data.remove::<DragState>(drag_id);
                });

                if is_long_press {
                    match drag_mode {
                        1 | 2 => {
                            // Resize complete
                            if new_start_minutes != original_start_minutes || new_end_minutes != original_end_minutes {
                                result.drag_resize = Some((entry, new_start_time, new_duration_seconds));
                            }
                        }
                        _ => {
                            // Move complete
                            if new_start_minutes != original_start_minutes {
                                result.drag_move = Some((entry, new_start_time));
                            }
                        }
                    }
                } else {
                    // Quick click = open edit dialog
                    result.edit_entry = Some(entry);
                }
            }
            // Still holding - render drag preview if past threshold
            else if primary_down && is_long_press {
                // Set appropriate cursor
                match drag_mode {
                    1 | 2 => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical),
                    _ => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
                }

                // Calculate ghost rect at new position
                let ghost_y = grid_rect.min.y + (new_start_minutes - start_minutes) as f32 * pixels_per_minute;
                let ghost_height = (new_end_minutes - new_start_minutes) as f32 * pixels_per_minute;

                let block_margin = 2.0;
                let ghost_rect = egui::Rect::from_min_size(
                    egui::pos2(original_col_x + block_margin, ghost_y),
                    egui::vec2(day_width - block_margin * 2.0, (ghost_height - 2.0).max(20.0))
                );

                // Render the entry at the new position (as ghost)
                // For bottom-edge resize, show duration; otherwise show start time
                let display_text = if drag_mode == 2 {
                    format_duration_with_format(new_duration_seconds, time_format)
                } else {
                    new_start_time.clone()
                };
                render_schedule_entry_ghost(ui, ghost_rect, &entry, time_format, &display_text);
            }
        }

        // Second pass: handle column interactions (only for empty space)
        // Check if we're currently dragging an existing entry (from memory)
        let is_dragging_entry: bool = ui.ctx().memory(|mem| {
            mem.data.get_temp::<DragState>(drag_id).is_some()
        }) && in_drag_mode;

        for (day_idx, day) in days.iter().enumerate() {
            let col_x = grid_rect.min.x + hour_label_width + day_idx as f32 * day_width;

            // Check if pointer is over any entry in this column
            let pointer_pos = ui.ctx().pointer_hover_pos();
            let over_entry = pointer_pos.map(|pos| {
                all_entry_rects.iter().any(|r| r.contains(pos))
            }).unwrap_or(false);

            // Handle interactions on empty space
            let col_rect = egui::Rect::from_min_size(
                egui::pos2(col_x, grid_rect.min.y),
                egui::vec2(day_width, total_grid_height)
            );

            let col_response = ui.interact(col_rect, ui.id().with(("day_col", day_idx)), egui::Sense::click_and_drag());

            // Track hover position for ghost preview (only if not over an entry AND not dragging an existing entry)
            if col_response.hovered() && !over_entry && !is_dragging_entry {
                if let Some(pos) = ui.ctx().pointer_hover_pos() {
                    if pos.y >= grid_rect.min.y && pos.y <= grid_rect.max.y {
                        let relative_y = pos.y - grid_rect.min.y;
                        let hover_minutes = start_minutes + (relative_y / pixels_per_minute) as i32;
                        let hour = hover_minutes / 60;
                        let minute = hover_minutes % 60;
                        // Snap to 15-minute intervals
                        let snapped_minute = (minute / 15) * 15;
                        let ghost_time = format!("{:02}:{:02}", hour, snapped_minute);

                        // Check if ghost would overlap existing entries (1 hour = 60 mins)
                        let day_entries = week_data.entries_for_day(*day);
                        if !check_time_overlap(&day_entries, &ghost_time, 60) {
                            result.ghost_position = Some((*day, ghost_time.clone()));

                            // Render the ghost preview
                            let ghost_start_minutes = hour * 60 + snapped_minute;
                            let ghost_y = grid_rect.min.y + (ghost_start_minutes - start_minutes) as f32 * pixels_per_minute;
                            let ghost_height = 60.0 * pixels_per_minute; // 1 hour

                            let ghost_rect = egui::Rect::from_min_size(
                                egui::pos2(col_x + 2.0, ghost_y),
                                egui::vec2(day_width - 4.0, ghost_height)
                            );

                            // Draw translucent ghost block
                            let ghost_color = Color32::from_rgba_unmultiplied(0x61, 0xAF, 0xEF, 60);
                            let ghost_border = Color32::from_rgba_unmultiplied(0x61, 0xAF, 0xEF, 120);
                            ui.painter().rect(ghost_rect, 4.0, ghost_color, egui::Stroke::new(1.0, ghost_border));

                            // Ghost label
                            let ghost_label = format!("{} + 1h", ghost_time);
                            ui.painter().text(
                                ghost_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                ghost_label,
                                egui::FontId::proportional(13.0),
                                Color32::from_rgba_unmultiplied(255, 255, 255, 150),
                            );
                        }
                    }
                }
            }

            // Single-click on ghost creates entry (only if not over an existing entry)
            if col_response.clicked() && !over_entry {
                if let Some((_, ref time)) = result.ghost_position {
                    if *day == result.ghost_position.as_ref().unwrap().0 {
                        result.ghost_clicked = true;
                        result.add_at = Some((*day, time.clone()));
                    }
                }
            }

            // Double-click on empty space creates entry (not over existing entry)
            if col_response.double_clicked() && !over_entry {
                if let Some(pos) = col_response.interact_pointer_pos() {
                    let relative_y = pos.y - grid_rect.min.y;
                    let clicked_minutes = start_minutes + (relative_y / pixels_per_minute) as i32;
                    let hour = clicked_minutes / 60;
                    let minute = clicked_minutes % 60;
                    // Snap to 15-minute intervals
                    let snapped_minute = (minute / 15) * 15;
                    let start_time = format!("{:02}:{:02}", hour, snapped_minute);
                    result.add_at = Some((*day, start_time));
                }
            }
        }
    }); // end ScrollArea

    result
}

/// Parse "HH:MM" to minutes since midnight
fn parse_time_to_minutes(time: &str) -> i32 {
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() >= 2 {
        if let (Ok(h), Ok(m)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
            return h * 60 + m;
        }
    }
    0
}

/// Check if a time slot overlaps with any existing entries
fn check_time_overlap(
    entries: &[&crate::api::TimeEntry],
    start_time: &str,
    duration_mins: i32,
) -> bool {
    let new_start = parse_time_to_minutes(start_time);
    let new_end = new_start + duration_mins;

    for entry in entries {
        let entry_start = parse_time_to_minutes(&entry.start_time);
        let entry_end = entry_start + (entry.seconds / 60) as i32;

        // Check for overlap: new block starts before entry ends AND new block ends after entry starts
        if new_start < entry_end && new_end > entry_start {
            return true;
        }
    }
    false
}

/// Paint a single entry block in the schedule view (no interaction - that's handled by caller)
fn render_schedule_entry_paint(
    ui: &mut Ui,
    rect: egui::Rect,
    entry: &crate::api::TimeEntry,
    time_format: TimeFormat,
) {
    let painter = ui.painter();

    // Accent color based on ticket type
    let accent_color = if entry.issue_key.starts_with("TIM-") {
        let summary_upper = entry.issue_summary.to_uppercase();
        if summary_upper.contains("MEETING") {
            Color32::from_rgb(0xe8, 0x28, 0x71)  // Pink/magenta
        } else if summary_upper.contains("SUPPORT") {
            Color32::from_rgb(0xec, 0x71, 0x1b)  // Orange
        } else if summary_upper.contains("ADMIN") {
            Color32::from_rgb(0xe5, 0xaa, 0x00)  // Yellow/gold
        } else {
            Color32::from_rgb(0x13, 0x98, 0xf4)  // Blue
        }
    } else {
        Color32::from_rgb(0x13, 0x98, 0xf4)  // Blue for regular tickets
    };

    // Draw block background
    let block_bg = Color32::from_rgb(0x1c, 0x1c, 0x1a);
    let corner_radius = 4.0;

    painter.rect(
        rect,
        corner_radius,
        block_bg,
        egui::Stroke::new(1.0, accent_color),
    );

    // Left accent stripe
    let accent_width = 3.0;
    let accent_rect = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(accent_width, rect.height())
    );
    painter.rect(
        accent_rect,
        egui::Rounding {
            nw: corner_radius,
            sw: corner_radius,
            ne: 0.0,
            se: 0.0,
        },
        accent_color,
        egui::Stroke::NONE,
    );

    // Text content
    let text_left = rect.min.x + accent_width + 4.0;
    let issue_key_color = Color32::from_rgb(200, 200, 192);  // Bright gray for issue keys
    let font_size = 13.0;
    let key_font = egui::FontId::proportional(font_size);

    // Get issue type icon info using shared function
    let icon_style = issue_type_icon(&entry.issue_type);

    // First line: Icon + Issue key + Duration (all on same line)
    if rect.height() > 20.0 {
        let line_y = rect.min.y + 12.0;
        let mut x = text_left;

        // Icon - all types now use colored square background (consistent with list view)
        let icon_size = font_size;
        let (icon_char, bg_color, icon_color) = match icon_style {
            IssueTypeIcon::OnSquare(icon, bg) => (icon, bg, Color32::WHITE),
            IssueTypeIcon::OnSquareBlack(icon, bg) => (icon, bg, Color32::BLACK),
        };

        // Draw colored square background
        let square_size = icon_size + 2.0;
        let square_rect = egui::Rect::from_center_size(
            egui::pos2(x + square_size / 2.0, line_y),
            egui::vec2(square_size, square_size)
        );
        painter.rect_filled(square_rect, 2.0, bg_color);
        // Draw filled icon (use phosphor-fill font family)
        painter.text(
            square_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon_char,
            egui::FontId::new(icon_size - 2.0, super::theme::phosphor_fill_family()),
            icon_color,
        );
        x += square_size + 3.0;

        // Issue key - bright gray
        let key_galley = painter.layout_no_wrap(entry.issue_key.clone(), key_font.clone(), issue_key_color);
        painter.galley(egui::pos2(x, line_y - key_galley.size().y / 2.0), key_galley.clone(), Color32::WHITE);
        x += key_galley.size().x + 6.0;

        // Duration - bright white bold for times to stand out
        let duration_text = crate::api::format_duration_with_format(entry.seconds, time_format);
        let dur_font = egui::FontId::new(key_font.size, super::theme::bold_family());
        let dur_galley = painter.layout_no_wrap(duration_text, dur_font, Color32::WHITE);
        // Only show duration if it fits (leave room for dots menu)
        let available_width = rect.max.x - x - 24.0;
        if dur_galley.size().x < available_width {
            painter.galley(egui::pos2(x, line_y - dur_galley.size().y / 2.0), dur_galley, Color32::WHITE);
        }
    }
}

/// Render an entry as a ghost (semi-transparent) during drag
/// display_text is either the new start time or the new duration depending on drag mode
fn render_schedule_entry_ghost(
    ui: &mut Ui,
    rect: egui::Rect,
    entry: &crate::api::TimeEntry,
    _time_format: TimeFormat,
    display_text: &str,
) {
    let painter = ui.painter();
    let alpha = 180; // Semi-transparent

    // Accent color based on ticket type (same logic as paint version)
    let accent_color = if entry.issue_key.starts_with("TIM-") {
        let summary_upper = entry.issue_summary.to_uppercase();
        if summary_upper.contains("MEETING") {
            Color32::from_rgba_unmultiplied(0xe8, 0x28, 0x71, alpha)
        } else if summary_upper.contains("SUPPORT") {
            Color32::from_rgba_unmultiplied(0xec, 0x71, 0x1b, alpha)
        } else if summary_upper.contains("ADMIN") {
            Color32::from_rgba_unmultiplied(0xe5, 0xaa, 0x00, alpha)
        } else {
            Color32::from_rgba_unmultiplied(0x13, 0x98, 0xf4, alpha)
        }
    } else {
        Color32::from_rgba_unmultiplied(0x13, 0x98, 0xf4, alpha)
    };

    // Draw block background
    let block_bg = Color32::from_rgba_unmultiplied(0x1c, 0x1c, 0x1a, alpha);
    let corner_radius = 4.0;

    painter.rect(
        rect,
        corner_radius,
        block_bg,
        egui::Stroke::new(2.0, accent_color), // Thicker border for ghost
    );

    // Left accent stripe
    let accent_width = 3.0;
    let accent_rect = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(accent_width, rect.height())
    );
    painter.rect(
        accent_rect,
        egui::Rounding {
            nw: corner_radius,
            sw: corner_radius,
            ne: 0.0,
            se: 0.0,
        },
        accent_color,
        egui::Stroke::NONE,
    );

    // Text content - always show, centered vertically
    let text_left = rect.min.x + accent_width + 4.0;
    let text_color = Color32::from_rgba_unmultiplied(200, 200, 192, alpha);
    let font_size = 13.0;
    let key_font = egui::FontId::proportional(font_size);

    // Center text vertically in the rect
    let line_y = rect.center().y;
    let mut x = text_left;

    // Display text (time or duration) - highlighted in bright blue
    let display_galley = painter.layout_no_wrap(
        display_text.to_string(),
        egui::FontId::new(font_size, super::theme::bold_family()),
        Color32::from_rgba_unmultiplied(0x61, 0xAF, 0xEF, 255), // Bright blue
    );
    painter.galley(egui::pos2(x, line_y - display_galley.size().y / 2.0), display_galley.clone(), Color32::WHITE);
    x += display_galley.size().x + 6.0;

    // Issue key - only if it fits
    let key_galley = painter.layout_no_wrap(entry.issue_key.clone(), key_font, text_color);
    let available_width = rect.max.x - x - 4.0;
    if key_galley.size().x < available_width {
        painter.galley(egui::pos2(x, line_y - key_galley.size().y / 2.0), key_galley, Color32::WHITE);
    }
}
