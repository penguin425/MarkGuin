use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Color32, FontId, Key, KeyboardShortcut, Modifiers, RichText, TextEdit,
    text::{CCursor, CCursorRange},
};

use crate::{
    document::{DiskState, Document},
    markdown,
};

const SESSION_KEY: &str = "markguin.session.v1";
const BG: Color32 = Color32::from_rgb(16, 17, 20);
const CANVAS: Color32 = Color32::from_rgb(20, 21, 24);
const SURFACE: Color32 = Color32::from_rgb(25, 26, 30);
const SURFACE_RAISED: Color32 = Color32::from_rgb(31, 32, 37);
const SURFACE_HOVER: Color32 = Color32::from_rgb(39, 40, 46);
const BORDER: Color32 = Color32::from_rgb(48, 49, 56);
const TEXT: Color32 = Color32::from_rgb(239, 239, 242);
const MUTED: Color32 = Color32::from_rgb(148, 149, 158);
const FAINT: Color32 = Color32::from_rgb(98, 99, 108);
const ACCENT: Color32 = Color32::from_rgb(123, 97, 255);
const ACCENT_SOFT: Color32 = Color32::from_rgb(48, 40, 82);
const SUCCESS: Color32 = Color32::from_rgb(83, 196, 139);

#[derive(Clone, Copy)]
enum Icon {
    Sidebar,
    File,
    Folder,
    Save,
    Export,
    Search,
    Focus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ViewMode {
    Editor,
    Split,
    Preview,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Session {
    text: String,
    path: Option<PathBuf>,
    dirty: bool,
    view: ViewMode,
    show_outline: bool,
    focus_mode: bool,
    #[serde(default = "default_true")]
    sync_scroll: bool,
}

fn default_true() -> bool {
    true
}

impl Session {
    fn capture(app: &MarkGuin) -> Self {
        Self {
            text: app.document.text.clone(),
            path: app.document.path.clone(),
            dirty: app.document.dirty,
            view: app.view,
            show_outline: app.show_outline,
            focus_mode: app.focus_mode,
            sync_scroll: app.sync_scroll,
        }
    }

    fn into_document(self) -> (Document, ViewMode, bool, bool, bool) {
        (
            Document::recovered(self.text, self.path, self.dirty),
            self.view,
            self.show_outline,
            self.focus_mode,
            self.sync_scroll,
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Insert {
    Bold,
    Italic,
    Code,
    Link,
    Heading,
    Quote,
    List,
    Task,
    Rule,
}

#[derive(Clone, Copy)]
enum PendingAction {
    New,
    Open,
    Close,
}

pub struct MarkGuin {
    document: Document,
    view: ViewMode,
    show_outline: bool,
    show_find: bool,
    find_query: String,
    replace_query: String,
    find_focus_requested: bool,
    status: Option<String>,
    cursor_char: usize,
    selection: (usize, usize),
    requested_selection: Option<(usize, usize)>,
    focus_mode: bool,
    pending_action: Option<PendingAction>,
    show_table_dialog: bool,
    table_columns: usize,
    table_rows: usize,
    last_disk_check: Instant,
    disk_conflict: Option<DiskState>,
    ignore_external_change: bool,
    sync_scroll: bool,
    scroll_ratio: f32,
    editor_scroll_max: f32,
    preview_scroll_max: f32,
}

impl MarkGuin {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let command_line_document = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .and_then(|path| Document::open(path).ok());
        let restored = command_line_document
            .is_none()
            .then(|| {
                cc.storage
                    .and_then(|storage| eframe::get_value::<Session>(storage, SESSION_KEY))
            })
            .flatten();
        let (document, view, show_outline, focus_mode, sync_scroll, status) =
            if let Some(document) = command_line_document {
                (document, ViewMode::Split, true, false, true, None)
            } else if let Some(session) = restored {
                let was_dirty = session.dirty;
                let (document, view, show_outline, focus_mode, sync_scroll) =
                    session.into_document();
                (
                    document,
                    view,
                    show_outline,
                    focus_mode,
                    sync_scroll,
                    was_dirty.then(|| "Recovered unsaved changes".into()),
                )
            } else {
                (
                    Document::default(),
                    ViewMode::Split,
                    true,
                    false,
                    true,
                    None,
                )
            };
        Self {
            document,
            view,
            show_outline,
            show_find: false,
            find_query: String::new(),
            replace_query: String::new(),
            find_focus_requested: false,
            status,
            cursor_char: 0,
            selection: (0, 0),
            requested_selection: None,
            focus_mode,
            pending_action: None,
            show_table_dialog: false,
            table_columns: 3,
            table_rows: 2,
            last_disk_check: Instant::now(),
            disk_conflict: None,
            ignore_external_change: false,
            sync_scroll,
            scroll_ratio: 0.0,
            editor_scroll_max: 0.0,
            preview_scroll_max: 0.0,
        }
    }

    fn request_action(&mut self, action: PendingAction) {
        if self.document.dirty {
            self.pending_action = Some(action);
        } else {
            self.perform_action(action);
        }
    }

    fn perform_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::New => self.new_document(),
            PendingAction::Open => self.open(),
            PendingAction::Close => {
                // Handled by the caller because the context is required.
            }
        }
    }

    fn new_document(&mut self) {
        self.document = Document::default();
        self.document.text.clear();
        self.selection = (0, 0);
        self.requested_selection = Some((0, 0));
        self.status = Some("New document".into());
    }

    fn open(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Markdown", &["md", "markdown", "mdown"])
            .pick_file()
        {
            match Document::open(path) {
                Ok(doc) => {
                    self.document = doc;
                    self.ignore_external_change = false;
                    self.status = Some("Opened".into());
                }
                Err(err) => self.status = Some(format!("Open failed: {err}")),
            }
        }
    }

    fn save(&mut self) -> bool {
        if self.document.path.is_none() {
            return self.save_as();
        }
        match self.document.save() {
            Ok(()) => {
                self.ignore_external_change = false;
                self.status = Some("Saved".into());
                true
            }
            Err(err) => {
                self.status = Some(format!("Save failed: {err}"));
                false
            }
        }
    }

    fn save_as(&mut self) -> bool {
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(self.document.title())
            .add_filter("Markdown", &["md"])
            .save_file()
        {
            return match self.document.save_as(path) {
                Ok(()) => {
                    self.ignore_external_change = false;
                    self.status = Some("Saved".into());
                    true
                }
                Err(err) => {
                    self.status = Some(format!("Save failed: {err}"));
                    false
                }
            };
        }
        false
    }

    fn export_html(&mut self) {
        let mut default_name = PathBuf::from(self.document.title());
        default_name.set_extension("html");
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(default_name.to_string_lossy())
            .add_filter("HTML document", &["html", "htm"])
            .save_file()
        else {
            return;
        };
        let html = markdown::to_html_document(&self.document.text, &self.document.title());
        self.status = Some(match fs::write(&path, html) {
            Ok(()) => format!("Exported {}", path.display()),
            Err(error) => format!("Export failed: {error}"),
        });
    }

    fn update_table_of_contents(&mut self) {
        let (text, updated) = markdown::upsert_table_of_contents(&self.document.text);
        self.document.text = text;
        self.document.dirty = true;
        let cursor = self.cursor_char.min(self.document.text.chars().count());
        self.selection = (cursor, cursor);
        self.requested_selection = Some(self.selection);
        self.status = Some(if updated {
            "Updated table of contents".into()
        } else {
            "Inserted table of contents".into()
        });
    }

    fn format_tables(&mut self) {
        let (text, count) = markdown::format_tables(&self.document.text);
        if count == 0 {
            self.status = Some("No Markdown tables found".into());
            return;
        }
        if text != self.document.text {
            self.document.text = text;
            self.document.dirty = true;
            let cursor = self.cursor_char.min(self.document.text.chars().count());
            self.selection = (cursor, cursor);
            self.requested_selection = Some(self.selection);
        }
        self.status = Some(format!("Formatted {count} table(s)"));
    }

    fn insert(&mut self, kind: Insert) {
        let (start, end) = self.selection;
        let (text, selection) = apply_insert(&self.document.text, start, end, kind);
        self.document.text = text;
        self.selection = selection;
        self.cursor_char = selection.1;
        self.requested_selection = Some(selection);
        self.document.dirty = true;
    }

    fn insert_text(&mut self, text: &str, selection_offset: usize, selection_len: usize) {
        let start = self.selection.0.min(self.selection.1);
        let end = self.selection.0.max(self.selection.1);
        let start_byte = char_to_byte(&self.document.text, start);
        let end_byte = char_to_byte(&self.document.text, end);
        self.document.text.replace_range(start_byte..end_byte, text);
        let selection = (
            start + selection_offset,
            start + selection_offset + selection_len,
        );
        self.selection = selection;
        self.cursor_char = selection.1;
        self.requested_selection = Some(selection);
        self.document.dirty = true;
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let paths = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if paths.is_empty() {
            return;
        }
        let links = paths
            .iter()
            .map(|path| markdown_link_for_path(path, self.document.path.as_deref()))
            .collect::<Vec<_>>()
            .join("\n");
        let len = links.chars().count();
        self.insert_text(&links, len, 0);
        self.status = Some(format!("Inserted {} file link(s)", paths.len()));
    }

    fn find_next(&mut self, backwards: bool) {
        if self.find_query.is_empty() {
            return;
        }
        let source = &self.document.text;
        let cursor_byte = char_to_byte(source, self.selection.1);
        let found = if backwards {
            source[..char_to_byte(source, self.selection.0)]
                .rfind(&self.find_query)
                .or_else(|| source.rfind(&self.find_query))
        } else {
            source[cursor_byte..]
                .find(&self.find_query)
                .map(|offset| cursor_byte + offset)
                .or_else(|| source.find(&self.find_query))
        };
        if let Some(byte) = found {
            let start = source[..byte].chars().count();
            let end = start + self.find_query.chars().count();
            self.selection = (start, end);
            self.requested_selection = Some((start, end));
            self.view = if self.view == ViewMode::Preview {
                ViewMode::Split
            } else {
                self.view
            };
        }
    }

    fn replace_current(&mut self) {
        if self.find_query.is_empty() {
            return;
        }
        let (start, end) = self.selection;
        let start_byte = char_to_byte(&self.document.text, start);
        let end_byte = char_to_byte(&self.document.text, end);
        if self.document.text.get(start_byte..end_byte) != Some(self.find_query.as_str()) {
            self.find_next(false);
            return;
        }
        let replacement = self.replace_query.clone();
        let replacement_len = replacement.chars().count();
        self.document
            .text
            .replace_range(start_byte..end_byte, &replacement);
        self.document.dirty = true;
        self.selection = (start, start + replacement_len);
        self.requested_selection = Some(self.selection);
        self.find_next(false);
        self.status = Some("Replaced current match".into());
    }

    fn replace_all(&mut self) {
        let (text, count) =
            replace_all_literal(&self.document.text, &self.find_query, &self.replace_query);
        if count == 0 {
            self.status = Some("No matches to replace".into());
            return;
        }
        self.document.text = text;
        self.document.dirty = true;
        let cursor = self.cursor_char.min(self.document.text.chars().count());
        self.selection = (cursor, cursor);
        self.requested_selection = Some(self.selection);
        self.status = Some(format!("Replaced {count} match(es)"));
    }

    fn go_to_line(&mut self, line: usize) {
        let char_index = self
            .document
            .text
            .split_inclusive('\n')
            .take(line.saturating_sub(1))
            .map(str::chars)
            .map(Iterator::count)
            .sum();
        self.selection = (char_index, char_index);
        self.requested_selection = Some(self.selection);
        self.view = if self.view == ViewMode::Preview {
            ViewMode::Split
        } else {
            self.view
        };
    }

    fn check_disk(&mut self) {
        if self.last_disk_check.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_disk_check = Instant::now();
        match self.document.disk_state() {
            Ok(DiskState::Unchanged) => self.ignore_external_change = false,
            Ok(state @ (DiskState::Changed | DiskState::Missing))
                if !self.ignore_external_change && self.disk_conflict.is_none() =>
            {
                if state == DiskState::Changed && !self.document.dirty {
                    if let Some(path) = self.document.path.clone() {
                        match Document::open(path) {
                            Ok(document) => {
                                self.document = document;
                                self.status = Some("Reloaded external changes".into());
                            }
                            Err(error) => self.status = Some(format!("Reload failed: {error}")),
                        }
                    }
                } else {
                    self.disk_conflict = Some(state);
                }
            }
            Ok(_) => {}
            Err(error) => self.status = Some(format!("File check failed: {error}")),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let command = Modifiers::COMMAND;
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(command, Key::S))) {
            let _ = self.save();
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(command | Modifiers::SHIFT, Key::S))
        }) {
            let _ = self.save_as();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(command, Key::O))) {
            self.request_action(PendingAction::Open);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(command, Key::N))) {
            self.request_action(PendingAction::New);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(command, Key::F))) {
            self.show_find = true;
            self.find_focus_requested = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(command, Key::H))) {
            self.show_find = true;
            self.find_focus_requested = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(command, Key::B))) {
            self.insert(Insert::Bold);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(command, Key::I))) {
            self.insert(Insert::Italic);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(command, Key::K))) {
            self.insert(Insert::Link);
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(command | Modifiers::ALT, Key::L))
        }) {
            self.format_tables();
        }
    }
}

