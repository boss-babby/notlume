//! The block editor.
//!
//! egui is immediate-mode, so a single document-wide text field with inline
//! rich text is not possible. Instead every block owns a `TextEdit` whose
//! galley is produced by a custom layouter that applies the block's marks.
//! Rich-text semantics (marks following edits, splitting, merging) live in
//! `nw_core::RichText`. Off-screen top-level blocks are skipped using cached
//! heights, so a 10 000 block page only lays out what is visible.

use crate::anim::{self, Spring};
use crate::fonts;
use crate::syntax;
use crate::theme::Palette;
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{pos2, vec2, Align2, Color32, Context, FontId, Id, Key, Modifiers, Rect, Response, Sense, Stroke, Ui};
use nw_core::block::{self, Block, BlockKind};
use nw_core::markdown::detect_shortcut;
use nw_core::page::mention_text;
use nw_core::richtext::{Mark, MarkKind};
use nw_core::search::fuzzy_score;
use nw_core::Uuid;
use std::collections::HashMap;

const GUTTER: f32 = 48.0;

#[derive(Clone, Copy)]
struct BlockDrag(Uuid);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuKind {
    Slash,
    Mention,
}

#[derive(Clone, Debug)]
pub struct MenuState {
    pub kind: MenuKind,
    pub block: Uuid,
    pub anchor: egui::Pos2,
    pub filter: String,
    /// Char index of the `@` that opened a mention menu.
    pub trigger: usize,
    pub sel: usize,
    pub pick: bool,
    pub moved: bool,
    pub rect: Option<Rect>,
}

pub struct EditorState {
    pub focus: Option<(Uuid, usize)>,
    pub heights: HashMap<Uuid, f32>,
    pub menu: Option<MenuState>,
    pub dismissed: Option<Uuid>,
    pub drop_spring: Spring,
    pub drop_active: bool,
    pub open_page: Option<Uuid>,
    pub mention_pages: Vec<(Uuid, String)>,
    pub tombstones: Vec<Uuid>,
    url_buf: HashMap<Uuid, String>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            focus: None,
            heights: HashMap::new(),
            menu: None,
            dismissed: None,
            drop_spring: Spring::new(0.0),
            drop_active: false,
            open_page: None,
            mention_pages: Vec::new(),
            tombstones: Vec::new(),
            url_buf: HashMap::new(),
        }
    }
}

impl EditorState {
    /// Forget per-page caches (call when navigating).
    pub fn reset(&mut self) {
        self.heights.clear();
        self.menu = None;
        self.focus = None;
        self.dismissed = None;
        self.drop_active = false;
        self.url_buf.clear();
    }

    fn title_of(&self, id: Uuid) -> Option<&str> {
        self.mention_pages.iter().find(|(p, _)| *p == id).map(|(_, t)| t.as_str())
    }
}

fn tid(block: Uuid) -> Id {
    Id::new(("blk-text", block))
}

enum Op {
    AddAfter(usize),
    Split { idx: usize, at: usize },
    MergePrev(usize),
    Delete(usize),
    Move { from: Uuid, to: usize },
    FocusPrev(usize),
    FocusNext(usize),
}

struct Pass<'a> {
    pal: &'a Palette,
    depth: usize,
    dragging: Option<Uuid>,
    drop_hint: Option<f32>,
    op: Option<Op>,
}

#[derive(Clone, Copy)]
struct EditOpts {
    size: f32,
    bold: bool,
    italic: bool,
    strike: bool,
    color: Color32,
    hint: &'static str,
}

const SLASH_ITEMS: &[(&str, &str, BlockKind)] = &[
    ("Text", "Plain paragraph", BlockKind::Paragraph),
    ("Heading 1", "# ", BlockKind::Heading(1)),
    ("Heading 2", "## ", BlockKind::Heading(2)),
    ("Heading 3", "### ", BlockKind::Heading(3)),
    ("Bulleted list", "- ", BlockKind::Bulleted),
    ("Numbered list", "1. ", BlockKind::Numbered),
    ("To-do list", "[] ", BlockKind::Todo),
    ("Toggle", "Collapsible", BlockKind::Toggle),
    ("Quote", "> ", BlockKind::Quote),
    ("Callout", "Highlighted note", BlockKind::Callout),
    ("Code", "```", BlockKind::Code),
    ("Divider", "---", BlockKind::Divider),
    ("Image", "From URL", BlockKind::Image),
    ("Table", "Simple grid", BlockKind::Table),
];

enum Choice {
    Kind(BlockKind),
    Page(Uuid, String),
}

