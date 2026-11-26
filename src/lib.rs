//! Notes: an in-game notepad. N opens it; notes are WORLD, SERVER or GLOBAL, saved in
//! `horizon-mod-data/notes/`, and pinned notes are drawn on the HUD.
mod layout;
mod store;

use std::sync::Mutex;

use mod_api::client::widgets::{Button, Outcome, TextField, TextEdit, WidgetId, Widgets};
use mod_api::{
    ClientHost, ClientTickPostEvent, ConfigPath, Flow, GuiLayer, GuiLayerDrawEvent,
    GuiLayerPlacement, Hooks, KeyConflictContext, KeyRegistration, RegisterGuiLayersEvent,
    RegistrationContext, Registrar, ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent,
    ScreenMouseButtonPressedPreEvent, ScreenMouseButtonReleasedPreEvent,
    ScreenMouseDraggedPreEvent, ScreenMouseScrolledPreEvent, ScreenRenderPostEvent, Text,
};
use mod_api::client::widgets::EditKey;
use store::{BODY_MAX, Note, Scope, TITLE_MAX};

pub const ID: &str = "notes";
const KEY: &str = "open";
const DEFAULT_KEY: &str = "key.keyboard.n";
const HUD_LAYER: &str = "notes:pinned";

const LINE_H: f32 = 10.0;
const ROW_H: f32 = 11.0;
const WHITE: u32 = 0xFFFF_FFFF;
const GREY: u32 = 0xFFA0_A0A0;
const GOLD: u32 = 0xFFFF_D24A;
const RED: u32 = 0xFFFF_6060;

/// Where the notes of the current play session are kept.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scope: Scope,
    key: String,
}

struct Ui {
    widgets: Widgets,
    title: WidgetId,
    new: WidgetId,
    pin: WidgetId,
    delete: WidgetId,
    close: WidgetId,
    tabs: [WidgetId; 3],
    tab: Scope,
    selected: Option<usize>,
    body: TextEdit,
    body_focus: bool,
    body_scroll: usize,
    follow: bool,
    lines: Vec<layout::Line>,
    confirm_delete: bool,
    list_scroll: usize,
    list_rows: usize,
    body_visible: usize,
    // Geometry of the last frame, for clicks.
    list_rect: [f32; 4],
    body_rect: [f32; 4],
    title_rect: [f32; 4],
}

#[derive(Default)]
struct State {
    context: Option<Context>,
    notes: [Vec<Note>; 3],
    /// Scopes whose file could not be read: never written over.
    locked: [bool; 3],
    global_loaded: bool,
    error: Option<String>,
    key_was_down: bool,
    close: bool,
    ui: Option<Ui>,
}

fn contains(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3]
}

/// A thin scrollbar on the right edge of `rect` when more than `shown` of `total` rows exist.
fn scrollbar(d: &mut dyn mod_api::DrawContext, rect: [f32; 4], scroll: usize, total: usize, shown: usize) {
    if total <= shown {
        return;
    }
    let max = (total - shown) as f32;
    let thumb = (rect[3] * shown as f32 / total as f32).max(8.0);
    let y = rect[1] + (rect[3] - thumb) * (scroll as f32 / max).min(1.0);
    d.fill([rect[0] + rect[2] - 4.0, rect[1], 3.0, rect[3]], 0xFF22_2018);
    d.fill([rect[0] + rect[2] - 4.0, y, 3.0, thumb], 0xFFB8_A050);
}

fn context_of(host: &dyn ClientHost) -> Option<Context> {
    let server = host.client_session().client_server()?;
    if server.integrated_local {
        server.save_folder.map(|key| Context { scope: Scope::World, key })
    } else {
        server.address.map(|key| Context { scope: Scope::Server, key })
    }
}