impl eframe::App for MarkGuin {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shortcuts(ctx);
        self.handle_dropped_files(ctx);
        self.check_disk();
        if ctx.input(|i| i.viewport().close_requested()) && self.document.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_action = Some(PendingAction::Close);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "{}{} — MarkGuin",
            if self.document.dirty { "● " } else { "" },
            self.document.title()
        )));

        if !self.focus_mode {
            self.top_bar(ctx);
        }
        if self.show_outline && !self.focus_mode {
            self.outline(ctx);
        }
        self.status_bar(ctx);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(CANVAS)
                    .inner_margin(egui::Margin::same(0)),
            )
            .show(ctx, |ui| match self.view {
                ViewMode::Editor => self.editor(ui),
                ViewMode::Preview => self.preview(ui),
                ViewMode::Split => {
                    let width = ui.available_width();
                    ui.columns(2, |cols| {
                        cols[0].set_width(width * 0.5);
                        self.editor(&mut cols[0]);
                        self.preview(&mut cols[1]);
                    });
                }
            });

        self.unsaved_dialog(ctx);
        self.table_dialog(ctx);
        self.external_change_dialog(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SESSION_KEY, &Session::capture(self));
    }
}

impl MarkGuin {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .exact_height(if self.show_find { 112.0 } else { 58.0 })
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .stroke(egui::Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin::symmetric(12, 9)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if icon_button(ui, Icon::Sidebar, "Toggle outline", self.show_outline).clicked()
                    {
                        self.show_outline = !self.show_outline;
                    }
                    ui.add_space(3.0);
                    if icon_button(ui, Icon::File, "New document", false).clicked() {
                        self.request_action(PendingAction::New);
                    }
                    if icon_button(ui, Icon::Folder, "Open Markdown file", false).clicked() {
                        self.request_action(PendingAction::Open);
                    }
                    if icon_button(ui, Icon::Save, "Save document", false).clicked() {
                        let _ = self.save();
                    }
                    if icon_button(ui, Icon::Export, "Export HTML", false).clicked() {
                        self.export_html();
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.add_space(1.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(self.document.title())
                                    .strong()
                                    .size(13.0)
                                    .color(TEXT),
                            );
                            if self.document.dirty {
                                ui.label(RichText::new("●").size(7.0).color(ACCENT));
                            }
                        });
                        ui.label(
                            RichText::new(if self.document.path.is_some() {
                                "Markdown document"
                            } else {
                                "Unsaved document"
                            })
                            .size(10.0)
                            .color(FAINT),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, Icon::Focus, "Distraction-free writing", false).clicked()
                        {
                            self.focus_mode = true;
                        }
                        if icon_button(ui, Icon::Search, "Find in document", self.show_find)
                            .clicked()
                        {
                            self.show_find = !self.show_find;
                            self.find_focus_requested = self.show_find;
                        }
                        ui.allocate_ui_with_layout(
                            egui::vec2(171.0, 32.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                egui::Frame::new()
                                    .fill(SURFACE_RAISED)
                                    .stroke(egui::Stroke::new(1.0, BORDER))
                                    .corner_radius(10)
                                    .inner_margin(2)
                                    .show(ui, |ui| {
                                        ui.with_layout(
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.spacing_mut().item_spacing.x = 0.0;
                                                view_button(
                                                    ui,
                                                    &mut self.view,
                                                    ViewMode::Editor,
                                                    "Edit",
                                                );
                                                view_button(
                                                    ui,
                                                    &mut self.view,
                                                    ViewMode::Split,
                                                    "Split",
                                                );
                                                view_button(
                                                    ui,
                                                    &mut self.view,
                                                    ViewMode::Preview,
                                                    "Preview",
                                                );
                                            },
                                        );
                                    });
                            },
                        );
                    });
                });
                if self.show_find {
                    ui.add_space(9.0);
                    egui::Frame::new()
                        .fill(SURFACE)
                        .stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(9)
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let response = ui.add(
                                    TextEdit::singleline(&mut self.find_query)
                                        .desired_width(220.0)
                                        .hint_text("Find…"),
                                );
                                if self.find_focus_requested {
                                    response.request_focus();
                                    self.find_focus_requested = false;
                                }
                                let count = if self.find_query.is_empty() {
                                    0
                                } else {
                                    self.document.text.matches(&self.find_query).count()
                                };
                                ui.label(
                                    RichText::new(format!("{count} found"))
                                        .size(11.0)
                                        .color(MUTED),
                                );
                                if compact_button(ui, "Previous").clicked() {
                                    self.find_next(true);
                                }
                                if compact_button(ui, "Next").clicked() {
                                    self.find_next(false);
                                }
                                ui.separator();
                                ui.add(
                                    TextEdit::singleline(&mut self.replace_query)
                                        .desired_width(180.0)
                                        .hint_text("Replace with…"),
                                );
                                if compact_button(ui, "Replace").clicked() {
                                    self.replace_current();
                                }
                                if compact_button(ui, "All").clicked() {
                                    self.replace_all();
                                }
                            });
                        });
                }
            });
    }

    fn outline(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("outline")
            .default_width(210.0)
            .min_width(176.0)
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .stroke(egui::Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin::symmetric(12, 14)),
            )
            .show(ctx, |ui| {
                let headings = markdown::headings(&self.document.text);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Outline").strong().size(12.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(headings.len().to_string())
                                .size(10.0)
                                .color(FAINT),
                        );
                    });
                });
                ui.add_space(13.0);
                if headings.is_empty() {
                    egui::Frame::new()
                        .fill(SURFACE)
                        .corner_radius(8)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("Add a heading to build your outline").color(MUTED),
                            );
                        });
                }
                for item in headings {
                    let indent = (item.level.saturating_sub(1) * 10) as f32;
                    let available = (ui.available_width() - indent).max(40.0);
                    ui.horizontal(|ui| {
                        ui.add_space(indent);
                        let label = egui::Button::new(
                            RichText::new(item.title)
                                .size(if item.level == 1 { 12.5 } else { 11.5 })
                                .color(if item.level == 1 { TEXT } else { MUTED }),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(6)
                        .min_size(egui::vec2(available, 27.0));
                        if ui
                            .add(label)
                            .on_hover_text(format!("Go to line {}", item.line))
                            .clicked()
                        {
                            self.go_to_line(item.line);
                        }
                    });
                }
            });
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(SURFACE)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("MARKDOWN").strong().size(9.0).color(FAINT));
                    ui.add_space(4.0);
                    for (label, hint, insert) in [
                        ("H", "Heading", Insert::Heading),
                        ("B", "Bold", Insert::Bold),
                        ("I", "Italic", Insert::Italic),
                        ("</>", "Inline code", Insert::Code),
                        ("↗", "Link", Insert::Link),
                        ("❯", "Quote", Insert::Quote),
                        ("•", "List", Insert::List),
                        ("☐", "Task", Insert::Task),
                    ] {
                        if format_button(ui, label).on_hover_text(hint).clicked() {
                            self.insert(insert);
                        }
                    }
                    ui.separator();
                    if format_button(ui, "Table").clicked() {
                        self.show_table_dialog = true;
                    }
                    if format_button(ui, "TOC")
                        .on_hover_text("Insert or update table of contents")
                        .clicked()
                    {
                        self.update_table_of_contents();
                    }
                    if format_button(ui, "—")
                        .on_hover_text("Horizontal rule")
                        .clicked()
                    {
                        self.insert(Insert::Rule);
                    }
                });
            });
        let mut area = egui::ScrollArea::both().id_salt("editor_scroll");
        if self.sync_scroll {
            area = area.vertical_scroll_offset(self.scroll_ratio * self.editor_scroll_max);
        }
        let scroll_output = area.show(ui, |ui| {
            let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
                let mut job = markdown::highlight_source(text);
                job.wrap.max_width = wrap_width;
                ui.fonts(|fonts| fonts.layout_job(job))
            };
            let mut output = TextEdit::multiline(&mut self.document.text)
                .id_source("document_editor")
                .font(FontId::monospace(15.0))
                .code_editor()
                .layouter(&mut layouter)
                .desired_width(f32::INFINITY)
                .desired_rows(40)
                .margin(egui::vec2(24.0, 22.0))
                .show(ui);
            if output.response.changed() {
                self.document.dirty = true;
            }
            if let Some(range) = output.cursor_range {
                self.cursor_char = range.primary.ccursor.index;
                let first = range.primary.ccursor.index;
                let second = range.secondary.ccursor.index;
                self.selection = (first.min(second), first.max(second));
            }
            if let Some((start, end)) = self.requested_selection.take() {
                output.state.cursor.set_char_range(Some(CCursorRange::two(
                    CCursor::new(start),
                    CCursor::new(end),
                )));
                output.state.store(ui.ctx(), output.response.id);
                output.response.request_focus();
            }
        });
        self.editor_scroll_max = scroll_max(
            scroll_output.content_size.y,
            scroll_output.inner_rect.height(),
        );
        if self.sync_scroll
            && ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|position| scroll_output.inner_rect.contains(position))
        {
            self.scroll_ratio =
                normalized_scroll(scroll_output.state.offset.y, self.editor_scroll_max);
        }
    }

    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending_action else {
            return;
        };
        egui::Window::new("Unsaved changes")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Unsaved changes")
                        .strong()
                        .size(18.0)
                        .color(TEXT),
                );
                ui.add_space(4.0);
                ui.label(format!("Save changes to {}?", self.document.title()));
                ui.label(
                    RichText::new("Your changes will be lost if you discard them.")
                        .color(Color32::GRAY),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() && self.save() {
                        self.pending_action = None;
                        self.finish_pending(action, ctx);
                    }
                    if ui.button("Discard").clicked() {
                        self.pending_action = None;
                        self.document.dirty = false;
                        self.finish_pending(action, ctx);
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_action = None;
                    }
                });
            });
    }

    fn table_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_table_dialog {
            return;
        }
        egui::Window::new("Insert table")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Insert table")
                        .strong()
                        .size(18.0)
                        .color(TEXT),
                );
                ui.label(RichText::new("Choose the table dimensions").color(MUTED));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label("Columns");
                    ui.add(egui::DragValue::new(&mut self.table_columns).range(1..=12));
                    ui.label("Body rows");
                    ui.add(egui::DragValue::new(&mut self.table_rows).range(1..=50));
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Insert").clicked() {
                        let table = generate_table(self.table_columns, self.table_rows);
                        let first_header_len = "Column 1".chars().count();
                        self.insert_text(&table, 2, first_header_len);
                        self.show_table_dialog = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_table_dialog = false;
                    }
                });
            });
    }

    fn finish_pending(&mut self, action: PendingAction, ctx: &egui::Context) {
        match action {
            PendingAction::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            other => self.perform_action(other),
        }
    }

    fn external_change_dialog(&mut self, ctx: &egui::Context) {
        let Some(state) = self.disk_conflict else {
            return;
        };
        let missing = state == DiskState::Missing;
        egui::Window::new("File changed outside MarkGuin")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("File changed outside MarkGuin")
                        .strong()
                        .size(18.0)
                        .color(TEXT),
                );
                ui.add_space(4.0);
                ui.label(if missing {
                    "The file was removed or moved on disk."
                } else {
                    "The file has changed on disk since it was opened."
                });
                if self.document.dirty {
                    ui.label(
                        RichText::new("Reloading will discard your unsaved changes.")
                            .color(Color32::from_rgb(235, 174, 95)),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!missing, egui::Button::new("Reload from disk"))
                        .clicked()
                        && let Some(path) = self.document.path.clone()
                    {
                        match Document::open(path) {
                            Ok(document) => {
                                self.document = document;
                                self.disk_conflict = None;
                                self.status = Some("Reloaded external changes".into());
                            }
                            Err(error) => self.status = Some(format!("Reload failed: {error}")),
                        }
                    }
                    if ui.button("Keep my version").clicked() {
                        let acknowledged = if missing {
                            self.ignore_external_change = true;
                            Ok(())
                        } else {
                            self.document.acknowledge_disk()
                        };
                        self.disk_conflict = None;
                        self.status = Some(match acknowledged {
                            Err(error) => format!("File check failed: {error}"),
                            Ok(()) if missing => {
                                "Keeping unsaved version; use Save As to restore the file".into()
                            }
                            Ok(()) => "Keeping local version".into(),
                        });
                    }
                });
            });
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(SURFACE)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .inner_margin(egui::Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PREVIEW").strong().size(9.0).color(FAINT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("Rendered Markdown").size(10.0).color(FAINT));
                    });
                });
            });
        let preview_width = ui.available_width().min(760.0);
        let mut area = egui::ScrollArea::vertical().id_salt("preview_scroll");
        if self.sync_scroll {
            area = area.vertical_scroll_offset(self.scroll_ratio * self.preview_scroll_max);
        }
        let scroll_output = area.show(ui, |ui| {
            ui.set_width(preview_width);
            ui.add_space(14.0);
            egui::Frame::NONE
                .inner_margin(egui::Margin::symmetric(18, 0))
                .show(ui, |ui| {
                    markdown::render(
                        ui,
                        &self.document.text,
                        self.document.path.as_deref().and_then(Path::parent),
                    )
                });
            ui.add_space(40.0);
        });
        self.preview_scroll_max = scroll_max(
            scroll_output.content_size.y,
            scroll_output.inner_rect.height(),
        );
        if self.sync_scroll
            && ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|position| scroll_output.inner_rect.contains(position))
        {
            self.scroll_ratio =
                normalized_scroll(scroll_output.state.offset.y, self.preview_scroll_max);
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(30.0)
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .stroke(egui::Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin::symmetric(12, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if self.focus_mode && ui.button("Exit focus").clicked() {
                        self.focus_mode = false;
                    }
                    if self.view == ViewMode::Split {
                        ui.toggle_value(&mut self.sync_scroll, "Sync scroll")
                            .on_hover_text("Keep editor and preview at the same relative position");
                    }
                    ui.label(
                        RichText::new(
                            self.document
                                .path
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "Unsaved document".into()),
                        )
                        .small()
                        .color(FAINT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} lines   {} words",
                                self.document.line_count(),
                                self.document.word_count()
                            ))
                            .size(10.0)
                            .color(FAINT),
                        );
                        if let Some(status) = &self.status {
                            ui.label(RichText::new(status).size(10.0).color(MUTED));
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("●").size(7.0).color(SUCCESS));
                            ui.label(RichText::new("Autosaved").size(10.0).color(FAINT));
                        })
                        .response
                        .on_hover_text("The current session is saved automatically");
                    });
                });
            });
    }
}

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

