//! Database views: table, board (Kanban, drag cards between columns), list
//! and calendar, with filter / sort / group controls.

use crate::theme::Palette;
use egui::{vec2, Align2, FontId, Id, RichText, Sense, Ui};
use nw_core::database::{
    days_in_month, weekday, ymd_from_ms, Database, Filter, FilterOp, PropKind, Property, Sort, Value, View, ViewKind,
};
use nw_core::time::now_ms;
use nw_core::Uuid;
use std::hash::Hash;

#[derive(Default)]
pub struct DbState {
    pub view: usize,
    pub month: Option<(i32, u32)>,
}

#[derive(Clone, Copy)]
struct CardDrag(usize);

type PropInfo = (Uuid, String, PropKind);

fn prop_combo(ui: &mut Ui, salt: impl Hash, cur: &mut Uuid, props: &[PropInfo]) -> bool {
    let mut changed = false;
    let name = props.iter().find(|p| p.0 == *cur).map_or("?", |p| p.1.as_str());
    egui::ComboBox::from_id_salt(salt).selected_text(name).width(90.0).show_ui(ui, |ui| {
        for (id, n, _) in props {
            changed |= ui.selectable_value(cur, *id, n).changed();
        }
    });
    changed
}

pub fn show_database(ui: &mut Ui, db: &mut Database, st: &mut DbState, pal: &Palette) -> bool {
    let mut changed = false;
    if db.views.is_empty() {
        db.views.push(View::new("Table", ViewKind::Table));
        changed = true;
    }
    st.view = st.view.min(db.views.len() - 1);
    let vi = st.view;

    // View tabs
    ui.horizontal(|ui| {
        for (i, v) in db.views.iter().enumerate() {
            let icon = match v.kind {
                ViewKind::Table => "▦",
                ViewKind::Board => "▥",
                ViewKind::List => "☰",
                ViewKind::Calendar => "📅",
            };
            let selected = st.view == i;
            let a = crate::anim::ease_bool(ui.ctx(), Id::new(("view-tab", i)), selected, crate::anim::NORMAL);
            let resp = ui.selectable_label(selected, format!("{icon} {}", v.name));
            if a > 0.01 {
                let y = resp.rect.bottom();
                let w = resp.rect.width() * a;
                let cx = resp.rect.center().x;
                ui.painter().hline(cx - w / 2.0..=cx + w / 2.0, y, egui::Stroke::new(2.0, pal.accent));
            }
            if resp.clicked() {
                st.view = i;
            }
        }
        ui.menu_button("＋", |ui| {
            for (label, kind) in [
                ("Table", ViewKind::Table),
                ("Board", ViewKind::Board),
                ("List", ViewKind::List),
                ("Calendar", ViewKind::Calendar),
            ] {
                if ui.button(label).clicked() {
                    let mut v = View::new(label, kind);
                    v.group_by = db.properties.iter().find(|p| p.kind == PropKind::Select).map(|p| p.id);
                    v.date_prop = db.properties.iter().find(|p| p.kind == PropKind::Date).map(|p| p.id);
                    db.views.push(v);
                    st.view = db.views.len() - 1;
                    changed = true;
                    ui.close_menu();
                }
            }
        });
    });
    ui.add_space(6.0);
    let vi = vi.min(db.views.len() - 1);
    changed |= toolbar(ui, db, vi);
    ui.add_space(10.0);

    let view = db.views[st.view.min(db.views.len() - 1)].clone();
    match view.kind {
        ViewKind::Table => changed |= table_view(ui, db, &view, pal),
        ViewKind::Board => changed |= board_view(ui, db, &view, pal),
        ViewKind::List => changed |= list_view(ui, db, &view, pal),
        ViewKind::Calendar => changed |= calendar_view(ui, db, &view, st, pal),
    }
    changed
}