impl State {
    fn file(&self, scope: Scope) -> Option<String> {
        match scope {
            Scope::Global => Some(store::file_name(Scope::Global, "")),
            s => self
                .context
                .as_ref()
                .filter(|c| c.scope == s)
                .map(|c| store::file_name(s, &c.key)),
        }
    }

    fn available(&self, scope: Scope) -> bool {
        scope == Scope::Global || self.context.as_ref().is_some_and(|c| c.scope == scope)
    }

    /// Reads one scope's file. A missing file is an empty list; an unreadable one locks the scope
    /// so it is never overwritten; unreadable lines are kept in a `.bak` copy first.
    fn load(&mut self, host: &dyn ClientHost, scope: Scope) {
        let i = scope.index();
        self.notes[i].clear();
        self.locked[i] = false;
        let Some(name) = self.file(scope) else { return };
        let Ok(path) = ConfigPath::new(&name) else { return };
        let files = host.data_files();
        match files.stat(&path) {
            Ok(None) => return,
            Ok(Some(_)) => {}
            Err(e) => {
                self.locked[i] = true;
                self.error = Some(format!("Cannot read file: {e:?}"));
                return;
            }
        }
        match files.read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let (notes, bad) = store::decode(&text);
                if bad > 0 {
                    if let Ok(bak) = ConfigPath::new(&format!("{name}.bak")) {
                        let _ = files.write(&bak, &bytes);
                    }
                    self.error = Some(format!("{bad} unreadable line(s), .bak copy made"));
                }
                self.notes[i] = notes;
            }
            Err(e) => {
                self.locked[i] = true;
                self.error = Some(format!("Cannot read file: {e:?}"));
            }
        }
    }

    fn save(&mut self, host: &dyn ClientHost, scope: Scope) {
        let i = scope.index();
        if self.locked[i] {
            self.error = Some("Unreadable file: save refused".into());
            return;
        }
        let Some(name) = self.file(scope) else { return };
        let Ok(path) = ConfigPath::new(&name) else { return };
        let text = store::encode(&self.notes[i]);
        match host.data_files().write(&path, text.as_bytes()) {
            Ok(()) => self.error = None,
            Err(e) => self.error = Some(format!("Save failed: {e:?}")),
        }
    }

    fn refresh_context(&mut self, host: &dyn ClientHost) {
        if !self.global_loaded {
            self.global_loaded = true;
            self.load(host, Scope::Global);
        }
        let now = context_of(host);
        if now != self.context {
            self.context = now;
            self.ui = None;
            for s in [Scope::World, Scope::Server] {
                self.load(host, s);
            }
        }
    }
}

impl Ui {
    fn new(default_tab: Scope) -> Self {
        let mut widgets = Widgets::new();
        let z = [0.0, 0.0, 10.0, 20.0];
        let mut title = TextField::new(z, TITLE_MAX);
        title.message = "Title".into();
        let title = widgets.add_text_field(title);
        let mut b = |label: &str| widgets.add_button(Button::new(z, Text::literal(label)));
        let new = b("New");
        let pin = b("Pin");
        let delete = b("Delete");
        let close = b("Close");
        let tabs = [b("WORLD"), b("SERVER"), b("GLOBAL")];
        let mut body = TextEdit::new(BODY_MAX);
        body.multiline = true;
        Self {
            widgets,
            title,
            new,
            pin,
            delete,
            close,
            tabs,
            tab: default_tab,
            selected: None,
            body,
            body_focus: false,
            body_scroll: 0,
            follow: true,
            lines: Vec::new(),
            confirm_delete: false,
            list_scroll: 0,
            list_rows: 1,
            body_visible: 1,
            list_rect: [0.0; 4],
            body_rect: [0.0; 4],
            title_rect: [0.0; 4],
        }
    }

