//! Databases: typed properties, rows, and table / board / list / calendar views
//! with filters, sorts and grouping.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropKind {
    Title,
    Text,
    Number,
    Select,
    Checkbox,
    Date,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Property {
    pub id: Uuid,
    pub name: String,
    pub kind: PropKind,
    #[serde(default)]
    pub options: Vec<String>,
}

impl Property {
    pub fn new(name: &str, kind: PropKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            kind,
            options: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Value {
    #[default]
    Empty,
    Text(String),
    Number(f64),
    Bool(bool),
    /// ISO date `YYYY-MM-DD`. (Select properties store their option as `Text`.)
    Date(String),
}

impl Value {
    pub fn display(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Text(s) | Self::Date(s) => s.clone(),
            Self::Number(n) => {
                if n.fract() == 0.0 {
                    format!("{n:.0}")
                } else {
                    n.to_string()
                }
            }
            Self::Bool(b) => b.to_string(),
        }
    }

    fn cmp_value(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Empty, Self::Empty) => Ordering::Equal,
            (Self::Empty, _) => Ordering::Greater, // empties last
            (_, Self::Empty) => Ordering::Less,
            (a, b) => a.display().to_lowercase().cmp(&b.display().to_lowercase()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub id: Uuid,
    #[serde(default)]
    pub cells: HashMap<Uuid, Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewKind {
    Table,
    Board,
    List,
    Calendar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterOp {
    Contains,
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    IsEmpty,
    IsNotEmpty,
}

impl FilterOp {
    pub const ALL: [Self; 7] = [
        Self::Contains,
        Self::Equals,
        Self::NotEquals,
        Self::GreaterThan,
        Self::LessThan,
        Self::IsEmpty,
        Self::IsNotEmpty,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::Equals => "is",
            Self::NotEquals => "is not",
            Self::GreaterThan => ">",
            Self::LessThan => "<",
            Self::IsEmpty => "is empty",
            Self::IsNotEmpty => "is not empty",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Filter {
    pub prop: Uuid,
    pub op: FilterOp,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sort {
    pub prop: Uuid,
    pub ascending: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct View {
    pub id: Uuid,
    pub name: String,
    pub kind: ViewKind,
    #[serde(default)]
    pub filters: Vec<Filter>,
    #[serde(default)]
    pub sorts: Vec<Sort>,
    #[serde(default)]
    pub group_by: Option<Uuid>,
    /// Date property used by calendar views.
    #[serde(default)]
    pub date_prop: Option<Uuid>,
}

impl View {
    pub fn new(name: &str, kind: ViewKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            kind,
            filters: Vec::new(),
            sorts: Vec::new(),
            group_by: None,
            date_prop: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Database {
    pub properties: Vec<Property>,
    pub rows: Vec<Row>,
    pub views: Vec<View>,
}

impl Database {
    /// A ready-made task tracker used for new databases.
    pub fn new_tasks() -> Self {
        let name = Property::new("Name", PropKind::Title);
        let mut status = Property::new("Status", PropKind::Select);
        status.options = vec!["Todo".into(), "In progress".into(), "Done".into()];
        let due = Property::new("Due", PropKind::Date);
        let effort = Property::new("Effort", PropKind::Number);
        let done = Property::new("Done", PropKind::Checkbox);
        let mut table = View::new("Table", ViewKind::Table);
        table.sorts.push(Sort { prop: name.id, ascending: true });
        let mut board = View::new("Board", ViewKind::Board);
        board.group_by = Some(status.id);
        let list = View::new("List", ViewKind::List);
        let mut cal = View::new("Calendar", ViewKind::Calendar);
        cal.date_prop = Some(due.id);
        let mut db = Self {
            properties: vec![name, status, due, effort, done],
            rows: Vec::new(),
            views: vec![table, board, list, cal],
        };
        for (n, s) in [("Design the sidebar", "Done"), ("Write the block editor", "In progress"), ("Ship v1", "Todo")] {
            let r = db.add_row();
            db.set(r, 0, Value::Text(n.into()));
            db.set(r, 1, Value::Text(s.into()));
        }
        db
    }

    pub fn prop_index(&self, id: Uuid) -> Option<usize> {
        self.properties.iter().position(|p| p.id == id)
    }

    pub fn add_row(&mut self) -> usize {
        self.rows.push(Row {
            id: Uuid::new_v4(),
            cells: HashMap::new(),
        });
        self.rows.len() - 1
    }

    pub fn set(&mut self, row: usize, prop: usize, v: Value) {
        let pid = self.properties[prop].id;
        self.rows[row].cells.insert(pid, v);
    }

    pub fn get(&self, row: usize, prop: Uuid) -> &Value {
        static EMPTY: Value = Value::Empty;
        self.rows[row].cells.get(&prop).unwrap_or(&EMPTY)
    }

    pub fn title_of(&self, row: usize) -> String {
        self.properties
            .iter()
            .find(|p| p.kind == PropKind::Title)
            .map(|p| self.get(row, p.id).display())
            .unwrap_or_default()
    }

    fn matches(&self, row: usize, f: &Filter) -> bool {
        let v = self.get(row, f.prop);
        let s = v.display().to_lowercase();
        let needle = f.value.to_lowercase();
        match f.op {
            FilterOp::Contains => s.contains(&needle),
            FilterOp::Equals => s == needle,
            FilterOp::NotEquals => s != needle,
            FilterOp::IsEmpty => matches!(v, Value::Empty) || s.is_empty(),
            FilterOp::IsNotEmpty => !(matches!(v, Value::Empty) || s.is_empty()),
            FilterOp::GreaterThan | FilterOp::LessThan => {
                let ord = match (v, needle.parse::<f64>()) {
                    (Value::Number(n), Ok(x)) => n.partial_cmp(&x),
                    _ => s.as_str().partial_cmp(needle.as_str()),
                };
                match (f.op, ord) {
                    (FilterOp::GreaterThan, Some(Ordering::Greater)) => true,
                    (FilterOp::LessThan, Some(Ordering::Less)) => true,
                    _ => false,
                }
            }
        }
    }

    /// Row indices that pass the view's filters, in the view's sort order.
    pub fn visible_rows(&self, view: &View) -> Vec<usize> {
        let mut rows: Vec<usize> = (0..self.rows.len())
            .filter(|&r| view.filters.iter().all(|f| self.matches(r, f)))
            .collect();
        rows.sort_by(|&a, &b| {
            for s in &view.sorts {
                let o = self.get(a, s.prop).cmp_value(self.get(b, s.prop));
                let o = if s.ascending { o } else { o.reverse() };
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        });
        rows
    }

    /// Group visible rows by a select property (declared option order first,
    /// then a trailing "No value" group).
    pub fn groups(&self, view: &View) -> Vec<(String, Vec<usize>)> {
        let rows = self.visible_rows(view);
        let Some(prop) = view.group_by.and_then(|id| self.properties.iter().find(|p| p.id == id)) else {
            return vec![("All".to_owned(), rows)];
        };
        let mut out: Vec<(String, Vec<usize>)> = prop.options.iter().map(|o| (o.clone(), Vec::new())).collect();
        let mut none = Vec::new();
        for r in rows {
            let key = self.get(r, prop.id).display();
            match out.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.push(r),
                None => none.push(r),
            }
        }
        out.push(("No value".to_owned(), none));
        out
    }

    /// Rows whose date property falls on the given day.
    pub fn rows_on(&self, view: &View, date: &str) -> Vec<usize> {
        let Some(p) = view.date_prop else { return Vec::new() };
        self.visible_rows(view)
            .into_iter()
            .filter(|&r| matches!(self.get(r, p), Value::Date(d) if d == date))
            .collect()
    }
}

// ---- calendar maths (proleptic Gregorian, no external crates) ----

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

/// Weekday of a date, 0 = Monday … 6 = Sunday (Sakamoto's algorithm).
pub fn weekday(y: i32, m: u32, d: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let yy = if m < 3 { y - 1 } else { y };
    let sun0 = (yy + yy / 4 - yy / 100 + yy / 400 + T[(m - 1) as usize] + d as i32).rem_euclid(7);
    ((sun0 + 6) % 7) as u32
}

/// (year, month, day) from milliseconds since the Unix epoch (UTC).
pub fn ymd_from_ms(ms: u64) -> (i32, u32, u32) {
    let z = (ms / 86_400_000) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    ((y + i64::from(m <= 2)) as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Database {
        let mut db = Database::new_tasks();
        db.rows.clear();
        let r = db.add_row();
        db.set(r, 0, Value::Text("b task".into()));
        db.set(r, 1, Value::Text("Todo".into()));
        db.set(r, 3, Value::Number(5.0));
        let r = db.add_row();
        db.set(r, 0, Value::Text("a task".into()));
        db.set(r, 1, Value::Text("Done".into()));
        db.set(r, 3, Value::Number(2.0));
        db
    }

    #[test]
    fn sort_filter_group() {
        let db = db();
        let mut v = View::new("t", ViewKind::Table);
        v.sorts.push(Sort { prop: db.properties[0].id, ascending: true });
        assert_eq!(db.visible_rows(&v), vec![1, 0]);
        v.filters.push(Filter { prop: db.properties[3].id, op: FilterOp::GreaterThan, value: "3".into() });
        assert_eq!(db.visible_rows(&v), vec![0]);
        let mut b = View::new("b", ViewKind::Board);
        b.group_by = Some(db.properties[1].id);
        let g = db.groups(&b);
        assert_eq!(g[0].0, "Todo");
        assert_eq!(g[0].1, vec![0]);
        assert_eq!(g[2].1, vec![1]);
    }

    #[test]
    fn calendar_math() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(weekday(2024, 1, 1), 0); // Monday
        assert_eq!(ymd_from_ms(0), (1970, 1, 1));
        assert_eq!(ymd_from_ms(1_709_164_800_000), (2024, 2, 29));
    }
}