fn toolbar(ui: &mut Ui, db: &mut Database, vi: usize) -> bool {
    let mut changed = false;
    let props: Vec<PropInfo> = db.properties.iter().map(|p| (p.id, p.name.clone(), p.kind)).collect();
    let selects: Vec<PropInfo> = props.iter().filter(|p| p.2 == PropKind::Select).cloned().collect();
    let dates: Vec<PropInfo> = props.iter().filter(|p| p.2 == PropKind::Date).cloned().collect();
    ui.horizontal(|ui| {
        let nf = db.views[vi].filters.len();
        ui.menu_button(if nf > 0 { format!("Filter · {nf}") } else { "Filter".to_owned() }, |ui| {
            let view = &mut db.views[vi];
            let mut remove = None;
            for (fi, f) in view.filters.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |= prop_combo(ui, ("filter-prop", fi), &mut f.prop, &props);
                    egui::ComboBox::from_id_salt(("filter-op", fi)).selected_text(f.op.label()).show_ui(ui, |ui| {
                        for op in FilterOp::ALL {
                            changed |= ui.selectable_value(&mut f.op, op, op.label()).changed();
                        }
                    });
                    if !matches!(f.op, FilterOp::IsEmpty | FilterOp::IsNotEmpty) {
                        changed |= ui.add(egui::TextEdit::singleline(&mut f.value).desired_width(90.0)).changed();
                    }
                    if ui.small_button("✕").clicked() {
                        remove = Some(fi);
                    }
                });
            }
            if let Some(fi) = remove {
                view.filters.remove(fi);
                changed = true;
            }
            if ui.button("＋ Add filter").clicked() {
                if let Some(p) = props.first() {
                    view.filters.push(Filter { prop: p.0, op: FilterOp::Contains, value: String::new() });
                    changed = true;
                }
            }
        });
        let ns = db.views[vi].sorts.len();
        ui.menu_button(if ns > 0 { format!("Sort · {ns}") } else { "Sort".to_owned() }, |ui| {
            let view = &mut db.views[vi];
            let mut remove = None;
            for (si, s) in view.sorts.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |= prop_combo(ui, ("sort-prop", si), &mut s.prop, &props);
                    if ui.button(if s.ascending { "↑ Asc" } else { "↓ Desc" }).clicked() {
                        s.ascending = !s.ascending;
                        changed = true;
                    }
                    if ui.small_button("✕").clicked() {
                        remove = Some(si);
                    }
                });
            }
            if let Some(si) = remove {
                view.sorts.remove(si);
                changed = true;
            }
            if ui.button("＋ Add sort").clicked() {
                if let Some(p) = props.first() {
                    view.sorts.push(Sort { prop: p.0, ascending: true });
                    changed = true;
                }
            }
        });
        match db.views[vi].kind {
            ViewKind::Board | ViewKind::List | ViewKind::Table => {
                let cur = db.views[vi].group_by;
                let label = cur
                    .and_then(|id| selects.iter().find(|p| p.0 == id))
                    .map_or("Group".to_owned(), |p| format!("Group · {}", p.1));
                ui.menu_button(label, |ui| {
                    if ui.button("None").clicked() {
                        db.views[vi].group_by = None;
                        changed = true;
                        ui.close_menu();
                    }
                    for (id, name, _) in &selects {
                        if ui.button(name).clicked() {
                            db.views[vi].group_by = Some(*id);
                            changed = true;
                            ui.close_menu();
                        }
                    }
                });
            }
            ViewKind::Calendar => {
                let mut cur = db.views[vi].date_prop.or(dates.first().map(|d| d.0));
                if let Some(c) = cur.as_mut() {
                    if prop_combo(ui, "cal-prop", c, &dates) {
                        db.views[vi].date_prop = Some(*c);
                        changed = true;
                    }
                }
            }
        }
        if ui.button("＋ Property").clicked() {
            let n = db.properties.len() + 1;
            db.properties.push(Property::new(&format!("Property {n}"), PropKind::Text));
            changed = true;
        }
    });
    changed
}

fn set_cell(db: &mut Database, r: usize, pid: Uuid, v: Value) {
    db.rows[r].cells.insert(pid, v);
}

