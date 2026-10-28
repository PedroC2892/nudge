use crate::ics;
use crate::store::{Data, Freq, List, Reminder, Store};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use eframe::egui::{
    self, Align, Button, Color32, ComboBox, CornerRadius, DragValue, FontId, Frame, Id, Key, Layout,
    Margin, Modal, RichText, ScrollArea, Stroke, TextEdit, TextStyle, Ui, vec2,
};
use egui_extras::DatePickerButton;
use std::fs;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::SystemTime;

const ACCENT: Color32 = Color32::from_rgb(0x35, 0x84, 0xe4);
const DANGER: Color32 = Color32::from_rgb(0xe0, 0x1b, 0x24);
const STAR: Color32 = Color32::from_rgb(0xf6, 0xd3, 0x2d);

#[derive(Clone, Copy, PartialEq)]
enum View {
    All,
    Today,
    Upcoming,
    Overdue,
    Important,
    Completed,
    List(u64),
}

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Due,
    Title,
    Created,
}

/// Editor state for one reminder; the date/time widgets need plain fields.
struct Draft {
    r: Reminder,
    has_due: bool,
    date: NaiveDate,
    hour: u32,
    minute: u32,
    has_until: bool,
    until: NaiveDate,
}

impl Draft {
    fn new(r: Reminder) -> Self {
        // Default for a new due time: the next full hour.
        let base = r.due.unwrap_or_else(|| Local::now() + Duration::hours(1));
        Draft {
            has_due: r.due.is_some(),
            date: base.date_naive(),
            hour: base.hour(),
            minute: if r.due.is_some() { base.minute() } else { 0 },
            has_until: r.repeat.until.is_some(),
            until: r.repeat.until.unwrap_or(base.date_naive() + Duration::days(30)),
            r,
        }
    }

    fn finish(mut self) -> Reminder {
        let old_due = self.r.due;
        self.r.title = self.r.title.trim().to_string();
        self.r.due = self
            .has_due
            .then(|| {
                let t = self.date.and_time(NaiveTime::from_hms_opt(self.hour, self.minute, 0)?);
                Local.from_local_datetime(&t).earliest()
            })
            .flatten();
        if !self.has_due {
            self.r.repeat.freq = Freq::Never;
        }
        self.r.repeat.until = self.has_until.then_some(self.until);
        // A changed time means a fresh notification.
        if self.r.due != old_due {
            self.r.notified = false;
        }
        self.r
    }
}