fn menu_items(m: &MenuState, pages: &[(Uuid, String)]) -> Vec<(String, String, Choice)> {
    match m.kind {
        MenuKind::Slash => SLASH_ITEMS
            .iter()
            .filter(|(l, _, _)| fuzzy_score(&m.filter, l).is_some())
            .map(|(l, h, k)| ((*l).to_owned(), (*h).to_owned(), Choice::Kind(*k)))
            .collect(),
        MenuKind::Mention => {
            let mut v: Vec<(i32, &(Uuid, String))> = pages
                .iter()
                .filter_map(|p| fuzzy_score(&m.filter, &p.1).map(|s| (s, p)))
                .collect();
            v.sort_by(|a, b| b.0.cmp(&a.0));
            v.into_iter()
                .take(12)
                .map(|(_, (id, t))| (t.clone(), "Link to page".to_owned(), Choice::Page(*id, t.clone())))
                .collect()
        }
    }
}

// ---------------------------------------------------------------- public API

/// Show a page body. Returns `true` if the content changed.
pub fn show_page_blocks(ui: &mut Ui, blocks: &mut Vec<Block>, st: &mut EditorState, pal: &Palette) -> bool {
    let mut changed = false;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        changed |= show_blocks(ui, blocks, st, pal, 0);

        // Clickable tail: add a block at the end.
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::click());
        if resp.hovered() {
            ui.painter().text(
                pos2(rect.left() + GUTTER, rect.top() + 14.0),
                Align2::LEFT_CENTER,
                "Click to add a block",
                FontId::proportional(13.0),
                pal.dim,
            );
        }
        if resp.clicked() {
            match blocks.last() {
                Some(l) if l.kind == BlockKind::Paragraph && l.text.is_empty() => st.focus = Some((l.id, 0)),
                _ => {
                    let nb = Block::new(BlockKind::Paragraph);
                    st.focus = Some((nb.id, 0));
                    blocks.push(nb);
                    changed = true;
                }
            }
        }
    });
    changed |= show_menu(ui.ctx(), blocks, st, pal);
    changed
}

fn show_blocks(ui: &mut Ui, blocks: &mut Vec<Block>, st: &mut EditorState, pal: &Palette, depth: usize) -> bool {
    let ctx = ui.ctx().clone();
    let pending_focus = st.focus;
    let dragging = egui::DragAndDrop::payload::<BlockDrag>(&ctx)
        .map(|p| p.0)
        .filter(|d| blocks.iter().any(|b| b.id == *d));
    let mut numbers = Vec::with_capacity(blocks.len());
    let mut counter = 0;
    for b in blocks.iter() {
        counter = if b.kind == BlockKind::Numbered { counter + 1 } else { 0 };
        numbers.push(counter);
    }
    let mut pass = Pass {
        pal,
        depth,
        dragging,
        drop_hint: None,
        op: None,
    };
    let mut changed = false;
    let width = ui.available_width();

    for i in 0..blocks.len() {
        let id = blocks[i].id;
        if depth == 0 {
            if let Some(&h) = st.heights.get(&id) {
                let rect = Rect::from_min_size(ui.cursor().min, vec2(width, h));
                let needed = st.focus.is_some_and(|(f, _)| f == id)
                    || ctx.memory(|m| m.has_focus(tid(id)))
                    || dragging.is_some()
                    || st.menu.as_ref().is_some_and(|m| m.block == id);
                if !needed && !ui.clip_rect().intersects(rect) {
                    ui.allocate_space(vec2(width, h));
                    continue;
                }
            }
        }
        let y0 = ui.cursor().min.y;
        changed |= row_ui(ui, &mut blocks[i], i, numbers[i], st, &mut pass);
        if depth == 0 {
            st.heights.insert(id, ui.cursor().min.y - y0);
        }
    }

    if let Some(op) = pass.op.take() {
        changed |= apply_op(op, blocks, st, depth);
        ctx.request_repaint();
    }
    if st.focus == pending_focus && pending_focus.is_some() {
        st.focus = None; // target was not on this level / never rendered
    }

    // Drop indicator with spring snapping.
    if let (Some(y), Some(_)) = (pass.drop_hint, dragging) {
        if !st.drop_active {
            st.drop_spring.snap(y);
            st.drop_active = true;
        }
        st.drop_spring.target = y;
        let dt = ctx.input(|i| i.stable_dt);
        let reduce = ctx.style().animation_time <= 0.0;
        if reduce {
            st.drop_spring.snap(y);
        } else if st.drop_spring.step(dt) {
            ctx.request_repaint();
        }
        let x0 = ui.min_rect().left() + GUTTER;
        let r = Rect::from_min_max(pos2(x0, st.drop_spring.pos - 1.5), pos2(x0 + width - GUTTER, st.drop_spring.pos + 1.5));
        ui.painter().rect_filled(r, 1.5, pal.accent);
        ui.painter().circle_filled(r.left_center(), 4.0, pal.accent);
    } else if dragging.is_none() {
        st.drop_active = false;
    }

    // Ghost preview following the pointer.
    if depth == 0 {
        if let (Some(d), Some(pos)) = (dragging, ctx.pointer_interact_pos()) {
            if let Some(b) = block::find(blocks, d) {
                let mut label: String = b.text.text.chars().take(60).collect();
                if label.is_empty() {
                    label = format!("{:?}", b.kind);
                }
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                egui::Area::new(Id::new("block-drag-ghost"))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(pos + vec2(14.0, 10.0))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_opacity(0.9);
                            ui.set_max_width(320.0);
                            ui.label(label);
                        });
                    });
            }
        }
    }
    changed
}

