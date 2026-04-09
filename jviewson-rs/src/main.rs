#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, Align, CentralPanel, CollapsingHeader, Color32, Context, FontId, RichText, ScrollArea,
    Sense, TextEdit, TextStyle, TopBottomPanel, Ui, text::LayoutJob, text::TextFormat, vec2,
};
use serde_json::Value;

const SEARCH_HIT_COLOR: Color32 = Color32::from_rgb(255, 215, 80);
const SEARCH_ACTIVE_COLOR: Color32 = Color32::from_rgb(255, 140, 70);
const TEXT_BASE_COLOR: Color32 = Color32::from_rgb(245, 245, 245);
const LINE_NO_COLOR: Color32 = Color32::from_rgb(170, 170, 170);
const MATCH_TEXT_COLOR: Color32 = Color32::from_rgb(30, 20, 10);
const ACTIVE_MATCH_TEXT_COLOR: Color32 = Color32::from_rgb(15, 8, 4);
const MINIMAP_WIDTH: f32 = 14.0;
const PREVIEW_CHAR_LIMIT: usize = 200_000;

fn main() -> eframe::Result<()> {
    let startup_file = startup_file_from_args();
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "JviewSON RS",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(JviewsonApp::with_startup_file(
                startup_file.clone(),
            )))
        }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewTab {
    Tree,
    Text,
}

#[derive(Clone, Copy)]
struct TextMatch {
    line: usize,
    column: usize,
}

struct JviewsonApp {
    tab: ViewTab,
    search_query: String,
    auto_reload: bool,
    current_file: Option<PathBuf>,
    json_value: Option<Value>,
    pretty_json: String,
    pretty_lines: Vec<String>,
    parse_error: Option<String>,
    status_message: String,
    last_modified_time: Option<SystemTime>,
    last_reload_check: Instant,
    tree_match_paths: Vec<String>,
    tree_match_set: HashSet<String>,
    tree_open_paths: HashSet<String>,
    active_tree_match: Option<usize>,
    active_text_match: Option<usize>,
    text_matches: Vec<TextMatch>,
    text_match_lines: HashSet<usize>,
    scroll_to_tree_path: Option<String>,
    scroll_to_text_line: Option<usize>,
    selected_tree_path: Option<String>,
    selected_tree_preview: String,
}

impl Default for JviewsonApp {
    fn default() -> Self {
        Self {
            tab: ViewTab::Tree,
            search_query: String::new(),
            auto_reload: false,
            current_file: None,
            json_value: None,
            pretty_json: String::new(),
            pretty_lines: Vec::new(),
            parse_error: None,
            status_message: "Open a JSON file to start.".to_owned(),
            last_modified_time: None,
            last_reload_check: Instant::now(),
            tree_match_paths: Vec::new(),
            tree_match_set: HashSet::new(),
            tree_open_paths: HashSet::new(),
            active_tree_match: None,
            active_text_match: None,
            text_matches: Vec::new(),
            text_match_lines: HashSet::new(),
            scroll_to_tree_path: None,
            scroll_to_text_line: None,
            selected_tree_path: None,
            selected_tree_preview: String::new(),
        }
    }
}

impl eframe::App for JviewsonApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.handle_file_drop(ctx);
        self.handle_auto_reload();

        if ctx.input(|i| i.key_pressed(egui::Key::F3)) {
            let backwards = ctx.input(|i| i.modifiers.shift);
            self.advance_match(backwards);
        }

        TopBottomPanel::top("toolbar").show(ctx, |ui| {
            self.toolbar_ui(ui);
        });

        CentralPanel::default().show(ctx, |ui| {
            self.content_ui(ui);
        });

        if self.auto_reload {
            ctx.request_repaint_after(Duration::from_millis(750));
        }
    }
}

impl JviewsonApp {
    fn with_startup_file(startup_file: Option<PathBuf>) -> Self {
        let mut app = Self::default();
        if let Some(path) = startup_file {
            app.load_json_from_path(path, false);
        }
        app
    }