pub struct App {
    store: Store,
    data: Data,
    mtime: Option<SystemTime>,
    view: View,
    search: String,
    sort: Sort,
    show_done: bool,
    draft: Option<Draft>,
    list_name: Option<(Option<u64>, String)>,
    confirm_delete: Option<(Option<u64>, u64)>, // (reminder, list) — exactly one is used
    settings_open: bool,
    status: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, listener: Option<UnixListener>) -> Self {
        if let Some(listener) = listener {
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                for _ in listener.incoming() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    ctx.request_repaint();
                }
            });
        }
        cc.egui_ctx.all_styles_mut(|s| {
            s.spacing.item_spacing = vec2(8.0, 8.0);
            s.spacing.button_padding = vec2(10.0, 6.0);
            s.spacing.interact_size.y = 30.0;
            s.text_styles.insert(TextStyle::Body, FontId::proportional(15.0));
            s.text_styles.insert(TextStyle::Button, FontId::proportional(15.0));
            s.text_styles.insert(TextStyle::Heading, FontId::proportional(24.0));
            s.text_styles.insert(TextStyle::Small, FontId::proportional(12.5));
            for w in [
                &mut s.visuals.widgets.inactive,
                &mut s.visuals.widgets.hovered,
                &mut s.visuals.widgets.active,
                &mut s.visuals.widgets.noninteractive,
            ] {
                w.corner_radius = CornerRadius::same(8);
            }
            s.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
            s.visuals.window_corner_radius = CornerRadius::same(14);
        });

        let store = Store::open().expect("cannot open data directory");
        let mut app = App {
            data: Data::default(),
            mtime: None,
            store,
            view: View::All,
            search: String::new(),
            sort: Sort::Due,
            show_done: false,
            draft: None,
            list_name: None,
            confirm_delete: None,
            settings_open: false,
            status: None,
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        match self.store.load() {
            Ok(d) => self.data = d,
            Err(e) => self.status = Some(format!("Cannot read data: {e}")),
        }
        self.mtime = self.store.modified();
    }

    /// Applies `f` to the freshly read file, so edits from the background
    /// service (notification actions) are never overwritten.
    fn commit(&mut self, f: impl FnOnce(&mut Data)) {
        match self.store.update(f) {
            Ok((d, ())) => self.data = d,
            Err(e) => self.status = Some(format!("Cannot save: {e}")),
        }
        self.mtime = self.store.modified();
    }

    fn fmt_time(&self, t: DateTime<Local>) -> String {
        let now = Local::now();
        let day = match (t.date_naive() - now.date_naive()).num_days() {
            0 => "Today".to_string(),
            1 => "Tomorrow".to_string(),
            -1 => "Yesterday".to_string(),
            _ if t.year() == now.year() => t.format("%a %-d %b").to_string(),
            _ => t.format("%a %-d %b %Y").to_string(),
        };
        let clock = if self.data.settings.time_24h { "%H:%M" } else { "%-I:%M %p" };
        format!("{day}, {}", t.format(clock))
    }

    fn matches(&self, r: &Reminder, view: View, now: DateTime<Local>) -> bool {
        let today = now.date_naive();
        match view {
            View::All => !r.done,
            View::Today => !r.done && r.due.is_some_and(|d| d.date_naive() == today),
            View::Upcoming => !r.done && r.due.is_some_and(|d| d > now),
            View::Overdue => r.is_overdue(now),
            View::Important => !r.done && r.important,
            View::Completed => r.done,
            View::List(id) => !r.done && r.list == Some(id),
        }
    }

    fn count(&self, view: View) -> usize {
        let now = Local::now();
        self.data.reminders.iter().filter(|r| self.matches(r, view, now)).count()
    }

    fn visible(&self) -> Vec<Reminder> {
        let now = Local::now();
        let q = self.search.to_lowercase();
        let mut v: Vec<Reminder> = self
            .data
            .reminders
            .iter()
            .filter(|r| {
                let in_view = self.matches(r, self.view, now)
                    || (self.show_done && r.done && self.matches(&Reminder { done: false, ..(*r).clone() }, self.view, now));
                in_view && (q.is_empty() || r.title.to_lowercase().contains(&q) || r.notes.to_lowercase().contains(&q))
            })
            .cloned()
            .collect();
        match self.sort {
            Sort::Due => v.sort_by_key(|r| (r.done, r.due.is_none(), r.due, !r.important)),
            Sort::Title => v.sort_by_key(|r| (r.done, r.title.to_lowercase())),
            Sort::Created => v.sort_by_key(|r| (r.done, std::cmp::Reverse(r.created))),
        }
        v
    }

    fn view_title(&self) -> String {
        match self.view {
            View::All => "All reminders".into(),
            View::Today => "Today".into(),
            View::Upcoming => "Upcoming".into(),
            View::Overdue => "Overdue".into(),
            View::Important => "Important".into(),
            View::Completed => "Completed".into(),
            View::List(id) => self.list_name_of(id).unwrap_or("List").into(),
        }
    }

    fn list_name_of(&self, id: u64) -> Option<&str> {
        self.data.lists.iter().find(|l| l.id == id).map(|l| l.name.as_str())
    }

    fn new_reminder(&mut self) {
        let list = match self.view {
            View::List(id) => Some(id),
            _ => None,
        };
        // Id is reserved on save; 0 marks "not stored yet".
        let mut r = Reminder::new(0, list);
        r.important = self.view == View::Important;
        let mut d = Draft::new(r);
        d.has_due = matches!(self.view, View::Today | View::Upcoming);
        self.draft = Some(d);
    }

    // ---------- panels ----------

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("⏰").size(26.0).color(ACCENT));
            ui.label(RichText::new("Nudge").size(22.0).strong());
        });
        ui.add_space(10.0);

        let smart = [
            (View::All, "📋", "All"),
            (View::Today, "☀", "Today"),
            (View::Upcoming, "📅", "Upcoming"),
            (View::Overdue, "⚠", "Overdue"),
            (View::Important, "★", "Important"),
            (View::Completed, "✔", "Completed"),
        ];
        for (view, icon, name) in smart {
            self.nav_item(ui, view, icon, name);
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("LISTS").small().weak());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(Button::new("+").frame(false)).on_hover_text("New list").clicked() {
                    self.list_name = Some((None, String::new()));
                }
            });
        });
        let lists = self.data.lists.clone();
        for l in &lists {
            let resp = self.nav_item(ui, View::List(l.id), "•", &l.name);
            resp.context_menu(|ui| {
                if ui.button("Rename").clicked() {
                    self.list_name = Some((Some(l.id), l.name.clone()));
                }
                if ui.button(RichText::new("Delete").color(DANGER)).clicked() {
                    self.confirm_delete = Some((None, l.id));
                }
            });
        }
        if lists.is_empty() {
            ui.label(RichText::new("Use + to group reminders into lists").small().weak());
        }

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(6.0);
            if ui.add(Button::new("⚙  Settings").frame(false)).clicked() {
                self.settings_open = true;
            }
        });
    }

    fn nav_item(&mut self, ui: &mut Ui, view: View, icon: &str, name: &str) -> egui::Response {
        let n = self.count(view);
        let color = match view {
            View::Overdue if n > 0 => DANGER,
            View::Important => STAR,
            _ => ACCENT,
        };
        let text = egui::Atoms::new((RichText::new(icon).color(color), format!("  {name}")));
        let resp = ui.add(
            Button::selectable(self.view == view, text)
                .right_text(RichText::new(if n > 0 { n.to_string() } else { String::new() }).weak())
                .min_size(vec2(ui.available_width(), 34.0)),
        );
        if resp.clicked() {
            self.view = view;
        }
        resp
    }

    fn main(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new(self.view_title()).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(Button::new(RichText::new("+  New reminder").color(Color32::WHITE)).fill(ACCENT))
                    .on_hover_text("Ctrl+N")
                    .clicked()
                {
                    self.new_reminder();
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut self.search).id(Id::unique("search")).hint_text("🔍 Search…").desired_width(260.0));
            ComboBox::from_id_salt("sort")
                .selected_text(match self.sort {
                    Sort::Due => "Sort: time",
                    Sort::Title => "Sort: title",
                    Sort::Created => "Sort: newest",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.sort, Sort::Due, "Time");
                    ui.selectable_value(&mut self.sort, Sort::Title, "Title");
                    ui.selectable_value(&mut self.sort, Sort::Created, "Newest");
                });
            if self.view != View::Completed {
                ui.checkbox(&mut self.show_done, "Show completed");
            }
        });
        if let Some(msg) = self.status.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(msg).color(DANGER));
                if ui.small_button("✖").clicked() {
                    self.status = None;
                }
            });
        }
        ui.separator();

        let items = self.visible();
        if items.is_empty() {
            ui.add_space(80.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("🎉").size(48.0));
                ui.label(RichText::new("Nothing to do here").size(18.0).weak());
                ui.label(RichText::new("Press Ctrl+N to add a reminder").weak());
            });
            return;
        }
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for r in &items {
                self.card(ui, r);
            }
        });
    }

    fn card(&mut self, ui: &mut Ui, r: &Reminder) {
        let now = Local::now();
        let overdue = r.is_overdue(now);
        let border = if overdue { DANGER.gamma_multiply(0.6) } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
        let frame = Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(10)
            .inner_margin(Margin::symmetric(12, 10));
        let resp = frame
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let check = if r.done { RichText::new("✔").color(Color32::WHITE) } else { RichText::new("  ") };
                    let fill = if r.done { ACCENT } else { Color32::TRANSPARENT };
                    if ui
                        .add(Button::new(check).fill(fill).stroke(Stroke::new(1.5, ACCENT)).corner_radius(14).min_size(vec2(26.0, 26.0)))
                        .on_hover_text(if r.done { "Mark as not done" } else { "Mark as done" })
                        .clicked()
                    {
                        let id = r.id;
                        self.commit(|d| {
                            if let Some(x) = d.get_mut(id) {
                                x.done = !x.done;
                            }
                        });
                    }
                    ui.vertical(|ui| {
                        let mut title = RichText::new(&r.title).size(16.0);
                        title = if r.done { title.strikethrough().weak() } else { title.strong() };
                        ui.label(title);
                        if let Some(first) = r.notes.lines().next().filter(|l| !l.is_empty()) {
                            ui.label(RichText::new(first).weak());
                        }
                        ui.horizontal_wrapped(|ui| {
                            if let Some(due) = r.due {
                                let c = if overdue { DANGER } else { ACCENT };
                                chip(ui, &format!("🕑 {}", self.fmt_time(due)), c);
                            }
                            if r.repeat.is_on() {
                                chip(ui, &format!("🔁 {}", r.repeat.describe()), ui.visuals().weak_text_color());
                            }
                            if let Some(name) = r.list.and_then(|id| self.list_name_of(id)) {
                                chip(ui, &format!("• {name}"), ui.visuals().weak_text_color());
                            }
                        });
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(Button::new("🗑").frame(false)).on_hover_text("Delete").clicked() {
                            self.confirm_delete = Some((Some(r.id), 0));
                        }
                        if ui.add(Button::new("✏").frame(false)).on_hover_text("Edit").clicked() {
                            self.draft = Some(Draft::new(r.clone()));
                        }
                        let star = if r.important { RichText::new("★").color(STAR) } else { RichText::new("☆").weak() };
                        if ui.add(Button::new(star.size(18.0)).frame(false)).on_hover_text("Important").clicked() {
                            let id = r.id;
                            self.commit(|d| {
                                if let Some(x) = d.get_mut(id) {
                                    x.important = !x.important;
                                }
                            });
                        }
                    });
                });
            })
            .response;
        if resp.interact(egui::Sense::click()).double_clicked() {
            self.draft = Some(Draft::new(r.clone()));
        }
        ui.add_space(2.0);
    }

    // ---------- dialogs ----------

    fn editor(&mut self, ctx: &egui::Context) {
        let Some(d) = self.draft.as_mut() else { return };
        let mut save = false;
        let mut cancel = false;
        let lists = self.data.lists.clone();
        let modal = Modal::new(Id::unique("editor")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading(if d.r.id == 0 { "New reminder" } else { "Edit reminder" });
            ui.add_space(6.0);
            let title = ui.add(TextEdit::singleline(&mut d.r.title).hint_text("What do you need to do?").desired_width(f32::INFINITY));
            if d.r.id == 0 && d.r.title.is_empty() {
                title.request_focus();
            }
            ui.add(TextEdit::multiline(&mut d.r.notes).hint_text("Notes").desired_rows(3).desired_width(f32::INFINITY));

            ui.add_space(4.0);
            ui.checkbox(&mut d.has_due, "Remind me at a time");
            if d.has_due {
                ui.horizontal(|ui| {
                    date_picker(ui, &mut d.date, "due_date");
                    ui.add(DragValue::new(&mut d.hour).range(0..=23).custom_formatter(|v, _| format!("{v:02}")));
                    ui.label(":");
                    ui.add(DragValue::new(&mut d.minute).range(0..=59).custom_formatter(|v, _| format!("{v:02}")));
                });
                ui.horizontal(|ui| {
                    for (label, mins) in [("In 15 min", 15), ("In 1 hour", 60), ("Tomorrow 9:00", -1)] {
                        if ui.small_button(label).clicked() {
                            let t = if mins < 0 {
                                let tomorrow = Local::now().date_naive() + Duration::days(1);
                                Local.from_local_datetime(&tomorrow.and_hms_opt(9, 0, 0).unwrap()).earliest().unwrap_or_else(Local::now)
                            } else {
                                Local::now() + Duration::minutes(mins)
                            };
                            (d.date, d.hour, d.minute) = (t.date_naive(), t.hour(), t.minute());
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Repeat");
                    if d.r.repeat.is_on() {
                        ui.label("every");
                        ui.add(DragValue::new(&mut d.r.repeat.every).range(1..=999));
                    }
                    ComboBox::from_id_salt("freq").selected_text(d.r.repeat.freq.label()).show_ui(ui, |ui| {
                        for f in Freq::ALL {
                            ui.selectable_value(&mut d.r.repeat.freq, f, f.label());
                        }
                    });
                });
                if d.r.repeat.is_on() {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut d.has_until, "Until");
                        if d.has_until {
                            date_picker(ui, &mut d.until, "until_date");
                        }
                    });
                }
            }

            ui.horizontal(|ui| {
                ui.label("List");
                let current = d.r.list.and_then(|id| lists.iter().find(|l| l.id == id)).map_or("None", |l| l.name.as_str());
                ComboBox::from_id_salt("list").selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut d.r.list, None, "None");
                    for l in &lists {
                        ui.selectable_value(&mut d.r.list, Some(l.id), &l.name);
                    }
                });
                ui.checkbox(&mut d.r.important, "★ Important");
            });

            ui.add_space(8.0);
            let ok = !d.r.title.trim().is_empty();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(ok, Button::new(RichText::new("Save").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
            if ok && ok_shortcut(ui, &title) {
                save = true;
            }
        });
        if save {
            let r = self.draft.take().unwrap().finish();
            self.commit(|d| {
                let mut r = r;
                if r.id == 0 {
                    r.id = d.new_id();
                }
                d.upsert(r);
            });
        } else if modal.should_close() || cancel {
            self.draft = None;
        }
    }

    fn list_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, name)) = self.list_name.as_mut() else { return };
        let mut save = false;
        let modal = Modal::new(Id::unique("list")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading(if id.is_some() { "Rename list" } else { "New list" });
            let field = ui.add(TextEdit::singleline(name).hint_text("List name").desired_width(f32::INFINITY));
            field.request_focus();
            save = ok_shortcut(ui, &field);
            ui.horizontal(|ui| {
                if ui.add(Button::new(RichText::new("Save").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    ui.close();
                }
            });
        });
        if save {
            let (id, name) = self.list_name.take().unwrap();
            let name = name.trim().to_string();
            if !name.is_empty() {
                let mut created = None;
                self.commit(|d| match id {
                    Some(id) => d.lists.iter_mut().filter(|l| l.id == id).for_each(|l| l.name = name.clone()),
                    None => {
                        let id = d.new_id();
                        d.lists.push(List { id, name });
                        created = Some(id);
                    }
                });
                if let Some(id) = created {
                    self.view = View::List(id);
                }
            }
        } else if modal.should_close() {
            self.list_name = None;
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some((reminder, list)) = self.confirm_delete else { return };
        let what = match reminder {
            Some(id) => self.data.reminders.iter().find(|r| r.id == id).map(|r| format!("Delete “{}”?", r.title)),
            None => self.list_name_of(list).map(|n| format!("Delete list “{n}” and all its reminders?")),
        };
        let Some(what) = what else {
            self.confirm_delete = None;
            return;
        };
        let mut yes = false;
        let modal = Modal::new(Id::unique("delete")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(RichText::new(what).size(16.0));
            ui.label(RichText::new("This cannot be undone.").weak());
            ui.horizontal(|ui| {
                if ui.add(Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(DANGER)).clicked() {
                    yes = true;
                }
                if ui.button("Cancel").clicked() {
                    ui.close();
                }
            });
        });
        if yes {
            self.commit(|d| match reminder {
                Some(id) => d.reminders.retain(|r| r.id != id),
                None => {
                    d.lists.retain(|l| l.id != list);
                    d.reminders.retain(|r| r.list != Some(list));
                }
            });
            if reminder.is_none() && self.view == View::List(list) {
                self.view = View::All;
            }
            self.confirm_delete = None;
        } else if modal.should_close() {
            self.confirm_delete = None;
        }
    }

    fn settings(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut s = self.data.settings.clone();
        let mut autostart = autostart_path().is_some_and(|p| p.exists());
        let was_autostart = autostart;
        let (mut import, mut export) = (false, false);
        let modal = Modal::new(Id::unique("settings")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Settings");
            ui.add_space(6.0);
            ui.checkbox(&mut s.time_24h, "Use 24-hour time");
            ui.checkbox(&mut s.sound, "Play sound with notifications");
            ui.horizontal(|ui| {
                ui.label("Snooze for");
                ui.add(DragValue::new(&mut s.snooze_minutes).range(1..=240).suffix(" min"));
            });
            ui.checkbox(&mut autostart, "Start notification service on login");
            ui.label(RichText::new("Notifications keep working after you close this window.").small().weak());
            ui.separator();
            ui.horizontal(|ui| {
                import = ui.button("📥 Import .ics").clicked();
                export = ui.button("📤 Export .ics").clicked();
            });
            ui.add_space(6.0);
            if ui.button("Close").clicked() {
                ui.close();
            }
        });
        if s != self.data.settings {
            self.commit(|d| d.settings = s);
        }
        if autostart != was_autostart
            && let Err(e) = set_autostart(autostart)
        {
            self.status = Some(format!("Cannot change autostart: {e}"));
        }
        if import {
            self.import_ics();
        }
        if export {
            self.export_ics();
        }
        if modal.should_close() {
            self.settings_open = false;
        }
    }

    fn import_ics(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("iCalendar", &["ics"]).pick_file() else { return };
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => return self.status = Some(format!("Cannot read {}: {e}", path.display())),
        };
        let list = match self.view {
            View::List(id) => Some(id),
            _ => None,
        };
        let mut n = 0;
        self.commit(|d| {
            let mut next = d.next_id;
            let items = ics::import(&text, || { next += 1; next }, list);
            d.next_id = next;
            n = items.len();
            d.reminders.extend(items);
        });
        self.status = Some(format!("Imported {n} reminders"));
    }

    fn export_ics(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("iCalendar", &["ics"]).set_file_name("reminders.ics").save_file() else {
            return;
        };
        self.status = Some(match fs::write(&path, ics::export(&self.data.reminders)) {
            Ok(()) => format!("Exported {} reminders to {}", self.data.reminders.len(), path.display()),
            Err(e) => format!("Cannot write {}: {e}", path.display()),
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Pick up changes made by the background service (snooze, done, repeats).
        if self.store.modified() != self.mtime {
            self.reload();
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        let no_dialog = self.draft.is_none() && self.list_name.is_none() && self.confirm_delete.is_none() && !self.settings_open;
        if no_dialog && ctx.input(|i| i.modifiers.command && i.key_pressed(Key::N)) {
            self.new_reminder();
        }
        if no_dialog && ctx.input(|i| i.modifiers.command && i.key_pressed(Key::F)) {
            ctx.memory_mut(|m| m.request_focus(Id::unique("search")));
        }

        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(220.0)
            .frame(Frame::new().fill(ui.visuals().panel_fill).inner_margin(12))
            .show(ui, |ui| self.sidebar(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(ui.visuals().extreme_bg_color).inner_margin(Margin::symmetric(24, 16)))
            .show(ui, |ui| self.main(ui));

        self.editor(&ctx);
        self.list_dialog(&ctx);
        self.delete_dialog(&ctx);
        self.settings(&ctx);
    }
}

/// The egui date picker works on jiff dates; the rest of the app uses chrono.
fn date_picker(ui: &mut Ui, date: &mut NaiveDate, salt: &str) {
    let mut j = jiff::civil::date(date.year() as i16, date.month() as i8, date.day() as i8);
    ui.add(DatePickerButton::new(&mut j).id_salt(salt).calendar_week(false));
    if let Some(n) = NaiveDate::from_ymd_opt(j.year().into(), j.month() as u32, j.day() as u32) {
        *date = n;
    }
}

fn chip(ui: &mut Ui, text: &str, color: Color32) {
    Frame::new()
        .fill(color.gamma_multiply(0.12))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(color)));
}

/// Enter in the single-line field, or Ctrl+Enter anywhere, confirms a dialog.
fn ok_shortcut(ui: &Ui, field: &egui::Response) -> bool {
    ui.input(|i| i.key_pressed(Key::Enter) && (i.modifiers.command || field.lost_focus()))
}

fn autostart_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("autostart").join("nudge-service.desktop"))
}

fn set_autostart(on: bool) -> std::io::Result<()> {
    let path = autostart_path().ok_or_else(|| std::io::Error::other("no config directory"))?;
    if !on {
        return fs::remove_file(path);
    }
    let exe = std::env::current_exe()?;
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(
        path,
        format!(
            "[Desktop Entry]\nType=Application\nName=Nudge\nComment=Reminder notifications and tray icon\nExec=\"{}\" --daemon\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        ),
    )
}