fn row_ui(ui: &mut Ui, b: &mut Block, i: usize, number: usize, st: &mut EditorState, pass: &mut Pass<'_>) -> bool {
    let ctx = ui.ctx().clone();
    let pal = pass.pal;
    let row_top = ui.cursor().min;
    let avail = ui.available_width();
    let id = b.id;
    let is_dragged = pass.dragging == Some(id);
    let mut changed = false;

    let inner = ui.scope(|ui| {
        if is_dragged {
            ui.set_opacity(0.35);
        }
        ui.horizontal_top(|ui| {
            ui.add_space(GUTTER);
            ui.vertical(|ui| {
                ui.set_width((avail - GUTTER - 8.0).max(120.0));
                let top_pad = if matches!(b.kind, BlockKind::Heading(_)) { 14.0 } else { 3.0 };
                ui.add_space(top_pad);
                changed |= content(ui, b, i, number, st, pass);
                ui.add_space(3.0);
            });
        });
    });
    let row_rect = Rect::from_min_max(row_top, pos2(row_top.x + avail, inner.response.rect.bottom()));

    // Hover handles (+ and grip) fade in.
    let hover = ui.rect_contains_pointer(row_rect) || is_dragged;
    let a = anim::ease_bool(&ctx, Id::new(("handles", id)), hover, anim::FAST);
    if a > 0.02 {
        let y = row_top.y + if matches!(b.kind, BlockKind::Heading(_)) { 14.0 } else { 3.0 };
        let plus = Rect::from_min_size(pos2(row_top.x + 2.0, y), vec2(22.0, 24.0));
        let grip = Rect::from_min_size(pos2(row_top.x + 24.0, y), vec2(22.0, 24.0));
        let pr = ui.interact(plus, Id::new(("plus", id)), Sense::click());
        let gr = ui.interact(grip, Id::new(("grip", id)), Sense::click_and_drag());
        for (r, resp, glyph) in [(plus, &pr, "+"), (grip, &gr, "⠿")] {
            if resp.hovered() || resp.dragged() {
                ui.painter().rect_filled(r, 4.0, pal.hover.gamma_multiply(a));
            }
            ui.painter()
                .text(r.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(15.0), pal.dim.gamma_multiply(a));
        }
        if pr.clicked() {
            pass.op = Some(Op::AddAfter(i));
        }
        if gr.drag_started() {
            gr.dnd_set_drag_payload(BlockDrag(id));
        }
        if gr.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
        }
    }

    // Drop target.
    if let Some(d) = pass.dragging {
        if d != id {
            if let Some(p) = ctx.pointer_latest_pos() {
                if row_rect.contains(p) {
                    let before = p.y < row_rect.center().y;
                    pass.drop_hint = Some(if before { row_rect.top() } else { row_rect.bottom() });
                    if ctx.input(|inp| inp.pointer.any_released()) {
                        pass.op = Some(Op::Move {
                            from: d,
                            to: if before { i } else { i + 1 },
                        });
                    }
                }
            }
        }
    }

    // Page-link chips for @mentions.
    let links = b.text.linked_pages();
    if !links.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(GUTTER);
            for l in links {
                if let Some(t) = st.title_of(l) {
                    if ui.small_button(format!("↗ {t}")).clicked() {
                        st.open_page = Some(l);
                    }
                }
            }
        });
    }
    changed
}