    /// Copies the selected note into the editing widgets.
    fn sync(&mut self, notes: &[Note]) {
        let note = self.selected.and_then(|i| notes.get(i));
        if self.selected.is_some() && note.is_none() {
            self.selected = None;
        }
        let (title, body) = note.map_or(("", ""), |n| (n.title.as_str(), n.body.as_str()));
        if let Some(f) = self.widgets.text_field_mut(self.title) {
            f.set_value(title);
            f.active = note.is_some();
        }
        self.body.set_value(body);
        self.body.cursor = 0;
        self.body.highlight = 0;
        self.body_scroll = 0;
        self.follow = true;
        self.confirm_delete = false;
    }

    fn place(&mut self, w: f32, h: f32, state: &State) -> [f32; 4] {
        let pw = (w - 16.0).min(380.0);
        let ph = (h - 16.0).min(230.0);
        let (px, py) = (((w - pw) / 2.0).floor(), ((h - ph) / 2.0).floor());
        let lw = 132.0;
        let rx = px + lw + 4.0;
        let rw = pw - lw - 10.0;
        let tab_w = ((lw - 8.0) / 3.0).floor();
        for (i, t) in self.tabs.iter().enumerate() {
            let scope = Scope::ALL[i];
            if let Some(b) = self.widgets.button_mut(*t) {
                b.rect = [px + 4.0 + i as f32 * tab_w, py + 20.0, tab_w - 1.0, 16.0];
                b.active = state.available(scope) && scope != self.tab;
            }
        }
        let bottom = py + ph - 24.0;
        if let Some(b) = self.widgets.button_mut(self.new) {
            b.rect = [px + 4.0, bottom, lw - 8.0, 20.0];
        }
        let bw = ((rw - 4.0) / 3.0).floor();
        for (i, id) in [self.pin, self.delete, self.close].into_iter().enumerate() {
            if let Some(b) = self.widgets.button_mut(id) {
                b.rect = [rx + i as f32 * (bw + 2.0), bottom, bw, 20.0];
            }
        }
        self.title_rect = [rx, py + 20.0, rw, 20.0];
        if let Some(f) = self.widgets.text_field_mut(self.title) {
            f.rect = self.title_rect;
        }
        self.list_rect = [px + 4.0, py + 40.0, lw - 8.0, bottom - py - 44.0];
        self.body_rect = [rx, py + 44.0, rw, bottom - py - 48.0];
        [px, py, pw, ph]
    }
}

/// Everything the mod keeps: its state behind one lock, never held across a host call that
/// could fire another of its hooks (screen opening and closing).
#[derive(Default)]
struct Notes {
    state: Mutex<State>,
}

impl Notes {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn input(
        &self,
        host: &dyn ClientHost,
        f: impl FnOnce(&mut State, &mut Ui) -> bool,
    ) -> Flow {
        if host.mod_screen() != Some(ID) {
            return Flow::Continue;
        }
        let mut st = self.state();
        let Some(mut ui) = st.ui.take() else { return Flow::Continue };
        let handled = f(&mut st, &mut ui);
        st.ui = Some(ui);
        if handled { Flow::Cancel } else { Flow::Continue }
    }
}

/// The current tab's notes, mutable.
fn tab_notes<'a>(st: &'a mut State, ui: &Ui) -> &'a mut Vec<Note> {
    &mut st.notes[ui.tab.index()]
}

fn commit_title(st: &mut State, ui: &mut Ui, host: &dyn ClientHost) {
    let value = ui.widgets.text_field(ui.title).map(|f| f.value().to_owned());
    if let (Some(i), Some(value)) = (ui.selected, value) {
        if let Some(n) = tab_notes(st, ui).get_mut(i) {
            n.title = value;
        }
        st.save(host, ui.tab);
    }
}

fn commit_body(st: &mut State, ui: &mut Ui, host: &dyn ClientHost) {
    if let Some(i) = ui.selected {
        let text = ui.body.value.clone();
        if let Some(n) = tab_notes(st, ui).get_mut(i) {
            n.body = text;
        }
        st.save(host, ui.tab);
        ui.follow = true;
    }
}