    fn toolbar_ui(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open JSON").clicked() {
                self.pick_file();
            }

            let can_reload = self.current_file.is_some();
            if ui
                .add_enabled(can_reload, egui::Button::new("Reload"))
                .clicked()
            {
                self.reload_current_file(true);
            }

            ui.separator();
            ui.checkbox(&mut self.auto_reload, "Auto reload");

            ui.separator();
            ui.selectable_value(&mut self.tab, ViewTab::Tree, "Tree");
            ui.selectable_value(&mut self.tab, ViewTab::Text, "Text");

            ui.separator();
            ui.label("Search:");
            let search_response =
                ui.add(TextEdit::singleline(&mut self.search_query).desired_width(260.0));

            if search_response.changed() {
                self.rebuild_search_index();
            }

            if search_response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let backwards = ui.input(|i| i.modifiers.shift);
                self.advance_match(backwards);
            }

            if let Some(summary) = self.search_summary() {
                ui.label(summary);
            }
        });

        if let Some(path) = &self.current_file {
            ui.label(format!("File: {}", path.display()));
        }
        ui.small(self.status_message.as_str());
    }

    fn content_ui(&mut self, ui: &mut Ui) {
        if let Some(err) = &self.parse_error {
            ui.colored_label(Color32::from_rgb(255, 110, 110), err);
            return;
        }

        match self.tab {
            ViewTab::Tree => self.render_tree_mode(ui),
            ViewTab::Text => self.render_text_mode(ui),
        }
    }

    fn render_tree_mode(&mut self, ui: &mut Ui) {
        let query_lc = normalized_query(self.search_query.as_str());
        let active_tree_path = self
            .active_tree_match
            .and_then(|idx| self.tree_match_paths.get(idx))
            .map(String::as_str);
        let selected_tree_path = self.selected_tree_path.as_deref();
        let scroll_to_tree_path = self.scroll_to_tree_path.as_deref();

        let mut clicked_tree: Option<(String, String)> = None;
        let mut did_scroll = false;

        ui.columns(2, |columns| {
            ScrollArea::vertical()
                .id_salt("tree_scroll")
                .show(&mut columns[0], |ui| {
                    if let Some(json) = &self.json_value {
                        render_tree_value(
                            ui,
                            "root",
                            "$",
                            json,
                            query_lc.as_deref(),
                            &self.tree_match_set,
                            &self.tree_open_paths,
                            active_tree_path,
                            selected_tree_path,
                            scroll_to_tree_path,
                            &mut did_scroll,
                            &mut clicked_tree,
                        );
                    } else {
                        ui.label("No JSON loaded.");
                    }
                });

            columns[1].heading("Selection");
            columns[1].separator();

            if let Some(path) = &self.selected_tree_path {
                columns[1].small(format!("Path: {path}"));
                columns[1].add_space(4.0);
                ScrollArea::both().show(&mut columns[1], |ui| {
                    ui.add(
                        TextEdit::multiline(&mut self.selected_tree_preview)
                            .font(TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
            } else {
                columns[1].label("Click a tree item to inspect its value.");
            }
        });

        if did_scroll {
            self.scroll_to_tree_path = None;
        }

        if let Some((path, preview)) = clicked_tree {
            self.selected_tree_path = Some(path);
            self.selected_tree_preview = preview;
        }
    }

    fn render_text_mode(&mut self, ui: &mut Ui) {
        if self.pretty_lines.is_empty() {
            ui.label("No JSON loaded.");
            return;
        }

        let query_lc = normalized_query(self.search_query.as_str());
        let active_match = self
            .active_text_match
            .and_then(|idx| self.text_matches.get(idx))
            .copied();
        let row_height = ui.text_style_height(&TextStyle::Monospace);
        let total_lines = self.pretty_lines.len();
        let active_line = active_match.map(|m| m.line);
        let target_line = self.scroll_to_text_line;

        let lines = &self.pretty_lines;
        let match_lines = &self.text_match_lines;
        let mut did_scroll = false;
        let mut minimap_line_pick: Option<usize> = None;

        let panel_size = ui.available_size();
        ui.allocate_ui_with_layout(panel_size, egui::Layout::left_to_right(Align::Min), |ui| {
            let content_width = (ui.available_width() - MINIMAP_WIDTH - 8.0).max(120.0);

            ui.allocate_ui_with_layout(
                vec2(content_width, ui.available_height()),
                egui::Layout::top_down(Align::Min),
                |ui| {
                    ui.set_min_width(content_width);
                    ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .id_salt("text_scroll")
                        .show_rows(ui, row_height, total_lines, |ui, row_range| {
                            for row in row_range {
                                let line = &lines[row];
                                let line_job = build_text_line_job(
                                    ui,
                                    row,
                                    line,
                                    query_lc.as_deref(),
                                    active_match,
                                    match_lines,
                                );
                                let response = ui.label(line_job);

                                if !did_scroll
                                    && let Some(target) = target_line
                                    && row == target
                                {
                                    response.scroll_to_me(Some(Align::Center));
                                    did_scroll = true;
                                }
                            }
                        });
                },
            );

            ui.add_space(4.0);
            minimap_line_pick = render_minimap(ui, total_lines, active_line, match_lines);
        });

        if did_scroll {
            self.scroll_to_text_line = None;
        }

        if let Some(line) = minimap_line_pick {
            self.scroll_to_text_line = Some(line);
            self.activate_nearest_text_match(line);
        }
    }

    fn pick_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
        {
            self.load_json_from_path(path, false);
        }
    }

    fn reload_current_file(&mut self, manual: bool) {
        if let Some(path) = self.current_file.clone() {
            self.load_json_from_path(path, manual);
        }
    }

    fn load_json_from_path(&mut self, path: PathBuf, manual_reload: bool) {
        match read_and_parse_json(path.as_path()) {
            Ok((json_value, pretty_text, modified_time)) => {
                self.json_value = Some(json_value.clone());
                self.pretty_json = pretty_text;
                self.pretty_lines = split_lines(self.pretty_json.as_str());
                self.parse_error = None;
                self.last_modified_time = modified_time;
                self.status_message = if manual_reload {
                    format!("File reloaded. {} lines.", self.pretty_lines.len())
                } else {
                    format!("File loaded. {} lines.", self.pretty_lines.len())
                };
                self.current_file = Some(path);

                self.selected_tree_path = Some("$".to_owned());
                self.selected_tree_preview = preview_for_value(&json_value);

                self.rebuild_search_index();
            }
            Err(error_message) => {
                self.parse_error = Some(error_message.clone());
                self.status_message = "Load failed.".to_owned();
                self.json_value = None;
                self.pretty_json.clear();
                self.pretty_lines.clear();
                self.selected_tree_path = None;
                self.selected_tree_preview.clear();
                self.clear_search_index();
            }
        }
    }

    fn clear_search_index(&mut self) {
        self.tree_match_paths.clear();
        self.tree_match_set.clear();
        self.tree_open_paths.clear();
        self.active_tree_match = None;
        self.active_text_match = None;
        self.text_matches.clear();
        self.text_match_lines.clear();
        self.scroll_to_tree_path = None;
        self.scroll_to_text_line = None;
    }

    fn rebuild_search_index(&mut self) {
        self.clear_search_index();

        let Some(query_lc) = normalized_query(self.search_query.as_str()) else {
            return;
        };

        if let Some(json) = &self.json_value {
            collect_tree_matches(
                "root",
                "$",
                json,
                query_lc.as_str(),
                &mut self.tree_match_paths,
                &mut self.tree_open_paths,
            );
            self.tree_match_set = self.tree_match_paths.iter().cloned().collect();
        }

        self.text_matches = collect_text_matches(&self.pretty_lines, query_lc.as_str());
        self.text_match_lines = self.text_matches.iter().map(|m| m.line).collect();

        if !self.tree_match_paths.is_empty() {
            self.active_tree_match = Some(0);
            self.scroll_to_tree_path = Some(self.tree_match_paths[0].clone());
        }

        if !self.text_matches.is_empty() {
            self.active_text_match = Some(0);
            self.scroll_to_text_line = Some(self.text_matches[0].line);
        }

        if self.tree_match_paths.is_empty() && self.text_matches.is_empty() {
            self.status_message = format!("No matches for '{}'.", self.search_query.trim());
        }
    }

    fn advance_match(&mut self, backwards: bool) {
        match self.tab {
            ViewTab::Tree => {
                if self.tree_match_paths.is_empty() {
                    self.status_message =
                        format!("No tree matches for '{}'.", self.search_query.trim());
                    return;
                }

                let total = self.tree_match_paths.len();
                let current = self.active_tree_match.unwrap_or(0);
                let next = next_index(current, total, backwards);
                self.active_tree_match = Some(next);
                self.scroll_to_tree_path = Some(self.tree_match_paths[next].clone());
                self.status_message = format!("Tree match {}/{}", next + 1, total);
            }
            ViewTab::Text => {
                if self.text_matches.is_empty() {
                    self.status_message =
                        format!("No text matches for '{}'.", self.search_query.trim());
                    return;
                }

                let total = self.text_matches.len();
                let current = self.active_text_match.unwrap_or(0);
                let next = next_index(current, total, backwards);
                self.active_text_match = Some(next);

                let matched = self.text_matches[next];
                self.scroll_to_text_line = Some(matched.line);
                self.status_message = format!(
                    "Text match {}/{} at line {}, column {}",
                    next + 1,
                    total,
                    matched.line + 1,
                    matched.column + 1
                );
            }
        }
    }

    fn activate_nearest_text_match(&mut self, line: usize) {
        if self.text_matches.is_empty() {
            return;
        }

        let mut best_idx = 0usize;
        let mut best_dist = usize::MAX;

        for (idx, m) in self.text_matches.iter().enumerate() {
            let dist = m.line.abs_diff(line);
            if dist < best_dist {
                best_idx = idx;
                best_dist = dist;
            }
        }

        self.active_text_match = Some(best_idx);
    }

    fn search_summary(&self) -> Option<String> {
        if normalized_query(self.search_query.as_str()).is_none() {
            return None;
        }

        let tree_part = if let Some(active) = self.active_tree_match {
            format!("Tree {}/{}", active + 1, self.tree_match_paths.len())
        } else {
            format!("Tree 0/{}", self.tree_match_paths.len())
        };

        let text_part = if let Some(active) = self.active_text_match {
            format!("Text {}/{}", active + 1, self.text_matches.len())
        } else {
            format!("Text 0/{}", self.text_matches.len())
        };

        Some(format!(
            "{tree_part} | {text_part} (Enter/F3 next, Shift+Enter/F3 prev)"
        ))
    }

    fn handle_auto_reload(&mut self) {
        if !self.auto_reload {
            return;
        }

        if self.last_reload_check.elapsed() < Duration::from_millis(750) {
            return;
        }
        self.last_reload_check = Instant::now();

        let Some(path) = self.current_file.clone() else {
            return;
        };

        let Ok(metadata) = fs::metadata(path.as_path()) else {
            self.status_message = "Auto reload: file metadata unavailable.".to_owned();
            return;
        };

        let Ok(current_modified) = metadata.modified() else {
            return;
        };

        let should_reload = self
            .last_modified_time
            .is_none_or(|previous| current_modified > previous);

        if should_reload {
            self.load_json_from_path(path, true);
            self.status_message = "File auto-reloaded after modification.".to_owned();
        }
    }

    fn handle_file_drop(&mut self, ctx: &Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }

        for file in dropped {
            if let Some(path) = file.path {
                self.load_json_from_path(path, false);
                return;
            }
        }
    }
}

fn startup_file_from_args() -> Option<PathBuf> {
    for arg in env::args_os().skip(1) {
        let candidate = PathBuf::from(&arg);
        if candidate.is_file() {
            return Some(candidate);
        }

        if let Some(raw_arg) = arg.to_str()
            && let Some(raw_path) = raw_arg.strip_prefix("file://")
        {
            let file_path = if cfg!(target_os = "windows") {
                raw_path.trim_start_matches('/')
            } else {
                raw_path
            };
            let file_path = file_path.replace("%20", " ");
            let candidate = PathBuf::from(file_path);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn render_tree_value(
    ui: &mut Ui,
    key: &str,
    path: &str,
    value: &Value,
    query_lc: Option<&str>,
    tree_match_set: &HashSet<String>,
    tree_open_paths: &HashSet<String>,
    active_tree_path: Option<&str>,
    selected_tree_path: Option<&str>,
    scroll_to_tree_path: Option<&str>,
    did_scroll: &mut bool,
    clicked_tree: &mut Option<(String, String)>,
) {
    let is_match = tree_match_set.contains(path);
    let is_active = active_tree_path == Some(path);
    let is_selected = selected_tree_path == Some(path);

    let base_color = if is_active {
        Some(SEARCH_ACTIVE_COLOR)
    } else if is_match {
        Some(SEARCH_HIT_COLOR)
    } else {
        None
    };

    match value {
        Value::Object(map) => {
            let mut header_text = RichText::new(format!("{key}: {{}}"));
            if let Some(color) = base_color {
                header_text = header_text.color(color);
            }
            if is_selected {
                header_text = header_text.strong();
            }

            let id = if query_lc.is_some() {
                format!("tree-search-{path}")
            } else {
                format!("tree-{path}")
            };

            let response = CollapsingHeader::new(header_text)
                .id_salt(id)
                .default_open(path == "$" || tree_open_paths.contains(path))
                .show(ui, |ui| {
                    for (child_key, child_value) in map {
                        let child_path = format!("{path}.{child_key}");
                        render_tree_value(
                            ui,
                            child_key,
                            &child_path,
                            child_value,
                            query_lc,
                            tree_match_set,
                            tree_open_paths,
                            active_tree_path,
                            selected_tree_path,
                            scroll_to_tree_path,
                            did_scroll,
                            clicked_tree,
                        );
                    }
                });

            if response.header_response.clicked() {
                *clicked_tree = Some((path.to_owned(), preview_for_value(value)));
            }

            if !*did_scroll && scroll_to_tree_path == Some(path) {
                response.header_response.scroll_to_me(Some(Align::Center));
                *did_scroll = true;
            }
        }
        Value::Array(items) => {
            let mut header_text = RichText::new(format!("{key}: [{}]", items.len()));
            if let Some(color) = base_color {
                header_text = header_text.color(color);
            }
            if is_selected {
                header_text = header_text.strong();
            }

            let id = if query_lc.is_some() {
                format!("tree-search-{path}")
            } else {
                format!("tree-{path}")
            };

            let response = CollapsingHeader::new(header_text)
                .id_salt(id)
                .default_open(path == "$" || tree_open_paths.contains(path))
                .show(ui, |ui| {
                    for (idx, child) in items.iter().enumerate() {
                        let child_key = format!("[{idx}]");
                        let child_path = format!("{path}[{idx}]");
                        render_tree_value(
                            ui,
                            &child_key,
                            &child_path,
                            child,
                            query_lc,
                            tree_match_set,
                            tree_open_paths,
                            active_tree_path,
                            selected_tree_path,
                            scroll_to_tree_path,
                            did_scroll,
                            clicked_tree,
                        );
                    }
                });

            if response.header_response.clicked() {
                *clicked_tree = Some((path.to_owned(), preview_for_value(value)));
            }

            if !*did_scroll && scroll_to_tree_path == Some(path) {
                response.header_response.scroll_to_me(Some(Align::Center));
                *did_scroll = true;
            }
        }
        _ => {
            let mut line = RichText::new(format!("{key}: {}", value_as_string(value)));
            if let Some(color) = base_color {
                line = line.color(color);
            }
            if is_selected {
                line = line.strong();
            }

            let response = ui.selectable_label(is_selected || is_active, line);
            if response.clicked() {
                *clicked_tree = Some((path.to_owned(), preview_for_value(value)));
            }

            if !*did_scroll && scroll_to_tree_path == Some(path) {
                response.scroll_to_me(Some(Align::Center));
                *did_scroll = true;
            }
        }
    }
}

fn render_minimap(
    ui: &mut Ui,
    total_lines: usize,
    active_line: Option<usize>,
    text_match_lines: &HashSet<usize>,
) -> Option<usize> {
    let height = ui.available_height().max(120.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(MINIMAP_WIDTH, height), Sense::click_and_drag());

    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, Color32::from_gray(28));

    if total_lines == 0 {
        return None;
    }

    let denom = (total_lines.saturating_sub(1)).max(1) as f32;
    for line in text_match_lines {
        let y = rect.top() + (*line as f32 / denom) * rect.height();
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            (1.0, SEARCH_HIT_COLOR),
        );
    }

    if let Some(line) = active_line {
        let y = rect.top() + (line as f32 / denom) * rect.height();
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            (2.0, SEARCH_ACTIVE_COLOR),
        );
    }

    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let ratio = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        let line = (ratio * (total_lines.saturating_sub(1) as f32)).round() as usize;
        return Some(line);
    }

    None
}