fn content(ui: &mut Ui, b: &mut Block, i: usize, number: usize, st: &mut EditorState, pass: &mut Pass<'_>) -> bool {
    let pal = pass.pal;
    let mut changed = false;
    let base = EditOpts {
        size: 16.0,
        bold: false,
        italic: false,
        strike: false,
        color: pal.text,
        hint: "Type '/' for commands",
    };
    match b.kind {
        BlockKind::Paragraph => changed |= rich_edit(ui, b, i, st, pass, base).1,
        BlockKind::Heading(n) => {
            let size = match n {
                1 => 32.0,
                2 => 25.0,
                _ => 20.0,
            };
            let hint = match n {
                1 => "Heading 1",
                2 => "Heading 2",
                _ => "Heading 3",
            };
            changed |= rich_edit(ui, b, i, st, pass, EditOpts { size, bold: true, hint, ..base }).1;
        }
        BlockKind::Bulleted | BlockKind::Numbered | BlockKind::Todo => {
            ui.horizontal_top(|ui| {
                match b.kind {
                    BlockKind::Bulleted => {
                        ui.add_sized([22.0, 22.0], egui::Label::new(egui::RichText::new("•").size(18.0)));
                    }
                    BlockKind::Numbered => {
                        ui.add_sized([22.0, 22.0], egui::Label::new(format!("{number}.")));
                    }
                    _ => {
                        let r = ui.add(egui::Checkbox::without_text(&mut b.checked));
                        if r.changed() {
                            b.touch();
                            changed = true;
                        }
                    }
                }
                let checked = b.kind == BlockKind::Todo && b.checked;
                let opts = EditOpts {
                    strike: checked,
                    color: if checked { pal.dim } else { pal.text },
                    hint: "List",
                    ..base
                };
                changed |= rich_edit(ui, b, i, st, pass, opts).1;
            });
        }
        BlockKind::Quote => {
            let top = ui.cursor().min;
            let left = ui.min_rect().left();
            ui.horizontal_top(|ui| {
                ui.add_space(14.0);
                changed |= rich_edit(ui, b, i, st, pass, EditOpts { hint: "Quote", size: 17.0, ..base }).1;
            });
            let bottom = ui.cursor().min.y;
            ui.painter()
                .rect_filled(Rect::from_min_max(pos2(left, top.y), pos2(left + 3.0, bottom)), 1.5, pal.text);
        }
        BlockKind::Callout => {
            egui::Frame::new()
                .fill(pal.callout_bg)
                .corner_radius(8)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut b.emoji)
                                .frame(false)
                                .desired_width(28.0)
                                .font(FontId::proportional(20.0)),
                        );
                        if r.changed() {
                            b.emoji = b.emoji.chars().take(2).collect();
                            b.touch();
                            changed = true;
                        }
                        changed |= rich_edit(ui, b, i, st, pass, EditOpts { hint: "Callout", ..base }).1;
                    });
                });
        }
        BlockKind::Toggle => {
            let cid = ui.make_persistent_id(("toggle", b.id));
            let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), cid, true);
            let mut head_changed = false;
            let header = state.show_header(ui, |ui| {
                head_changed = rich_edit(ui, b, i, st, pass, EditOpts { hint: "Toggle", ..base }).1;
            });
            changed |= head_changed;
            let mut child_changed = false;
            header.body(|ui| {
                ui.add_space(2.0);
                if b.children.is_empty() {
                    if ui.small_button("+ Add a block inside").clicked() {
                        b.children.push(Block::new(BlockKind::Paragraph));
                        child_changed = true;
                    }
                } else {
                    child_changed |= show_blocks(ui, &mut b.children, st, pal, pass.depth + 1);
                }
            });
            if child_changed {
                b.touch();
                changed = true;
            }
        }
        BlockKind::Divider => {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
            ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, pal.border));
        }
        BlockKind::Code => changed |= code_block(ui, b, i, st, pass),
        BlockKind::Image => changed |= image_block(ui, b, st, pal),
        BlockKind::Table => {
            let cols = b.table.first().map_or(0, Vec::len);
            let rows = b.table.len();
            egui::Grid::new(("table", b.id)).spacing([4.0, 4.0]).show(ui, |ui| {
                for r in 0..rows {
                    for c in 0..cols.min(b.table[r].len()) {
                        let resp = ui.add(egui::TextEdit::singleline(&mut b.table[r][c]).desired_width(130.0));
                        changed |= resp.changed();
                    }
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                if ui.small_button("+ Row").clicked() {
                    b.table.push(vec![String::new(); cols.max(1)]);
                    changed = true;
                }
                if ui.small_button("+ Column").clicked() {
                    for r in &mut b.table {
                        r.push(String::new());
                    }
                    changed = true;
                }
                if ui.small_button("Delete table").clicked() {
                    pass.op = Some(Op::Delete(i));
                }
            });
            if changed {
                b.touch();
            }
        }
    }
    changed
}