fn press(st: &mut State, ui: &mut Ui, host: &dyn ClientHost, id: WidgetId) {
    let was_confirming = std::mem::take(&mut ui.confirm_delete);
    if let Some(t) = ui.tabs.iter().position(|t| *t == id) {
        let scope = Scope::ALL[t];
        if st.available(scope) {
            ui.tab = scope;
            ui.selected = None;
            ui.list_scroll = 0;
            ui.sync(&st.notes[scope.index()].clone());
        }
    } else if id == ui.new {
        let scope = ui.tab;
        if st.locked[scope.index()] {
            st.error = Some("Unreadable file: creation refused".into());
            return;
        }
        let list = &mut st.notes[scope.index()];
        let id = store::next_id(list);
        list.push(Note { id, title: "New note".into(), ..Note::default() });
        ui.selected = Some(list.len() - 1);
        let snapshot = list.clone();
        ui.sync(&snapshot);
        ui.body_focus = false;
        ui.widgets.set_focus(host, Some(ui.title));
        st.save(host, scope);
    } else if id == ui.pin {
        if let Some(i) = ui.selected
            && let Some(n) = tab_notes(st, ui).get_mut(i)
        {
            n.pinned = !n.pinned;
            st.save(host, ui.tab);
        }
    } else if id == ui.delete {
        if ui.selected.is_some() {
            if was_confirming {
                let i = ui.selected.take().unwrap_or(0);
                let list = tab_notes(st, ui);
                if i < list.len() {
                    list.remove(i);
                }
                let snapshot = list.clone();
                st.save(host, ui.tab);
                ui.sync(&snapshot);
            } else {
                ui.confirm_delete = true;
            }
        }
    } else if id == ui.close {
        st.close = true;
    }
}

fn body_key(st: &mut State, ui: &mut Ui, host: &dyn ClientHost, key: &str, mods: u32) -> bool {
    let (shift, ctrl) = (mods & 1 != 0, mods & 2 != 0);
    let edit = |ui: &mut Ui, k: EditKey| ui.body.key(k, ctrl, shift);
    let changed = match key {
        "key.keyboard.backspace" => edit(ui, EditKey::Backspace),
        "key.keyboard.delete" => edit(ui, EditKey::Delete),
        "key.keyboard.left" => edit(ui, EditKey::Left),
        "key.keyboard.right" => edit(ui, EditKey::Right),
        "key.keyboard.home" => edit(ui, EditKey::Home),
        "key.keyboard.end" => edit(ui, EditKey::End),
        "key.keyboard.a" if ctrl => edit(ui, EditKey::SelectAll),
        "key.keyboard.c" if ctrl => {
            host.set_clipboard_text(&ui.body.selected_text());
            false
        }
        "key.keyboard.x" if ctrl => {
            host.set_clipboard_text(&ui.body.selected_text());
            ui.body.insert_text("");
            true
        }
        "key.keyboard.v" if ctrl => match host.clipboard_text() {
            Some(text) => {
                ui.body.insert_text(&text);
                true
            }
            None => false,
        },
        "key.keyboard.page_up" | "key.keyboard.page_down" => {
            let dir = if key.ends_with("up") { -1 } else { 1 };
            let mut pos = ui.body.cursor;
            for _ in 0..ui.body_visible.max(1) {
                pos = layout::vertical(&ui.lines, pos, dir);
            }
            ui.body.set_cursor(pos, shift);
            ui.follow = true;
            return true;
        }
        "key.keyboard.up" | "key.keyboard.down" => {
            let dir = if key.ends_with("up") { -1 } else { 1 };
            let pos = layout::vertical(&ui.lines, ui.body.cursor, dir);
            ui.body.set_cursor(pos, shift);
            ui.follow = true;
            return true;
        }
        "key.keyboard.enter" | "key.keyboard.keypad.enter" => {
            ui.body.insert_text("\n");
            true
        }
        _ => return false,
    };
    ui.follow = true;
    if changed {
        commit_body(st, ui, host);
    }
    true
}