fn scroll_max(content_height: f32, viewport_height: f32) -> f32 {
    (content_height - viewport_height).max(0.0)
}

fn normalized_scroll(offset: f32, maximum: f32) -> f32 {
    if maximum <= f32::EPSILON {
        0.0
    } else {
        (offset / maximum).clamp(0.0, 1.0)
    }
}

fn apply_insert(source: &str, start: usize, end: usize, kind: Insert) -> (String, (usize, usize)) {
    let char_count = source.chars().count();
    let start = start.min(end).min(char_count);
    let end = end.max(start).min(char_count);
    let start_byte = char_to_byte(source, start);
    let end_byte = char_to_byte(source, end);
    let selected = &source[start_byte..end_byte];

    let (replacement, selected_offset, selected_len) = match kind {
        Insert::Bold => wrapped(selected, "**", "**", "bold text"),
        Insert::Italic => wrapped(selected, "*", "*", "italic text"),
        Insert::Code => wrapped(selected, "`", "`", "code"),
        Insert::Link => wrapped(selected, "[", "](https://)", "link text"),
        Insert::Heading => line_prefixed(selected, "## ", "Heading"),
        Insert::Quote => line_prefixed(selected, "> ", "Quote"),
        Insert::List => line_prefixed(selected, "- ", "List item"),
        Insert::Task => line_prefixed(selected, "- [ ] ", "Task"),
        Insert::Rule => ("\n---\n".to_owned(), 5, 0),
    };

    let mut result = String::with_capacity(source.len() + replacement.len());
    result.push_str(&source[..start_byte]);
    result.push_str(&replacement);
    result.push_str(&source[end_byte..]);
    let selection_start = start + selected_offset;
    (result, (selection_start, selection_start + selected_len))
}