fn image_block(ui: &mut Ui, b: &mut Block, st: &mut EditorState, pal: &Palette) -> bool {
    let mut changed = false;
    if b.url.is_empty() {
        let buf = st.url_buf.entry(b.id).or_default();
        egui::Frame::new().fill(pal.callout_bg).corner_radius(8).inner_margin(12).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("🖼");
                let r = ui.add(
                    egui::TextEdit::singleline(buf)
                        .hint_text("Paste an image URL, then press Enter")
                        .desired_width(ui.available_width() - 80.0),
                );
                let commit = (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) || ui.button("Embed").clicked();
                if commit && !buf.trim().is_empty() {
                    b.url = buf.trim().to_owned();
                    b.touch();
                    changed = true;
                }
            });
        });
    } else {
        let resp = ui.add(egui::Image::new(b.url.clone()).max_width(ui.available_width()));
        resp.context_menu(|ui| {
            if ui.button("Replace image").clicked() {
                b.url.clear();
                b.touch();
                changed = true;
                ui.close_menu();
            }
        });
    }
    changed
}

fn code_block(ui: &mut Ui, b: &mut Block, i: usize, st: &mut EditorState, pass: &mut Pass<'_>) -> bool {
    let pal = pass.pal;
    let mut changed = false;
    let id = tid(b.id);
    if let Some((f, _)) = st.focus {
        if f == b.id {
            ui.ctx().memory_mut(|m| m.request_focus(id));
            st.focus = None;
        }
    }
    if ui.ctx().memory(|m| m.has_focus(id))
        && b.text.is_empty()
        && ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Backspace))
    {
        pass.op = Some(Op::Delete(i));
    }
    egui::Frame::new().fill(pal.code_bg).corner_radius(8).inner_margin(12).show(ui, |ui| {
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut b.language)
                    .frame(false)
                    .desired_width(90.0)
                    .text_color(pal.dim)
                    .font(FontId::proportional(12.0)),
            );
            changed |= r.changed();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(b.text.text.clone());
                }
            });
        });
        let lang = b.language.clone();
        let mut buf = b.text.text.clone();
        let mut layouter = |ui: &Ui, text: &str, wrap: f32| {
            ui.fonts(|f| f.layout_job(syntax::highlight(text, &lang, 14.0, pal, wrap)))
        };
        let out = egui::TextEdit::multiline(&mut buf)
            .id(id)
            .frame(false)
            .code_editor()
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .hint_text("Code")
            .layouter(&mut layouter)
            .show(ui);
        if out.response.changed() {
            b.text.set_plain(&buf);
            changed = true;
        }
    });
    if changed {
        b.touch();
    }
    changed
}