fn cell_editor(ui: &mut Ui, db: &mut Database, r: usize, pi: usize) -> bool {
    let (kind, pid) = (db.properties[pi].kind, db.properties[pi].id);
    let cur = db.get(r, pid).clone();
    let mut changed = false;
    match kind {
        PropKind::Title | PropKind::Text => {
            let mut s = cur.display();
            if ui.add(egui::TextEdit::singleline(&mut s).desired_width(150.0).frame(false)).changed() {
                set_cell(db, r, pid, Value::Text(s));
                changed = true;
            }
        }
        PropKind::Number => {
            let mut n = if let Value::Number(n) = cur { n } else { 0.0 };
            if ui.add(egui::DragValue::new(&mut n).speed(0.1)).changed() {
                set_cell(db, r, pid, Value::Number(n));
                changed = true;
            }
        }
        PropKind::Checkbox => {
            let mut b = matches!(cur, Value::Bool(true));
            if ui.checkbox(&mut b, "").changed() {
                set_cell(db, r, pid, Value::Bool(b));
                changed = true;
            }
        }
        PropKind::Select => {
            let options = db.properties[pi].options.clone();
            let text = if cur.display().is_empty() { "—".to_owned() } else { cur.display() };
            egui::ComboBox::from_id_salt(("sel", r, pi)).selected_text(text).width(120.0).show_ui(ui, |ui| {
                if ui.selectable_label(false, "—").clicked() {
                    set_cell(db, r, pid, Value::Empty);
                    changed = true;
                }
                for o in &options {
                    if ui.selectable_label(cur.display() == *o, o).clicked() {
                        set_cell(db, r, pid, Value::Text(o.clone()));
                        changed = true;
                    }
                }
            });
        }
        PropKind::Date => {
            let mut s = cur.display();
            if ui
                .add(egui::TextEdit::singleline(&mut s).hint_text("YYYY-MM-DD").desired_width(100.0).frame(false))
                .changed()
            {
                set_cell(db, r, pid, if s.is_empty() { Value::Empty } else { Value::Date(s) });
                changed = true;
            }
        }
    }
    changed
}

fn table_view(ui: &mut Ui, db: &mut Database, view: &View, pal: &Palette) -> bool {
    let mut changed = false;
    let rows = db.visible_rows(view);
    let mut delete = None;
    egui::ScrollArea::horizontal().show(ui, |ui| {
        egui::Grid::new(("db-table", view.id)).spacing([10.0, 6.0]).min_col_width(100.0).show(ui, |ui| {
            for p in &mut db.properties {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut p.name)
                        .frame(false)
                        .desired_width(110.0)
                        .text_color(pal.dim)
                        .font(FontId::proportional(13.0)),
                );
                changed |= r.changed();
            }
            ui.label("");
            ui.end_row();
            for &r in &rows {
                for pi in 0..db.properties.len() {
                    changed |= cell_editor(ui, db, r, pi);
                }
                if ui.small_button("🗑").on_hover_text("Delete row").clicked() {
                    delete = Some(r);
                }
                ui.end_row();
            }
        });
    });
    if let Some(r) = delete {
        db.rows.remove(r);
        changed = true;
    }
    if ui.button("＋ New").clicked() {
        db.add_row();
        changed = true;
    }
    changed
}

fn board_view(ui: &mut Ui, db: &mut Database, view: &View, pal: &Palette) -> bool {
    let mut changed = false;
    let groups = db.groups(view);
    let group_prop = view.group_by.and_then(|id| db.prop_index(id));
    let mut moves: Vec<(usize, String)> = Vec::new();
    let mut add_to: Option<String> = None;
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for (name, rows) in &groups {
                if name == "No value" && rows.is_empty() {
                    continue;
                }
                let frame = egui::Frame::new().fill(pal.callout_bg).corner_radius(10).inner_margin(10);
                let (inner, dropped) = ui.dnd_drop_zone::<CardDrag, _>(frame, |ui| {
                    ui.set_width(220.0);
                    ui.set_min_height(120.0);
                    ui.label(RichText::new(format!("{name}  {}", rows.len())).strong().color(pal.dim));
                    ui.add_space(6.0);
                    for &r in rows {
                        ui.dnd_drag_source(Id::new(("card", db.rows[r].id)), CardDrag(r), |ui| {
                            egui::Frame::new()
                                .fill(pal.bg)
                                .stroke(egui::Stroke::new(1.0, pal.border))
                                .corner_radius(8)
                                .inner_margin(10)
                                .show(ui, |ui| {
                                    ui.set_width(196.0);
                                    let t = db.title_of(r);
                                    ui.label(if t.is_empty() { "Untitled".to_owned() } else { t });
                                    for p in &db.properties {
                                        if matches!(p.kind, PropKind::Title) || Some(p.id) == view.group_by {
                                            continue;
                                        }
                                        let d = db.get(r, p.id).display();
                                        if !d.is_empty() {
                                            ui.label(RichText::new(format!("{}: {d}", p.name)).size(12.0).color(pal.dim));
                                        }
                                    }
                                });
                        });
                        ui.add_space(6.0);
                    }
                    if ui.button("＋ New").clicked() {
                        add_to = Some(name.clone());
                    }
                });
                let _ = inner;
                if let Some(d) = dropped {
                    moves.push((d.0, name.clone()));
                }
                ui.add_space(8.0);
            }
        });
    });
    if let Some(gp) = group_prop {
        let pid = db.properties[gp].id;
        for (r, name) in moves {
            if r < db.rows.len() {
                set_cell(db, r, pid, if name == "No value" { Value::Empty } else { Value::Text(name) });
                changed = true;
            }
        }
        if let Some(name) = add_to {
            let r = db.add_row();
            if name != "No value" {
                set_cell(db, r, pid, Value::Text(name));
            }
            changed = true;
        }
    } else if add_to.is_some() {
        db.add_row();
        changed = true;
    }
    changed
}