fn build_text_line_job(
    ui: &Ui,
    line_idx: usize,
    line: &str,
    query_lc: Option<&str>,
    active_match: Option<TextMatch>,
    match_lines: &HashSet<usize>,
) -> LayoutJob {
    let mono_font = ui
        .style()
        .text_styles
        .get(&TextStyle::Monospace)
        .cloned()
        .unwrap_or_else(|| FontId::monospace(13.0));

    let line_no_fmt = TextFormat {
        font_id: mono_font.clone(),
        color: LINE_NO_COLOR,
        ..Default::default()
    };
    let base_fmt = TextFormat {
        font_id: mono_font.clone(),
        color: TEXT_BASE_COLOR,
        ..Default::default()
    };
    let match_fmt = TextFormat {
        font_id: mono_font.clone(),
        color: MATCH_TEXT_COLOR,
        background: SEARCH_HIT_COLOR,
        ..Default::default()
    };
    let active_match_fmt = TextFormat {
        font_id: mono_font,
        color: ACTIVE_MATCH_TEXT_COLOR,
        background: SEARCH_ACTIVE_COLOR,
        ..Default::default()
    };

    let mut job = LayoutJob::default();
    let line_no = format!("{:>6} | ", line_idx + 1);
    job.append(line_no.as_str(), 0.0, line_no_fmt);

    let Some(query_lc) = query_lc else {
        job.append(line, 0.0, base_fmt);
        return job;
    };
    if query_lc.is_empty() {
        job.append(line, 0.0, base_fmt);
        return job;
    }

    if !match_lines.contains(&line_idx) {
        job.append(line, 0.0, base_fmt);
        return job;
    }

    let line_lc = line.to_ascii_lowercase();
    let mut start = 0usize;
    while let Some(found_at) = line_lc[start..].find(query_lc) {
        let match_start = start + found_at;
        let match_end = match_start + query_lc.len();

        if match_start > start {
            let chunk = &line[start..match_start];
            job.append(chunk, 0.0, base_fmt.clone());
        }

        let chunk = &line[match_start..match_end];
        let is_active = active_match
            .map(|m| m.line == line_idx && m.column == match_start)
            .unwrap_or(false);
        if is_active {
            job.append(chunk, 0.0, active_match_fmt.clone());
        } else {
            job.append(chunk, 0.0, match_fmt.clone());
        }

        start = match_end;
    }

    if start < line.len() {
        job.append(&line[start..], 0.0, base_fmt);
    }

    job
}