fn rich_job(text: &str, marks: &[Mark], o: &EditOpts, pal: &Palette, wrap: f32) -> LayoutJob {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut cuts = vec![0, n];
    for m in marks {
        cuts.push(m.start.min(n));
        cuts.push(m.end.min(n));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    for w in cuts.windows(2) {
        let (a, z) = (w[0], w[1]);
        if a == z {
            continue;
        }
        let seg: String = chars[a..z].iter().collect();
        let (mut bold, mut italic, mut strike) = (o.bold, o.italic, o.strike);
        let (mut code, mut underline) = (false, false);
        let mut color = o.color;
        for m in marks.iter().filter(|m| m.start <= a && m.end >= z) {
            match &m.kind {
                MarkKind::Bold => bold = true,
                MarkKind::Italic => italic = true,
                MarkKind::Strike => strike = true,
                MarkKind::Code => code = true,
                MarkKind::Color(c) => color = Color32::from_rgb(c[0], c[1], c[2]),
                MarkKind::Link(_) => {
                    color = pal.link;
                    underline = true;
                }
            }
        }
        let font = if code {
            FontId::monospace(o.size * 0.88)
        } else if bold {
            fonts::bold(o.size)
        } else {
            FontId::proportional(o.size)
        };
        job.append(
            &seg,
            0.0,
            TextFormat {
                font_id: font,
                color,
                background: if code { pal.code_bg } else { Color32::TRANSPARENT },
                italics: italic,
                underline: if underline { Stroke::new(1.0, color) } else { Stroke::NONE },
                strikethrough: if strike { Stroke::new(1.0, color) } else { Stroke::NONE },
                ..Default::default()
            },
        );
    }
    if job.sections.is_empty() {
        job.append("", 0.0, TextFormat::simple(FontId::proportional(o.size), o.color));
    }
    job
}

fn char_index_to_byte(s: &str, idx: usize) -> usize {
    s.char_indices().nth(idx).map_or(s.len(), |(b, _)| b)
}

/// One rich-text field. Returns the response and whether the content changed.
fn rich_edit(ui: &mut Ui, b: &mut Block, i: usize, st: &mut EditorState, pass: &mut Pass<'_>, o: EditOpts) -> (Response, bool) {
    let ctx = ui.ctx().clone();
    let pal = pass.pal;
    let id = tid(b.id);
    let mut applied_focus = false;

    if let Some((fid, pos)) = st.focus {
        if fid == b.id {
            ctx.memory_mut(|m| m.request_focus(id));
            let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
            let p = pos.min(b.text.char_len());
            state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(p))));
            state.store(&ctx, id);
            st.focus = None;
            applied_focus = true;
        }
    }

    let focused = ctx.memory(|m| m.has_focus(id));
    let cursor = egui::TextEdit::load_state(&ctx, id).and_then(|s| s.cursor.char_range());
    let (sel_lo, sel_hi) = cursor.map_or((0, 0), |r| {
        (r.primary.index.min(r.secondary.index), r.primary.index.max(r.secondary.index))
    });
    let len = b.text.char_len();

    if focused {
        let menu_here = st.menu.as_ref().is_some_and(|m| m.block == b.id);
        let shift = ui.input(|inp| inp.modifiers.shift);
        let mut mark_toggle: Option<MarkKind> = None;
        ctx.input_mut(|inp| {
            if menu_here {
                if let Some(m) = st.menu.as_mut() {
                    if inp.consume_key(Modifiers::NONE, Key::ArrowDown) {
                        m.sel += 1;
                        m.moved = true;
                    }
                    if inp.consume_key(Modifiers::NONE, Key::ArrowUp) {
                        m.sel = m.sel.saturating_sub(1);
                        m.moved = true;
                    }
                    if inp.consume_key(Modifiers::NONE, Key::Enter) || inp.consume_key(Modifiers::NONE, Key::Tab) {
                        m.pick = true;
                    }
                    if inp.consume_key(Modifiers::NONE, Key::Escape) {
                        st.dismissed = Some(b.id);
                    }
                }
            } else {
                if !shift && inp.consume_key(Modifiers::NONE, Key::Enter) {
                    pass.op = Some(Op::Split { idx: i, at: sel_hi });
                }
                if sel_lo == sel_hi && sel_lo == 0 && inp.consume_key(Modifiers::NONE, Key::Backspace) {
                    pass.op = Some(Op::MergePrev(i));
                }
                if sel_lo == sel_hi && sel_lo == 0 && inp.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    pass.op = Some(Op::FocusPrev(i));
                }
                if sel_lo == sel_hi && sel_hi == len && inp.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    pass.op = Some(Op::FocusNext(i));
                }
            }
            if inp.consume_key(Modifiers::COMMAND, Key::B) {
                mark_toggle = Some(MarkKind::Bold);
            }
            if inp.consume_key(Modifiers::COMMAND, Key::I) {
                mark_toggle = Some(MarkKind::Italic);
            }
            if inp.consume_key(Modifiers::COMMAND, Key::E) {
                mark_toggle = Some(MarkKind::Code);
            }
            if inp.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
                mark_toggle = Some(MarkKind::Strike);
            }
            if inp.consume_key(Modifiers::COMMAND, Key::H) {
                mark_toggle = Some(MarkKind::Color([224, 62, 62]));
            }
        });
        if let Some(k) = mark_toggle {
            if sel_lo != sel_hi {
                b.text.toggle(sel_lo, sel_hi, k);
                b.touch();
            }
        }
    }

    let mut buf = b.text.text.clone();
    let marks = &b.text.marks;
    let mut layouter = |ui: &Ui, text: &str, wrap: f32| ui.fonts(|f| f.layout_job(rich_job(text, marks, &o, pal, wrap)));
    let out = egui::TextEdit::multiline(&mut buf)
        .id(id)
        .frame(false)
        .desired_rows(1)
        .desired_width(f32::INFINITY)
        .font(FontId::proportional(o.size))
        .hint_text(egui::RichText::new(o.hint).size(o.size).color(pal.dim.gamma_multiply(0.7)))
        .layouter(&mut layouter)
        .show(ui);
    let resp = out.response.clone();
    if applied_focus {
        resp.scroll_to_me(None);
    }
    let mut changed = false;
    if resp.changed() {
        b.text.set_plain(&buf);
        b.touch();
        changed = true;
        if b.kind == BlockKind::Paragraph {
            if let Some((kind, strip)) = detect_shortcut(&b.text.text) {
                let rest: String = b.text.text.chars().skip(strip).collect();
                b.text.set_plain(&rest);
                b.kind = kind;
                match kind {
                    BlockKind::Divider => pass.op = Some(Op::AddAfter(i)),
                    BlockKind::Code => b.language = "rust".into(),
                    BlockKind::Callout => b.emoji = "💡".into(),
                    BlockKind::Table => b.table = vec![vec![String::new(); 3]; 3],
                    _ => {}
                }
                if kind != BlockKind::Divider {
                    st.focus = Some((b.id, 0));
                }
            }
        }
    }

    // Slash / mention menus.
    if resp.has_focus() {
        let text = b.text.text.as_str();
        let cur = out.cursor_range.map_or(len, |r| r.primary.index);
        let before_cursor = &text[..char_index_to_byte(text, cur)];
        if !text.starts_with('/') && st.dismissed == Some(b.id) {
            st.dismissed = None;
        }
        let want = if text.starts_with('/') && !text.contains('\n') && st.dismissed != Some(b.id) {
            Some((MenuKind::Slash, text[1..].to_owned(), 0))
        } else if let Some(at) = before_cursor.rfind('@') {
            let after = &before_cursor[at + 1..];
            let boundary_ok = at == 0 || before_cursor[..at].ends_with(char::is_whitespace);
            if boundary_ok && !after.contains(char::is_whitespace) && st.dismissed != Some(b.id) {
                Some((MenuKind::Mention, after.to_owned(), before_cursor[..at].chars().count()))
            } else {
                None
            }
        } else {
            None
        };
        match want {
            Some((kind, filter, trigger)) => {
                let (sel, pick, moved) = st
                    .menu
                    .as_ref()
                    .filter(|m| m.block == b.id && m.kind == kind)
                    .map_or((0, false, false), |m| (m.sel, m.pick, m.moved));
                let reset = st.menu.as_ref().is_some_and(|m| m.filter != filter);
                st.menu = Some(MenuState {
                    kind,
                    block: b.id,
                    anchor: resp.rect.left_bottom(),
                    filter,
                    trigger,
                    sel: if reset { 0 } else { sel },
                    pick,
                    moved,
                    rect: st.menu.as_ref().and_then(|m| m.rect),
                });
            }
            None => {
                if st.menu.as_ref().is_some_and(|m| m.block == b.id) {
                    st.menu = None;
                }
            }
        }
    }
    (resp, changed)
}