fn wrapped(selected: &str, before: &str, after: &str, placeholder: &str) -> (String, usize, usize) {
    let content = if selected.is_empty() {
        placeholder
    } else {
        selected
    };
    (
        format!("{before}{content}{after}"),
        before.chars().count(),
        content.chars().count(),
    )
}

fn line_prefixed(selected: &str, prefix: &str, placeholder: &str) -> (String, usize, usize) {
    let content = if selected.is_empty() {
        placeholder
    } else {
        selected
    };
    let replacement = content
        .split_inclusive('\n')
        .map(|line| format!("{prefix}{line}"))
        .collect::<String>();
    (replacement, prefix.chars().count(), content.chars().count())
}

fn generate_table(columns: usize, rows: usize) -> String {
    let columns = columns.clamp(1, 12);
    let rows = rows.clamp(1, 50);
    let header = (1..=columns)
        .map(|column| format!("Column {column}"))
        .collect::<Vec<_>>()
        .join(" | ");
    let separator = std::iter::repeat_n("---", columns)
        .collect::<Vec<_>>()
        .join(" | ");
    let body = std::iter::repeat_n(" ", columns)
        .collect::<Vec<_>>()
        .join(" | ");
    let mut table = format!("| {header} |\n| {separator} |\n");
    for _ in 0..rows {
        table.push_str(&format!("| {body} |\n"));
    }
    table
}