fn read_and_parse_json(path: &Path) -> Result<(Value, String, Option<SystemTime>), String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Could not read file '{}': {e}", path.display()))?;

    let parsed: Value = serde_json::from_str(content.as_str())
        .map_err(|e| format!("Invalid JSON in '{}': {e}", path.display()))?;

    let pretty_text = serde_json::to_string_pretty(&parsed)
        .map_err(|e| format!("Could not format JSON from '{}': {e}", path.display()))?;

    let modified = fs::metadata(path).ok().and_then(|m| m.modified().ok());
    Ok((parsed, pretty_text, modified))
}

fn split_lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

fn value_as_string(value: &Value) -> String {
    match value {
        Value::String(s) => format!("{s:?}"),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_owned(),
        Value::Array(_) | Value::Object(_) => "<nested>".to_owned(),
    }
}

fn normalized_query(query: &str) -> Option<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_ascii_lowercase())
    }
}

fn collect_tree_matches(
    key: &str,
    path: &str,
    value: &Value,
    query_lc: &str,
    matches: &mut Vec<String>,
    open_paths: &mut HashSet<String>,
) -> bool {
    let label = node_label(key, value).to_ascii_lowercase();
    let self_match = label.contains(query_lc);
    if self_match {
        matches.push(path.to_owned());
    }

    let mut subtree_match = self_match;

    match value {
        Value::Object(map) => {
            for (child_key, child_value) in map {
                let child_path = format!("{path}.{child_key}");
                if collect_tree_matches(
                    child_key,
                    &child_path,
                    child_value,
                    query_lc,
                    matches,
                    open_paths,
                ) {
                    subtree_match = true;
                }
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let child_key = format!("[{idx}]");
                let child_path = format!("{path}[{idx}]");
                if collect_tree_matches(
                    &child_key,
                    &child_path,
                    child,
                    query_lc,
                    matches,
                    open_paths,
                ) {
                    subtree_match = true;
                }
            }
        }
        _ => {}
    }

    if subtree_match {
        open_paths.insert(path.to_owned());
    }

    subtree_match
}