fn insert_after_id(blocks: &mut Vec<Block>, id: Uuid, nb: Block) -> bool {
    if let Some(pos) = blocks.iter().position(|b| b.id == id) {
        blocks.insert(pos + 1, nb);
        return true;
    }
    for b in blocks.iter_mut() {
        if !b.children.is_empty() && insert_after_id(&mut b.children, id, nb.clone()) {
            return true;
        }
    }
    false
}

fn apply_op(op: Op, blocks: &mut Vec<Block>, st: &mut EditorState, depth: usize) -> bool {
    fn tomb(st: &mut EditorState, depth: usize, id: Uuid) {
        if depth == 0 {
            st.tombstones.push(id);
        }
    }
    match op {
        Op::AddAfter(i) => {
            let nb = Block::new(BlockKind::Paragraph);
            st.focus = Some((nb.id, 0));
            blocks.insert(i + 1, nb);
            true
        }
        Op::Split { idx, at } => {
            let b = &mut blocks[idx];
            if b.kind.is_list() && b.text.is_empty() {
                b.kind = BlockKind::Paragraph;
                b.touch();
                st.focus = Some((b.id, 0));
                return true;
            }
            let tail = b.text.split_off(at);
            let kind = if b.kind.is_list() { b.kind } else { BlockKind::Paragraph };
            let mut nb = Block::new(kind);
            nb.text = tail;
            b.touch();
            st.focus = Some((nb.id, 0));
            blocks.insert(idx + 1, nb);
            true
        }
        Op::MergePrev(idx) => {
            if blocks[idx].kind != BlockKind::Paragraph {
                let b = &mut blocks[idx];
                b.kind = BlockKind::Paragraph;
                b.touch();
                st.focus = Some((b.id, 0));
                return true;
            }
            if idx == 0 {
                return false;
            }
            if !blocks[idx - 1].kind.has_text() {
                let dead = blocks.remove(idx - 1);
                tomb(st, depth, dead.id);
                return true;
            }
            let cur = blocks.remove(idx);
            let prev = &mut blocks[idx - 1];
            let join = prev.text.char_len();
            prev.text.push_rich(&cur.text);
            prev.children.extend(cur.children);
            prev.touch();
            st.focus = Some((prev.id, join));
            tomb(st, depth, cur.id);
            true
        }
        Op::Delete(idx) => {
            let dead = blocks.remove(idx);
            tomb(st, depth, dead.id);
            if blocks.is_empty() {
                blocks.push(Block::new(BlockKind::Paragraph));
            }
            let target = idx.saturating_sub(1).min(blocks.len() - 1);
            st.focus = Some((blocks[target].id, usize::MAX));
            true
        }
        Op::Move { from, to } => {
            let Some(src) = blocks.iter().position(|b| b.id == from) else { return false };
            let mut b = blocks.remove(src);
            let at = if src < to { to - 1 } else { to }.min(blocks.len());
            b.touch();
            blocks.insert(at, b);
            true
        }
        Op::FocusPrev(idx) => {
            if idx > 0 {
                st.focus = Some((blocks[idx - 1].id, usize::MAX));
            }
            false
        }
        Op::FocusNext(idx) => {
            if idx + 1 < blocks.len() {
                st.focus = Some((blocks[idx + 1].id, 0));
            }
            false
        }
    }
}