fn markdown_link_for_path(path: &Path, document_path: Option<&Path>) -> String {
    let display_path = document_path
        .and_then(Path::parent)
        .and_then(|parent| path.strip_prefix(parent).ok())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let target = if display_path.contains([' ', '(', ')']) {
        format!("<{display_path}>")
    } else {
        display_path
    };
    let label = path
        .file_stem()
        .or_else(|| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let is_image = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif"
            )
        });
    if is_image {
        format!("![{label}]({target})")
    } else {
        format!("[{label}]({target})")
    }
}

fn replace_all_literal(source: &str, query: &str, replacement: &str) -> (String, usize) {
    if query.is_empty() {
        return (source.to_owned(), 0);
    }
    let count = source.matches(query).count();
    (source.replace(query, replacement), count)
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, tooltip: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            ACCENT_SOFT
        } else if response.hovered() {
            SURFACE_HOVER
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 8.0, fill);
        if selected {
            ui.painter().rect_stroke(
                rect,
                8.0,
                egui::Stroke::new(1.0, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        paint_icon(
            ui.painter(),
            rect.center(),
            icon,
            if selected { TEXT } else { MUTED },
        );
    }
    response.on_hover_text(tooltip)
}

fn paint_icon(painter: &egui::Painter, center: egui::Pos2, icon: Icon, color: Color32) {
    let stroke = egui::Stroke::new(1.45, color);
    let x = center.x;
    let y = center.y;
    match icon {
        Icon::Sidebar => {
            let rect = egui::Rect::from_center_size(center, egui::vec2(15.0, 14.0));
            painter.rect_stroke(rect, 2.5, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [egui::pos2(x - 2.5, y - 7.0), egui::pos2(x - 2.5, y + 7.0)],
                stroke,
            );
        }
        Icon::File => {
            painter.line_segment(
                [egui::pos2(x - 5.0, y - 7.0), egui::pos2(x + 2.0, y - 7.0)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(x - 5.0, y - 7.0), egui::pos2(x - 5.0, y + 7.0)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(x - 5.0, y + 7.0), egui::pos2(x + 5.0, y + 7.0)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(x + 5.0, y - 4.0), egui::pos2(x + 5.0, y + 7.0)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(x + 2.0, y - 7.0), egui::pos2(x + 5.0, y - 4.0)],
                stroke,
            );
            painter.line_segment([egui::pos2(x, y), egui::pos2(x + 6.0, y)], stroke);
            painter.line_segment(
                [egui::pos2(x + 3.0, y - 3.0), egui::pos2(x + 3.0, y + 3.0)],
                stroke,
            );
        }
        Icon::Folder => {
            let points = vec![
                egui::pos2(x - 7.0, y - 5.0),
                egui::pos2(x - 1.0, y - 5.0),
                egui::pos2(x + 1.0, y - 2.5),
                egui::pos2(x + 7.0, y - 2.5),
                egui::pos2(x + 6.0, y + 6.0),
                egui::pos2(x - 6.0, y + 6.0),
            ];
            painter.add(egui::Shape::closed_line(points, stroke));
        }
        Icon::Save => {
            let rect = egui::Rect::from_center_size(center, egui::vec2(14.0, 14.0));
            painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(
                egui::Rect::from_min_max(
                    egui::pos2(x - 3.5, y - 7.0),
                    egui::pos2(x + 3.5, y - 2.0),
                ),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.circle_stroke(egui::pos2(x, y + 3.0), 2.5, stroke);
        }
        Icon::Export => {
            painter.line_segment(
                [egui::pos2(x - 6.0, y + 1.0), egui::pos2(x - 6.0, y + 6.0)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(x - 6.0, y + 6.0), egui::pos2(x + 6.0, y + 6.0)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(x + 6.0, y + 6.0), egui::pos2(x + 6.0, y + 1.0)],
                stroke,
            );
            painter.line_segment([egui::pos2(x, y + 2.0), egui::pos2(x, y - 7.0)], stroke);
            painter.line_segment(
                [egui::pos2(x, y - 7.0), egui::pos2(x - 3.5, y - 3.5)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(x, y - 7.0), egui::pos2(x + 3.5, y - 3.5)],
                stroke,
            );
        }
        Icon::Search => {
            painter.circle_stroke(egui::pos2(x - 1.5, y - 1.5), 5.0, stroke);
            painter.line_segment(
                [egui::pos2(x + 2.0, y + 2.0), egui::pos2(x + 6.5, y + 6.5)],
                stroke,
            );
        }
        Icon::Focus => {
            for (a, b) in [
                ((-6.0, -2.0), (-6.0, -6.0)),
                ((-6.0, -6.0), (-2.0, -6.0)),
                ((6.0, -2.0), (6.0, -6.0)),
                ((6.0, -6.0), (2.0, -6.0)),
                ((-6.0, 2.0), (-6.0, 6.0)),
                ((-6.0, 6.0), (-2.0, 6.0)),
                ((6.0, 2.0), (6.0, 6.0)),
                ((6.0, 6.0), (2.0, 6.0)),
            ] {
                painter.line_segment(
                    [egui::pos2(x + a.0, y + a.1), egui::pos2(x + b.0, y + b.1)],
                    stroke,
                );
            }
        }
    }
}

fn view_button(ui: &mut egui::Ui, view: &mut ViewMode, value: ViewMode, label: &str) {
    let selected = *view == value;
    let button = egui::Button::new(RichText::new(label).size(11.0).color(if selected {
        TEXT
    } else {
        MUTED
    }))
    .fill(if selected {
        SURFACE_HOVER
    } else {
        Color32::TRANSPARENT
    })
    .stroke(egui::Stroke::NONE)
    .corner_radius(7)
    .min_size(egui::vec2(55.0, 28.0));
    if ui.add(button).clicked() {
        *view = value;
    }
}

fn compact_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(10.5).color(MUTED))
            .fill(SURFACE_RAISED)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .corner_radius(6)
            .min_size(egui::vec2(44.0, 24.0)),
    )
}