#[mod_api::horizon_mod]
impl Hooks for Notes {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_key_mapping(
            KeyRegistration::new(KEY, DEFAULT_KEY)
                .label("key.notes.open")
                .context(KeyConflictContext::InGame),
        );
    }

    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        event.layers.push(GuiLayer {
            id: HUD_LAYER.into(),
            placement: GuiLayerPlacement::Above("minecraft:hotbar".into()),
        });
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
        let down = host.is_down(ID, KEY);
        let (pressed, close) = {
            let mut st = self.state();
            st.refresh_context(host);
            let pressed = down && !st.key_was_down;
            st.key_was_down = down;
            (pressed, std::mem::take(&mut st.close))
        };
        if close {
            host.mod_screen_close(ID);
        }
        if pressed && host.playing() && host.mod_screen().is_none() {
            self.state().ui = None;
            host.mod_screen_open(ID);
        }
    }

    fn gui_layer_draw(&self, _: &dyn ClientHost, event: &mut GuiLayerDrawEvent<'_>) {
        if event.layer != HUD_LAYER {
            return;
        }
        let st = self.state();
        let [w, _] = event.draw.size();
        let box_w = 150.0f32.min(w / 3.0);
        let mut y = 4.0;
        let pinned = [Scope::Global, Scope::World, Scope::Server]
            .into_iter()
            .flat_map(|s| st.notes[s.index()].iter())
            .filter(|n| n.pinned)
            .take(3);
        for n in pinned {
            let widths = |c: char| event.draw.text_width(&c.to_string());
            let mut rows = vec![(n.title.clone(), GOLD)];
            for l in layout::wrap(&n.body, box_w - 8.0, &widths).into_iter().take(6) {
                let text: String = n.body.chars().skip(l.start).take(l.end - l.start).collect();
                rows.push((text, WHITE));
            }
            let h = rows.len() as f32 * LINE_H + 6.0;
            event.draw.fill([4.0, y, box_w, h], 0x6000_0000);
            for (i, (t, c)) in rows.iter().enumerate() {
                event.draw.text(t, [8.0, y + 3.0 + i as f32 * LINE_H], *c, true);
            }
            y += h + 3.0;
        }
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        if host.mod_screen() != Some(ID) {
            return;
        }
        let [w, h] = event.draw.size();
        let mut st = self.state();
        let Some(mut ui) = st.ui.take().or_else(|| {
            let tab = st.context.as_ref().map_or(Scope::Global, |c| c.scope);
            let mut ui = Ui::new(tab);
            ui.sync(&st.notes[tab.index()].clone());
            Some(ui)
        }) else {
            return;
        };
        let [px, py, pw, ph] = ui.place(w, h, &st);
        let mouse = event.screen.mouse;
        let d = &mut *event.draw;
        // Frame, shadow and header band.
        d.fill([px + 3.0, py + 3.0, pw, ph], 0x5000_0000);
        d.fill([px - 1.0, py - 1.0, pw + 2.0, ph + 2.0], 0xFF6B_5A30);
        d.fill([px, py, pw, ph], 0xF21A_1814);
        d.fill_gradient([px, py, pw, 17.0], 0xFF3A_3220, 0xFF26_2216);
        d.fill([px, py + 17.0, pw, 1.0], 0xFF6B_5A30);
        d.text("Notes", [px + 6.0, py + 5.0], GOLD, true);
        let scope_name = match ui.tab {
            Scope::World => "WORLD",
            Scope::Server => "SERVER",
            Scope::Global => "GLOBAL",
        };
        let count = st.notes[ui.tab.index()].len();
        let head = format!("{scope_name}: {count} note{}", if count == 1 { "" } else { "s" });
        d.text(&head, [px + 42.0, py + 5.0], GREY, false);
        let right = if let Some(e) = &st.error {
            Some((e.clone(), RED))
        } else if ui.selected.is_some() {
            let n = ui.body.value.chars().count();
            Some((format!("{n}/{BODY_MAX}"), if n * 10 >= BODY_MAX * 9 { RED } else { GREY }))
        } else {
            None
        };
        if let Some((text, color)) = right {
            let tw = d.text_width(&text);
            d.text(&text, [px + pw - tw - 6.0, py + 5.0], color, false);
        }
        // The list.
        let lr = ui.list_rect;
        d.fill([lr[0] - 1.0, lr[1] - 1.0, lr[2] + 2.0, lr[3] + 2.0], 0xFF3A_362C);
        d.fill(lr, 0xFF0E_0D0A);
        let list = &st.notes[ui.tab.index()];
        let rows = ((lr[3] / ROW_H) as usize).max(1);
        ui.list_rows = rows;
        if let Some(sel) = ui.selected {
            if sel < ui.list_scroll {
                ui.list_scroll = sel;
            } else if sel >= ui.list_scroll + rows {
                ui.list_scroll = sel + 1 - rows;
            }
        }
        ui.list_scroll = ui.list_scroll.min(list.len().saturating_sub(rows));
        let _ = d.push_clip(lr);
        for (i, n) in list.iter().enumerate().skip(ui.list_scroll).take(rows) {
            let y = lr[1] + (i - ui.list_scroll) as f32 * ROW_H;
            let row = [lr[0], y, lr[2], ROW_H];
            if ui.selected == Some(i) {
                d.fill(row, 0xFF3A_3118);
                d.fill([lr[0], y, 2.0, ROW_H], GOLD);
            } else if contains(row, mouse) {
                d.fill(row, 0x30FF_FFFF);
            }
            let name = if n.title.is_empty() { "(untitled)" } else { n.title.as_str() };
            let label = if n.pinned { format!("* {name}") } else { name.to_owned() };
            d.text(&label, [lr[0] + 5.0, y + 1.5], if n.pinned { GOLD } else { WHITE }, false);
        }
        let _ = d.pop_clip();
        if list.is_empty() {
            d.text("No notes yet", [lr[0] + 5.0, lr[1] + 4.0], GREY, false);
            d.text("Press New to start", [lr[0] + 5.0, lr[1] + 4.0 + LINE_H], GREY, false);
        }
        scrollbar(d, lr, ui.list_scroll, list.len(), rows);
        // The body.
        let br = ui.body_rect;
        let focused = ui.body_focus;
        d.fill(
            [br[0] - 1.0, br[1] - 1.0, br[2] + 2.0, br[3] + 2.0],
            if focused { 0xFFB8_A050 } else { 0xFF3A_362C },
        );
        d.fill(br, 0xFF0A_0907);
        if ui.selected.is_some() {
            let widths = |c: char| d.text_width(&c.to_string());
            ui.lines = layout::wrap(&ui.body.value, br[2] - 14.0, &widths);
            let visible = ((br[3] - 4.0) / LINE_H).max(1.0) as usize;
            ui.body_visible = visible;
            let (row, col) = layout::locate(&ui.lines, ui.body.cursor);
            if ui.follow {
                if row < ui.body_scroll {
                    ui.body_scroll = row;
                } else if row >= ui.body_scroll + visible {
                    ui.body_scroll = row + 1 - visible;
                }
                ui.follow = false;
            }
            ui.body_scroll = ui.body_scroll.min(ui.lines.len().saturating_sub(visible));
            let (a, b) = ui.body.selected();
            let _ = d.push_clip(br);
            for (r, l) in ui.lines.iter().enumerate().skip(ui.body_scroll).take(visible + 1) {
                let y = br[1] + 2.0 + (r - ui.body_scroll) as f32 * LINE_H;
                let text: String = ui.body.value.chars().skip(l.start).take(l.end - l.start).collect();
                let (s, e) = (a.max(l.start), b.min(l.end));
                if s < e {
                    let (x0, x1) = (l.edges[s - l.start], l.edges[e - l.start]);
                    d.fill([br[0] + 4.0 + x0, y - 1.0, x1 - x0, LINE_H], 0xFF30_60C0);
                }
                d.text(&text, [br[0] + 4.0, y], WHITE, false);
            }
            if focused && (host.millis() / 400) % 2 == 0 && row >= ui.body_scroll {
                let l = &ui.lines[row];
                let x = br[0] + 4.0 + l.edges.get(col).copied().unwrap_or(0.0);
                let y = br[1] + 2.0 + (row - ui.body_scroll) as f32 * LINE_H;
                d.fill([x, y - 1.0, 1.0, LINE_H], WHITE);
            }
            let _ = d.pop_clip();
            scrollbar(d, br, ui.body_scroll, ui.lines.len(), visible);
        } else {
            d.text("Select a note or press New", [br[0] + 6.0, br[1] + 6.0], GREY, false);
        }
        // Button labels that change.
        let pinned = ui.selected.and_then(|i| list.get(i)).is_some_and(|n| n.pinned);
        let has = ui.selected.is_some();
        let confirm = ui.confirm_delete;
        if let Some(b) = ui.widgets.button_mut(ui.pin) {
            b.label = Text::literal(if pinned { "Unpin" } else { "Pin" });
            b.active = has;
        }
        if let Some(b) = ui.widgets.button_mut(ui.delete) {
            b.label = Text::literal(if confirm { "Confirm?" } else { "Delete" });
            b.active = has;
        }
        let _ = ui.widgets.draw(&mut *event.draw, host, event.screen.mouse);
        for (i, t) in ui.tabs.iter().enumerate() {
            if Scope::ALL[i] != ui.tab {
                continue;
            }
            if let Some(rect) = ui.widgets.button(*t).map(|b| b.rect) {
                let d = &mut *event.draw;
                d.fill(rect, 0xFF6B_5A22);
                d.fill([rect[0], rect[1] + rect[3] - 2.0, rect[2], 2.0], GOLD);
                let name = ["WORLD", "SERVER", "GLOBAL"][i];
                let tw = d.text_width(name);
                d.text(name, [rect[0] + (rect[2] - tw) / 2.0, rect[1] + 4.0], WHITE, false);
            }
        }
        st.ui = Some(ui);
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        let (at, button, mods) = (event.screen.mouse, event.button, event.modifiers);
        self.input(host, |st, ui| {
            let outcome = ui.widgets.mouse_pressed(host, at, button, mods);
            match outcome {
                Outcome::Pressed(id) => {
                    press(st, ui, host, id);
                    return true;
                }
                Outcome::Ignored => {}
                _ => {
                    ui.body_focus = false;
                    return true;
                }
            }
            if button != 0 {
                return false;
            }
            if contains(ui.list_rect, at) {
                let row = ((at[1] - ui.list_rect[1]) / ROW_H) as usize + ui.list_scroll;
                let list = st.notes[ui.tab.index()].clone();
                if row < list.len() {
                    ui.selected = Some(row);
                    ui.sync(&list);
                    ui.body_focus = false;
                    ui.widgets.set_focus(host, None);
                }
                return true;
            }
            if contains(ui.body_rect, at) && ui.selected.is_some() {
                ui.widgets.set_focus(host, None);
                ui.body_focus = true;
                let row = (((at[1] - ui.body_rect[1] - 2.0) / LINE_H).max(0.0) as usize
                    + ui.body_scroll)
                    .min(ui.lines.len().saturating_sub(1));
                if let Some(l) = ui.lines.get(row) {
                    let col = layout::column_at(l, at[0] - ui.body_rect[0] - 4.0);
                    ui.body.set_cursor(l.start + col, false);
                }
                return true;
            }
            false
        })
    }

    fn screen_mouse_scrolled_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseScrolledPreEvent<'_>,
    ) -> Flow {
        let (at, dy) = (event.screen.mouse, event.scroll[1]);
        self.input(host, |st, ui| {
            if dy == 0.0 {
                return false;
            }
            let notches = dy.abs().ceil().max(1.0) as isize;
            let delta = if dy > 0.0 { -notches } else { notches };
            let shift = |now: usize, by: isize, max: usize| {
                (now as isize + by).clamp(0, max as isize) as usize
            };
            if contains(ui.list_rect, at) {
                let max = st.notes[ui.tab.index()].len().saturating_sub(ui.list_rows);
                ui.list_scroll = shift(ui.list_scroll, delta, max);
                true
            } else if contains(ui.body_rect, at) && ui.selected.is_some() {
                let max = ui.lines.len().saturating_sub(ui.body_visible);
                ui.body_scroll = shift(ui.body_scroll, delta * 3, max);
                ui.follow = false;
                true
            } else {
                false
            }
        })
    }

    fn screen_mouse_button_released_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
    ) -> Flow {
        let button = event.button;
        self.input(host, |_, ui| ui.widgets.mouse_released(button).handled())
    }

    fn screen_mouse_dragged_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseDraggedPreEvent<'_>,
    ) -> Flow {
        let (at, button) = (event.screen.mouse, event.button);
        self.input(host, |_, ui| {
            if ui.body_focus && button == 0 && contains(ui.body_rect, at) {
                let row = (((at[1] - ui.body_rect[1] - 2.0) / LINE_H).max(0.0) as usize
                    + ui.body_scroll)
                    .min(ui.lines.len().saturating_sub(1));
                if let Some(l) = ui.lines.get(row) {
                    let col = layout::column_at(l, at[0] - ui.body_rect[0] - 4.0);
                    ui.body.set_cursor(l.start + col, true);
                }
                return true;
            }
            ui.widgets.mouse_dragged(host, at, button).handled()
        })
    }

    fn screen_key_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        let (key, mods) = (event.key, event.modifiers);
        self.input(host, |st, ui| {
            if key == "key.keyboard.escape" {
                return false;
            }
            if key == "key.keyboard.tab" && ui.selected.is_some() {
                if ui.body_focus {
                    ui.body_focus = false;
                    ui.widgets.set_focus(host, Some(ui.title));
                } else {
                    ui.body_focus = true;
                    ui.widgets.set_focus(host, None);
                }
                return true;
            }
            if ui.body_focus && ui.selected.is_some() {
                return body_key(st, ui, host, key, mods);
            }
            match ui.widgets.key_pressed(host, key, mods) {
                Outcome::Changed(id) if id == ui.title => {
                    commit_title(st, ui, host);
                    true
                }
                Outcome::Submitted(id) if id == ui.title => {
                    ui.body_focus = ui.selected.is_some();
                    if ui.body_focus {
                        ui.widgets.set_focus(host, None);
                    }
                    true
                }
                Outcome::Pressed(id) => {
                    press(st, ui, host, id);
                    true
                }
                other => other.handled(),
            }
        })
    }

    fn screen_char_typed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenCharTypedPreEvent<'_>,
    ) -> Flow {
        let c = event.c;
        self.input(host, |st, ui| {
            if ui.body_focus && ui.selected.is_some() {
                ui.body.insert_text(&c.to_string());
                commit_body(st, ui, host);
                return true;
            }
            match ui.widgets.char_typed(host, c) {
                Outcome::Changed(id) if id == ui.title => {
                    commit_title(st, ui, host);
                    true
                }
                other => other.handled(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_is_half_open() {
        let r = [10.0, 10.0, 5.0, 5.0];
        assert!(contains(r, [10.0, 10.0]));
        assert!(!contains(r, [15.0, 12.0]));
    }
}