/// Draw the slash / mention popup (fade + slide + slight scale) and apply a pick.
fn show_menu(ctx: &Context, blocks: &mut Vec<Block>, st: &mut EditorState, pal: &Palette) -> bool {
    let open = st.menu.is_some();
    let a = anim::ease_bool(ctx, Id::new("editor-menu-anim"), open, anim::FAST);
    let Some(mut m) = st.menu.take() else { return false };

    let focused = ctx.memory(|mem| mem.has_focus(tid(m.block)));
    let over = m
        .rect
        .zip(ctx.pointer_latest_pos())
        .is_some_and(|(r, p)| r.expand(4.0).contains(p));
    if !focused && !over {
        return false;
    }

    let items = menu_items(&m, &st.mention_pages);
    m.sel = m.sel.min(items.len().saturating_sub(1));

    if m.pick {
        if let Some((_, _, choice)) = items.into_iter().nth(m.sel) {
            return apply_choice(blocks, st, &m, choice);
        }
        m.pick = false;
        st.menu = Some(m);
        return false;
    }

    let w = 270.0 * (0.96 + 0.04 * a);
    let pos = m.anchor + vec2(0.0, 4.0 + (1.0 - a) * 8.0);
    let area = egui::Area::new(Id::new("editor-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            ui.set_opacity(a);
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(w);
                let title = if m.kind == MenuKind::Slash { "BLOCKS" } else { "LINK TO PAGE" };
                ui.label(egui::RichText::new(title).size(11.0).color(pal.dim));
                if items.is_empty() {
                    ui.label(egui::RichText::new("No results").color(pal.dim));
                }
                egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                    for (idx, (label, hint, _)) in items.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                        if idx == m.sel || resp.hovered() {
                            ui.painter().rect_filled(r, 6.0, pal.hover);
                        }
                        if idx == m.sel && m.moved {
                            resp.scroll_to_me(None);
                        }
                        ui.painter()
                            .text(r.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(14.0), pal.text);
                        ui.painter()
                            .text(r.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, hint, FontId::proportional(11.0), pal.dim);
                        if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
                            m.sel = idx;
                            m.pick = true;
                        }
                    }
                });
            });
        });
    m.moved = false;
    m.rect = Some(area.response.rect);
    st.menu = Some(m);
    false
}

fn apply_choice(blocks: &mut Vec<Block>, st: &mut EditorState, m: &MenuState, choice: Choice) -> bool {
    let Some(b) = block::find_mut(blocks, m.block) else { return false };
    match choice {
        Choice::Kind(kind) => {
            b.text.set_plain("");
            b.kind = kind;
            match kind {
                BlockKind::Code => b.language = "rust".into(),
                BlockKind::Callout => b.emoji = "💡".into(),
                BlockKind::Table if b.table.is_empty() => b.table = vec![vec![String::new(); 3]; 3],
                _ => {}
            }
            b.touch();
            let id = b.id;
            if matches!(kind, BlockKind::Divider | BlockKind::Image | BlockKind::Table) {
                let nb = Block::new(BlockKind::Paragraph);
                st.focus = Some((nb.id, 0));
                insert_after_id(blocks, id, nb);
            } else {
                st.focus = Some((id, 0));
            }
        }
        Choice::Page(pid, title) => {
            let chars: Vec<char> = b.text.text.chars().collect();
            let head: String = chars[..m.trigger.min(chars.len())].iter().collect();
            let mention = mention_text(&title, pid);
            let mut new = nw_core::RichText::new(&head);
            // Keep marks that precede the trigger.
            new.marks = b.text.marks.iter().filter(|mk| mk.end <= m.trigger).cloned().collect();
            new.push_rich(&mention);
            new.push_rich(&nw_core::RichText::new(" "));
            let end = new.char_len();
            b.text = new;
            b.touch();
            st.focus = Some((b.id, end));
        }
    }
    true
}