fn format_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(11.0).color(MUTED))
            .fill(Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE)
            .corner_radius(6)
            .min_size(egui::vec2(28.0, 24.0)),
    )
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SURFACE;
    style.visuals.window_stroke = egui::Stroke::new(1.0, BORDER);
    style.visuals.window_corner_radius = egui::CornerRadius::same(14);
    style.visuals.menu_corner_radius = egui::CornerRadius::same(10);
    style.visuals.extreme_bg_color = Color32::from_rgb(13, 16, 23);
    style.visuals.faint_bg_color = SURFACE_RAISED;
    style.visuals.code_bg_color = Color32::from_rgb(28, 31, 43);
    style.visuals.hyperlink_color = Color32::from_rgb(164, 145, 255);
    style.visuals.selection.bg_fill = ACCENT_SOFT;
    style.visuals.selection.stroke = egui::Stroke::new(1.0, Color32::from_rgb(197, 185, 255));
    style.visuals.widgets.noninteractive.bg_fill = SURFACE;
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE_RAISED;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, MUTED);
    style.visuals.widgets.hovered.weak_bg_fill = SURFACE_HOVER;
    style.visuals.widgets.hovered.bg_fill = SURFACE_HOVER;
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, BORDER);
    style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT);
    style.visuals.widgets.active.weak_bg_fill = ACCENT_SOFT;
    style.visuals.widgets.active.bg_fill = ACCENT_SOFT;
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, BORDER);
    style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, Color32::WHITE);
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(8);
    }
    style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.indent = 18.0;
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_char_position_becomes_valid_byte_position() {
        assert_eq!(char_to_byte("a日b", 2), 4);
        assert_eq!(char_to_byte("a日b", 99), 5);
    }

    #[test]
    fn formatting_wraps_selected_unicode_text() {
        let (text, selection) = apply_insert("before 日本語 after", 7, 10, Insert::Bold);
        assert_eq!(text, "before **日本語** after");
        assert_eq!(selection, (9, 12));
    }

    #[test]
    fn formatting_selects_placeholder_for_fast_replacement() {
        let (text, selection) = apply_insert("hello ", 6, 6, Insert::Link);
        assert_eq!(text, "hello [link text](https://)");
        assert_eq!(selection, (7, 16));
    }

    #[test]
    fn session_round_trip_preserves_unsaved_unicode_document() {
        let session = Session {
            text: "# 下書き\n未保存".into(),
            path: Some(PathBuf::from("notes.md")),
            dirty: true,
            view: ViewMode::Editor,
            show_outline: false,
            focus_mode: true,
            sync_scroll: true,
        };
        let json = serde_json::to_string(&session).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.text, session.text);
        assert_eq!(restored.path, session.path);
        assert!(restored.dirty);
        assert_eq!(restored.view, ViewMode::Editor);
        assert!(!restored.show_outline);
        assert!(restored.focus_mode);
        assert!(restored.sync_scroll);
    }

    #[test]
    fn table_generator_respects_requested_dimensions() {
        let table = generate_table(4, 3);
        assert_eq!(table.lines().count(), 5);
        assert!(table.lines().all(|line| line.matches('|').count() == 5));
        assert!(table.starts_with("| Column 1 | Column 2 | Column 3 | Column 4 |"));
    }

    #[test]
    fn dropped_image_uses_relative_markdown_image_link() {
        let link = markdown_link_for_path(
            Path::new("/notes/assets/cover image.png"),
            Some(Path::new("/notes/article.md")),
        );
        assert_eq!(link, "![cover image](<assets/cover image.png>)");
    }

    #[test]
    fn dropped_document_uses_normal_markdown_link() {
        let link = markdown_link_for_path(Path::new("/tmp/spec.pdf"), None);
        assert_eq!(link, "[spec](/tmp/spec.pdf)");
    }

    #[test]
    fn replace_all_is_unicode_safe_and_reports_count() {
        let (text, count) = replace_all_literal("猫と猫、犬", "猫", "ねこ");
        assert_eq!(text, "ねことねこ、犬");
        assert_eq!(count, 2);
    }

    #[test]
    fn empty_search_never_mutates_document() {
        let (text, count) = replace_all_literal("unchanged", "", "x");
        assert_eq!(text, "unchanged");
        assert_eq!(count, 0);
    }

    #[test]
    fn synchronized_scroll_uses_clamped_relative_position() {
        assert_eq!(scroll_max(1_000.0, 400.0), 600.0);
        assert_eq!(normalized_scroll(300.0, 600.0), 0.5);
        assert_eq!(normalized_scroll(900.0, 600.0), 1.0);
        assert_eq!(normalized_scroll(-20.0, 600.0), 0.0);
        assert_eq!(normalized_scroll(20.0, 0.0), 0.0);
    }
}