fn list_view(ui: &mut Ui, db: &mut Database, view: &View, pal: &Palette) -> bool {
    let mut changed = false;
    let rows = db.visible_rows(view);
    let title_idx = db.properties.iter().position(|p| p.kind == PropKind::Title);
    for &r in &rows {
        ui.horizontal(|ui| {
            ui.label(RichText::new("•").color(pal.dim));
            if let Some(ti) = title_idx {
                changed |= cell_editor(ui, db, r, ti);
            }
            for p in &db.properties {
                if p.kind == PropKind::Select {
                    let d = db.get(r, p.id).display();
                    if !d.is_empty() {
                        ui.label(RichText::new(d).size(12.0).background_color(pal.accent_soft));
                    }
                }
            }
        });
    }
    if ui.button("＋ New").clicked() {
        db.add_row();
        changed = true;
    }
    changed
}

fn calendar_view(ui: &mut Ui, db: &mut Database, view: &View, st: &mut DbState, pal: &Palette) -> bool {
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
        "November", "December",
    ];
    let mut changed = false;
    let (ty, tm, td) = ymd_from_ms(now_ms());
    let (mut y, mut m) = st.month.unwrap_or((ty, tm));
    ui.horizontal(|ui| {
        if ui.button("◀").clicked() {
            if m == 1 {
                m = 12;
                y -= 1;
            } else {
                m -= 1;
            }
        }
        ui.label(RichText::new(format!("{} {y}", MONTHS[(m - 1) as usize])).strong().size(16.0));
        if ui.button("▶").clicked() {
            if m == 12 {
                m = 1;
                y += 1;
            } else {
                m += 1;
            }
        }
        if ui.button("Today").clicked() {
            y = ty;
            m = tm;
        }
    });
    st.month = Some((y, m));
    ui.add_space(6.0);
    let Some(date_prop) = view.date_prop else {
        ui.label(RichText::new("Choose a date property above.").color(pal.dim));
        return false;
    };
    let lead = weekday(y, m, 1);
    let days = days_in_month(y, m);
    let mut add_on: Option<String> = None;
    egui::Grid::new(("cal", view.id)).spacing([4.0, 4.0]).show(ui, |ui| {
        for d in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] {
            ui.label(RichText::new(d).color(pal.dim).size(12.0));
        }
        ui.end_row();
        let cells = (lead + days).div_ceil(7) * 7;
        for i in 0..cells {
            let day = i as i64 - i64::from(lead) + 1;
            if day < 1 || day > i64::from(days) {
                ui.allocate_space(vec2(118.0, 84.0));
            } else {
                let date = format!("{y:04}-{m:02}-{day:02}");
                let today = (y, m, day as u32) == (ty, tm, td);
                let on = db.rows_on(view, &date);
                let (rect, resp) = ui.allocate_exact_size(vec2(118.0, 84.0), Sense::click());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 6.0, if resp.hovered() { pal.hover } else { pal.callout_bg });
                if today {
                    painter.circle_filled(rect.left_top() + vec2(14.0, 14.0), 10.0, pal.accent);
                }
                painter.text(
                    rect.left_top() + vec2(14.0, 14.0),
                    Align2::CENTER_CENTER,
                    day.to_string(),
                    FontId::proportional(12.0),
                    if today { egui::Color32::WHITE } else { pal.dim },
                );
                for (k, &r) in on.iter().take(3).enumerate() {
                    let t = db.title_of(r);
                    painter.text(
                        rect.left_top() + vec2(8.0, 30.0 + k as f32 * 16.0),
                        Align2::LEFT_CENTER,
                        t.chars().take(16).collect::<String>(),
                        FontId::proportional(11.5),
                        pal.text,
                    );
                }
                if resp.double_clicked() {
                    add_on = Some(date);
                }
            }
            if (i + 1) % 7 == 0 {
                ui.end_row();
            }
        }
    });
    ui.label(RichText::new("Double-click a day to add an item.").size(12.0).color(pal.dim));
    if let Some(date) = add_on {
        let r = db.add_row();
        set_cell(db, r, date_prop, Value::Date(date));
        changed = true;
    }
    changed
}