fn node_label(key: &str, value: &Value) -> String {
    match value {
        Value::Object(_) => format!("{key}: {{}}"),
        Value::Array(items) => format!("{key}: [{}]", items.len()),
        _ => format!("{key}: {}", value_as_string(value)),
    }
}

fn collect_text_matches(lines: &[String], query_lc: &str) -> Vec<TextMatch> {
    let mut matches = Vec::new();

    for (line_idx, line) in lines.iter().enumerate() {
        let line_lc = line.to_ascii_lowercase();
        let mut start = 0usize;

        while let Some(found_at) = line_lc[start..].find(query_lc) {
            let col = start + found_at;
            matches.push(TextMatch {
                line: line_idx,
                column: col,
            });
            start = col + 1;
        }
    }

    matches
}

fn next_index(current: usize, total: usize, backwards: bool) -> usize {
    if backwards {
        if current == 0 { total - 1 } else { current - 1 }
    } else {
        (current + 1) % total
    }
}

fn preview_for_value(value: &Value) -> String {
    let raw = match value {
        Value::Object(_) | Value::Array(_) => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
        _ => value_as_string(value),
    };

    truncate_for_preview(raw, PREVIEW_CHAR_LIMIT)
}

fn truncate_for_preview(text: String, max_chars: usize) -> String {
    let total_chars = text.chars().count();
    if total_chars <= max_chars {
        return text;
    }

    let truncated: String = text.chars().take(max_chars).collect();
    format!("{truncated}\n\n[Preview truncated: showing {max_chars} of {total_chars} characters]")
}
