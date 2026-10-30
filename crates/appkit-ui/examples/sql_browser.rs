//! H4, the flagship, step 1: a SQLite browser. Each window is a database
//! (Open… a file, or New for a scratch database in memory): its schema in
//! the sidebar, SQL over the results, run on a background thread (⌘R or
//! the toolbar), results of millions of rows in a native table. A newer
//! query cancels the one running.
//! `cargo run --release -p appkit-ui --example sql_browser [file.sqlite]`
//! (without a file it opens a generated sample with a million orders);
//! `CHECK=all` runs its self-checks and measurements.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use appkit_ui::*;
use rusqlite::Connection;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::ValueRef;

const RUN: Command = Command::new("run-query");
const TOGGLE_SIDEBAR: Command = Command::new("toggle-sidebar");
const TOGGLE_INSPECTOR: Command = Command::new("toggle-inspector");
const TOGGLE_CHART: Command = Command::new("toggle-chart");
const EXPORT_CSV: Command = Command::new("export-csv");
const COPY_CSV: Command = Command::new("copy-csv");

/// An open database: one connection, used by one query at a time from
/// background threads.
#[derive(Clone)]
struct Db(Arc<Mutex<Connection>>);

impl PartialEq for Db {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Db {
    fn scratch() -> Db {
        Db(Arc::new(Mutex::new(Connection::open_in_memory().expect("an in-memory database"))))
    }

    fn open(path: &Path) -> std::io::Result<Db> {
        let mut header = [0u8; 16];
        let len = std::io::Read::read(&mut std::fs::File::open(path)?, &mut header)?;
        if len > 0 && &header != b"SQLite format 3\0" {
            return Err(std::io::Error::other("not a SQLite database"));
        }
        let conn = Connection::open(path).map_err(std::io::Error::other)?;
        Ok(Db(Arc::new(Mutex::new(conn))))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A sidebar item: a section, table, view, index or column.
#[derive(Clone, PartialEq)]
struct Item {
    id: String,
    title: String,
    icon: Option<&'static str>,
    /// The table or view to show when it's selected.
    source: Option<String>,
}

impl Identifiable for Item {
    type Id = String;
    fn id(&self) -> String {
        self.id.clone()
    }
}

/// One cell of a result.
enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(Box<str>),
    Blob(usize),
}

#[derive(Clone, Copy, PartialEq)]
enum ColKind {
    Int,
    Real(usize),
    Text,
}

/// A query's result: the last statement's rows (row-major) and what the
/// statements changed.
#[derive(Default)]
struct Results {
    names: Vec<String>,
    kinds: Vec<ColKind>,
    cells: Vec<Cell>,
    rows: usize,
    changed: usize,
    ms: f64,
    schema_changed: bool,
}

impl Results {
    fn cell(&self, row: u32, col: usize) -> &Cell {
        &self.cells[row as usize * self.names.len() + col]
    }

    fn number(&self, row: u32, col: usize) -> f64 {
        match self.cell(row, col) {
            Cell::Int(i) => *i as f64,
            Cell::Real(r) => *r,
            _ => f64::NAN,
        }
    }

    fn text(&self, row: u32, col: usize) -> String {
        match self.cell(row, col) {
            Cell::Null => "NULL".into(),
            Cell::Int(i) => i.to_string(),
            Cell::Real(r) => r.to_string(),
            Cell::Text(t) => t.to_string(),
            Cell::Blob(n) => format!("<{n} bytes>"),
        }
    }
}

/// Runs `sql` (one or more statements) and keeps the last result.
fn run_sql(db: &Db, sql: &str, cancel: &Cancel) -> Result<Results, String> {
    let start = Instant::now();
    let mut conn = db.lock();
    let c = cancel.clone();
    // A newer query (or a closed window) interrupts this one.
    conn.progress_handler(10_000, Some(move || c.is_cancelled())).map_err(|e| e.to_string())?;
    let version = |conn: &Connection| conn.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    let before = version(&conn);
    let mut out = Results::default();
    let result = statements(&conn, sql, &mut out);
    let _ = conn.progress_handler(0, None::<fn() -> bool>);
    result.map_err(|e| if cancel.is_cancelled() { "Cancelled".to_string() } else { e.to_string() })?;
    out.schema_changed = version(&conn) != before;
    out.ms = start.elapsed().as_secs_f64() * 1e3;
    Ok(out)
}

fn statements(conn: &Connection, sql: &str, out: &mut Results) -> rusqlite::Result<()> {
    let mut batch = rusqlite::Batch::new(conn, sql);
    while let Some(mut stmt) = batch.next()? {
        if stmt.column_count() == 0 {
            out.changed += stmt.execute([])?;
            continue;
        }
        let names: Vec<String> = stmt.column_names().map(String::from).collect();
        let width = names.len();
        let (mut cells, mut count) = (Vec::new(), 0);
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            for i in 0..width {
                cells.push(match row.get_ref(i)? {
                    ValueRef::Null => Cell::Null,
                    ValueRef::Integer(n) => Cell::Int(n),
                    ValueRef::Real(r) => Cell::Real(r),
                    ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).into()),
                    ValueRef::Blob(b) => Cell::Blob(b.len()),
                });
            }
            count += 1;
        }
        // A column of numbers (and no NULLs) sorts and aligns as numbers.
        let kinds = (0..width)
            .map(|i| {
                let mut kind = ColKind::Int;
                for cell in cells.iter().skip(i).step_by(width) {
                    kind = match (kind, cell) {
                        (k, Cell::Int(_)) => k,
                        (ColKind::Int, Cell::Real(r)) | (ColKind::Real(_), Cell::Real(r)) => {
                            let d = if let ColKind::Real(d) = kind { d } else { 0 };
                            let shown = r.to_string();
                            ColKind::Real(d.max(shown.split_once('.').map_or(0, |(_, f)| f.len()).min(6)))
                        }
                        _ => ColKind::Text,
                    };
                    if kind == ColKind::Text {
                        break;
                    }
                }
                if count == 0 { ColKind::Text } else { kind }
            })
            .collect();
        *out = Results { names, kinds, cells, rows: count, changed: out.changed, ..Default::default() };
    }
    Ok(())
}

/// The sidebar: tables (with their columns), views and indexes.
fn schema(db: &Db) -> Result<Vec<TreeNode<Item>>, String> {
    let conn = db.lock();
    let mut stmt = conn
        .prepare("SELECT type, name, tbl_name FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name COLLATE NOCASE")
        .map_err(|e| e.to_string())?;
    let entries: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .and_then(|rows| rows.collect())
        .map_err(|e| e.to_string())?;
    let columns = |name: &str| -> Vec<TreeNode<Item>> {
        let sql = format!("PRAGMA table_info({})", quote(name));
        let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)));
        rows.map(|rows| {
            rows.flatten()
                .map(|(col, ty)| {
                    let title = if ty.is_empty() { col.clone() } else { format!("{col}  {}", ty.to_lowercase()) };
                    TreeNode::leaf(Item { id: format!("{name}.{col}"), title, icon: None, source: None })
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let section = |kind: &str, title: &str, icon: &'static str| {
        let items: Vec<TreeNode<Item>> = entries
            .iter()
            .filter(|(k, ..)| k == kind)
            .map(|(_, name, table)| {
                let shows = (kind != "index").then(|| name.clone());
                let title = if kind == "index" { format!("{name} (on {table})") } else { name.clone() };
                let item = Item { id: format!("{kind}:{name}"), title, icon: Some(icon), source: shows };
                if kind == "index" { TreeNode::leaf(item) } else { TreeNode::branch(item, columns(name)) }
            })
            .collect();
        (!items.is_empty()).then(|| TreeNode::branch(Item { id: title.into(), title: title.into(), icon: None, source: None }, items))
    };
    Ok([section("table", "Tables", "tablecells"), section("view", "Views", "eye"), section("index", "Indexes", "arrow.up.arrow.down")]
        .into_iter()
        .flatten()
        .collect())
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// One window's state.
#[derive(Clone, Copy)]
struct Browser {
    query: Signal<String>,
    /// The query last run (and a count, so running it again reloads).
    ran: Signal<(u64, String)>,
    results: Resource<Arc<Results>>,
    schema: Resource<Vec<TreeNode<Item>>>,
    selected: Signal<Option<String>>,
    fill_ms: Signal<f64>,
    /// The results row the inspector shows.
    row: Signal<Option<u32>>,
    no_sidebar: Signal<bool>,
    no_inspector: Signal<bool>,
    no_chart: Signal<bool>,
    /// Queries run, newest first.
    history: Signal<Vec<String>>,
    history_shown: Signal<bool>,
}

impl Browser {
    fn new(db: &Db) -> Browser {
        let ran = Signal::new((0, String::new()));
        let (d1, d2) = (db.clone(), db.clone());
        Browser {
            query: Signal::new(String::new()),
            ran,
            results: Resource::new(move || ran.get(), move |(_, sql): (u64, String), cancel: &Cancel| run_sql(&d1, &sql, cancel).map(Arc::new)),
            schema: Resource::new(|| (), move |(), _: &Cancel| schema(&d2)),
            selected: Signal::new(None),
            fill_ms: Signal::new(0.0),
            row: Signal::new(None),
            no_sidebar: Signal::new(false),
            no_inspector: Signal::new(false),
            no_chart: Signal::new(true),
            history: Signal::new(Vec::new()),
            history_shown: Signal::new(false),
        }
    }

    fn run(&self) {
        let sql = self.query.get_untracked();
        if !sql.trim().is_empty() {
            self.history.update(|h| {
                h.retain(|q| *q != sql);
                h.insert(0, sql.clone());
                h.truncate(20);
            });
        }
        // A query of one's own isn't the selected table's any more.
        let shown = self.selected.get_untracked().and_then(|id| id.split_once(':').map(|(_, name)| format!("SELECT * FROM {};", quote(name))));
        if shown.is_some_and(|q| q != sql) {
            self.selected.set(None);
        }
        self.ran.update(|(n, s)| {
            *n += 1;
            *s = sql;
        });
    }

    fn show(&self, source: &str) {
        self.query.set(format!("SELECT * FROM {};", quote(source)));
        self.run();
    }

    fn status(&self) -> String {
        if self.results.loading() {
            return t("Running…");
        }
        match self.results.state() {
            Load::Loading => t("Running…"),
            Load::Failed(e) => t_with("Error: {message}", &[("message", &e)]),
            Load::Ready(r) if r.names.is_empty() && r.changed > 0 => format!("{} · {:.0} ms", t_plural(r.changed as i64, "{n} row changed", "{n} rows changed"), r.ms),
            Load::Ready(r) if r.names.is_empty() => format!("{} · {:.0} ms", t("Done"), r.ms),
            Load::Ready(r) => format!(
                "{} · {}",
                t_plural(r.rows as i64, "{n} row", "{n} rows"),
                t_with("query {query} ms · table {table} ms", &[("query", &format!("{:.0}", r.ms)), ("table", &format!("{:.0}", self.fill_ms.get()))])
            ),
        }
    }
}

/// The table's columns for a result. Rows are row numbers into it.
fn columns(r: &Arc<Results>) -> Vec<Column<u32>> {
    (0..r.names.len())
        .map(|i| {
            let (id, name, res) = (format!("c{i}"), &r.names[i], r.clone());
            // Wide enough for the title and the first rows' values.
            let chars = (0..r.rows.min(200) as u32).map(|row| r.text(row, i).chars().count()).chain([name.chars().count()]).max().unwrap_or(4);
            let width = (chars.min(40) as f64 * 7.5 + 20.0).max(60.0);
            let column = match r.kinds[i] {
                ColKind::Int => Column::number(&id, name, move |&row| res.number(row, i)).decimals(0),
                ColKind::Real(d) => Column::number(&id, name, move |&row| res.number(row, i)).decimals(d),
                ColKind::Text => Column::text(&id, name, move |&row| res.text(row, i)),
            };
            column.width(width)
        })
        .collect()
}

fn database_view(doc: &Document<Db>) -> impl View + use<> {
    let db = doc.content().get_untracked();
    let b = Browser::new(&db);
    let editor = TextModel::new(ViewId(1), "");
    editor.set_font(Some("Menlo"), 12.0);
    editor.set_spell_checking(false);
    BROWSERS.with_borrow_mut(|all| all.push((b, editor.clone())));
    let model = std::rc::Rc::new(TableModel::new(ViewId(2), Vec::new()));
    let m = model.clone();
    // A new result replaces the columns and rows of the same native table.
    let fill = watch(move || {
        let Some(r) = b.results.get() else { return };
        let start = Instant::now();
        m.set_columns(columns(&r));
        m.set_rows((0..r.rows as u32).collect());
        b.row.set(None);
        b.fill_ms.set(start.elapsed().as_secs_f64() * 1e3);
        if r.schema_changed {
            b.schema.refetch();
        }
    });
    // Selecting a table or view shows its rows.
    let select = watch(move || {
        let Some(id) = b.selected.get() else { return };
        // Not when the schema reloads (after a write): only a new selection.
        let source = untrack(|| b.schema.with(|s| find(s.map(Vec::as_slice).unwrap_or_default(), &id).and_then(|i| i.source.clone())));
        if let Some(source) = source {
            untrack(|| b.show(&source));
        }
    });
    // A new database starts on its first table.
    let first = watch(move || {
        let first = b.schema.with(|s| s.and_then(|s| s.first()).filter(|t| t.value.id == "Tables").and_then(|t| t.children.first()).map(|t| t.value.id.clone()));
        if first.is_some() && untrack(|| b.selected.get().is_none() && b.query.get().is_empty()) {
            untrack(|| b.selected.set(first));
        }
    });
    split((
        // (Section names are translated here: the schema is read off the
        // main thread, where the app's tables aren't.)
        sidebar(move || b.schema.get().unwrap_or_default(), |i: &Item| if i.icon.is_none() && i.source.is_none() && !i.id.contains('.') { t(&i.title) } else { i.title.clone() })
            .icon(|i: &Item| i.icon.map(String::from))
            .selection(&b.selected),
        split((
            text_editor(&editor).bind(&b.query).syntax(Syntax::sql()).accessibility_label("Query"),
            vstack((
                split((table(&model).on_select(move |rows| b.row.set(rows.first().map(|&r| r as u32))).accessibility_label("Results"), chart(b)))
                    .stacked()
                    .collapsible(1, &b.no_chart)
                    .trailing(240.0),
                hstack((
                    label(move || b.status()),
                    spacer(),
                    csv_chip(b),
                    button(&t("History"), move || b.history_shown.set(true)).popover(&b.history_shown, move || history(b)),
                ))
                .padding(6.0),
                fill,
                select,
                first,
            ))
            .spacing(0.0),
        ))
        .stacked()
        .position(110.0),
        inspector(b),
    ))
    .collapsible(0, &b.no_sidebar)
    .collapsible(2, &b.no_inspector)
    .position(220.0)
    .trailing(260.0)
    .on_command(RUN, move || b.run())
    .on_command(TOGGLE_SIDEBAR, move || b.no_sidebar.update(|h| *h = !*h))
    .on_command(TOGGLE_INSPECTOR, move || b.no_inspector.update(|h| *h = !*h))
    .on_command(TOGGLE_CHART, move || b.no_chart.update(|h| *h = !*h))
    .on_command_when(EXPORT_CSV, move || b.results.with(|r| r.is_some_and(|r| !r.names.is_empty())), move || export_csv(b))
    .on_command_when(COPY_CSV, move || b.results.with(|r| r.is_some_and(|r| !r.names.is_empty())), move || {
        if let Some(r) = b.results.get() {
            Clipboard::general().set_text(&csv(&r));
        }
    })
    .focused_value("browser", b)
}

/// What the chart shows for a result: the first column across, the first
/// numeric column after it up. A numeric first column makes a line (every
/// row); text makes bars (the first 200 rows).
enum Plot {
    Line(String, Vec<(f64, f64)>),
    Bars(Vec<(String, f64)>),
    Nothing,
}

fn plot(r: &Results) -> Plot {
    let numeric = |k: ColKind| matches!(k, ColKind::Int | ColKind::Real(_));
    let Some(y) = (1..r.names.len()).find(|&i| numeric(r.kinds[i])) else {
        return Plot::Nothing;
    };
    if numeric(r.kinds[0]) {
        Plot::Line(r.names[y].clone(), (0..r.rows as u32).map(|row| (r.number(row, 0), r.number(row, y))).collect())
    } else {
        Plot::Bars((0..r.rows.min(200) as u32).map(|row| (r.text(row, 0), r.number(row, y))).collect())
    }
}

/// The results as a chart (only computed while it's shown).
fn chart(b: Browser) -> impl View {
    // Rebuilt only when the kind of chart changes, not for every result.
    let shape = Memo::new(move || {
        let numeric = |k: ColKind| matches!(k, ColKind::Int | ColKind::Real(_));
        b.results.with(|r| match r {
            Some(r) if (1..r.names.len()).any(|i| numeric(r.kinds[i])) => if numeric(r.kinds[0]) { 1 } else { 2 },
            _ => 0,
        })
    });
    dynamic(move || {
        let shape = shape.get();
        let data = move || if b.no_chart.get() { None } else { b.results.get().map(|r| plot(&r)) };
        match shape {
            1 => AnyView::new(
                line_chart(move || match data() {
                    Some(Plot::Line(name, points)) => vec![Series::new(&name, points)],
                    _ => Vec::new(),
                })
                .exportable("chart"),
            ),
            2 => AnyView::new(
                bar_chart(move || match data() {
                    Some(Plot::Bars(bars)) => bars,
                    _ => Vec::new(),
                })
                .exportable("chart"),
            ),
            _ => AnyView::new(text(t("Charts need a first column and a number column after it")).padding(12.0)),
        }
    })
}

/// A result as CSV (RFC 4180): a header line, then the rows; fields with
/// commas, quotes or line breaks are quoted, NULL is empty.
fn csv(r: &Results) -> String {
    let mut out = String::new();
    write_csv(r, &mut out).expect("writing to a string");
    out
}

fn write_csv(r: &Results, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    let field = |out: &mut dyn std::fmt::Write, s: &str| {
        if s.contains([',', '"', '\n', '\r']) { write!(out, "\"{}\"", s.replace('"', "\"\"")) } else { out.write_str(s) }
    };
    for (i, name) in r.names.iter().enumerate() {
        if i > 0 {
            out.write_char(',')?;
        }
        field(out, name)?;
    }
    out.write_char('\n')?;
    for row in 0..r.rows as u32 {
        for i in 0..r.names.len() {
            if i > 0 {
                out.write_char(',')?;
            }
            match r.cell(row, i) {
                Cell::Null => {}
                _ => field(out, &r.text(row, i))?,
            }
        }
        out.write_char('\n')?;
    }
    Ok(())
}

/// Writes the result as CSV at `path` (off the main thread for a million rows).
fn save_csv(r: &Results, path: &std::path::Path) -> std::io::Result<()> {
    struct Out<W: std::io::Write>(W, Option<std::io::Error>);
    impl<W: std::io::Write> std::fmt::Write for Out<W> {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            self.0.write_all(s.as_bytes()).map_err(|e| {
                self.1 = Some(e);
                std::fmt::Error
            })
        }
    }
    let mut out = Out(std::io::BufWriter::new(std::fs::File::create(path)?), None);
    if write_csv(r, &mut out).is_err() {
        return Err(out.1.unwrap_or_else(|| std::io::Error::other("couldn't write the CSV")));
    }
    std::io::Write::flush(&mut out.0)
}

/// File ▸ Export Results as CSV…: a Save panel, then written in the background.
fn export_csv(b: Browser) {
    let (Some(r), Some(window)) = (b.results.get_untracked(), focused_window()) else { return };
    save_panel(&window, "results.csv", None, move |path| {
        let Some(path) = path else { return };
        spawn(move || save_csv(&r, &path), |result| {
            if let Err(e) = result {
                eprintln!("Couldn't export: {e}");
            }
        });
    });
}

/// A "results.csv" chip: drag it to the Finder (or Mail) for the results as
/// a CSV file, written where it's dropped.
fn csv_chip(b: Browser) -> impl View {
    canvas(|p| {
        p.fill_rounded_rect(0.0, 0.0, p.width(), p.height(), 5.0, TextColor::Separator);
        p.text(p.width() / 2.0, 3.0, "results.csv", 11.0, TextColor::Secondary, Align::Center);
    })
    .draggable(move || {
        let r = b.results.get_untracked()?;
        (!r.names.is_empty()).then(|| DragItem::promise("results.csv", move |path| save_csv(&r, path)))
    })
    .description(|| t("results.csv: drag out the results as a CSV file"))
    .min_size(84.0, 20.0)
    .frame(84.0, 20.0)
}

/// The selected row, one field per line.
fn inspector(b: Browser) -> impl View {
    dynamic(move || {
        let (Some(row), Some(r)) = (b.row.get(), b.results.get()) else {
            return AnyView::new(text(t("No row selected")).padding(12.0));
        };
        if row as usize >= r.rows {
            return AnyView::new(text(t("No row selected")).padding(12.0));
        }
        let fields: Vec<_> = r.names.iter().enumerate().map(|(i, name)| (text(name.clone()), text(r.text(row, i)))).collect();
        AnyView::new(scroll(vstack((text(t_with("Row {n}", &[("n", &(row + 1))])), grid(fields).align_column(0, Align::End))).padding(12.0)))
    })
}

/// The query history, in a popover: choosing one puts it in the editor.
fn history(b: Browser) -> impl View {
    let queries = b.history.get_untracked();
    if queries.is_empty() {
        return AnyView::new(text(t("No queries yet")).padding(12.0));
    }
    let items: Vec<AnyView> = queries
        .into_iter()
        .map(|q| {
            let title: String = q.split_whitespace().collect::<Vec<_>>().join(" ");
            let title = if title.chars().count() > 60 { format!("{}…", title.chars().take(59).collect::<String>()) } else { title };
            AnyView::new(button(&title, move || {
                b.query.set(q.clone());
                b.history_shown.set(false);
            }))
        })
        .collect();
    AnyView::new(vstack(items).alignment(Align::Start).padding(12.0))
}

/// "Show Sidebar" or "Hide Sidebar", for the focused window.
fn shown_title(what: &str, hidden: fn(&Browser) -> Signal<bool>) -> String {
    let hidden = focused_value::<Browser>("browser").is_some_and(|b| hidden(&b).get_untracked());
    t(&format!("{} {what}", if hidden { "Show" } else { "Hide" }))
}

fn find<'a>(nodes: &'a [TreeNode<Item>], id: &str) -> Option<&'a Item> {
    nodes.iter().find_map(|n| if n.value.id == id { Some(&n.value) } else { find(&n.children, id) })
}

thread_local! {
    /// Every window's state, for the self-checks.
    static BROWSERS: std::cell::RefCell<Vec<(Browser, TextModel)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn database() -> DocumentType<Db> {
    DocumentType::new(&["sqlite", "db", "sqlite3"], Db::scratch, database_view)
        .read(Db::open)
        .toolbar(|_| {
            vec![
                toolbar_command("run", &t("Run"), "play.fill", RUN),
                toolbar_space(),
                toolbar_command("chart", &t("Chart"), "chart.xyaxis.line", TOGGLE_CHART),
                toolbar_command("inspector", &t("Inspector"), "sidebar.right", TOGGLE_INSPECTOR),
            ]
        })
        .size(1100.0, 700.0)
}

fn main() {
    let check = std::env::var("CHECK").unwrap_or_default();
    // A file to open (not AppKit's options, like `-AppleLanguages "(de)"`).
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option_value = |i: usize| i > 0 && args[i - 1].starts_with('-');
    let file = args.iter().enumerate().find(|(i, a)| !(a.starts_with('-') || option_value(*i))).map(|(_, a)| std::path::PathBuf::from(a));
    App::new("SQL Browser")
        .translations(&[("de", include_str!("sql_browser.de.strings"))])
        .documents(database())
        .menu("File", vec![menu_separator(), menu_command(&t("Export Results as CSV…"), EXPORT_CSV).key("E")])
        .menu(&t("Query"), vec![menu_command(&t("Run Query"), RUN).key("r"), menu_command(&t("Copy Results as CSV"), COPY_CSV)])
        .menu(
            &t("View"),
            vec![
                menu_command(&t("Hide Sidebar"), TOGGLE_SIDEBAR)
                    .key_with("s", &[Modifier::Control])
                    .title_when(|| shown_title("Sidebar", |b| b.no_sidebar)),
                menu_command(&t("Hide Inspector"), TOGGLE_INSPECTOR)
                    .key_with("i", &[Modifier::Option])
                    .title_when(|| shown_title("Inspector", |b| b.no_inspector)),
                menu_command(&t("Show Chart"), TOGGLE_CHART)
                    .key_with("k", &[Modifier::Option])
                    .title_when(|| shown_title("Chart", |b| b.no_chart)),
            ],
        )
        .on_launch(move || {
            if check == "all" {
                self_checks::run();
                return;
            }
            if check == "l10n" {
                // Run with -AppleLanguages "(de)": the app in German.
                self_checks::l10n();
                return;
            }
            let open = |path: &Path| {
                if let Err(e) = open_document(path) {
                    eprintln!("Couldn't open {}: {e}", path.display());
                }
            };
            match file {
                Some(path) => open(&path),
                None => {
                    let path = std::env::temp_dir().join("ferrokit-sample.sqlite");
                    spawn(
                        move || {
                            if !path.exists() {
                                sample::create(&path, 1_000_000).map_err(|e| e.to_string())?;
                            }
                            Ok::<_, String>(path)
                        },
                        move |r| match r {
                            Ok(path) => open(&path),
                            Err(e) => eprintln!("Couldn't make the sample: {e}"),
                        },
                    );
                }
            }
        })
        .run();
}

// ---- the sample database ----

mod sample {
    use rusqlite::{Connection, params};

    /// Customers, products and `orders` rows of orders, from a fixed seed.
    pub fn create(path: &std::path::Path, orders: usize) -> rusqlite::Result<()> {
        let tmp = path.with_extension("partial");
        let _ = std::fs::remove_file(&tmp);
        let mut conn = Connection::open(&tmp)?;
        conn.execute_batch(
            "PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;
             CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, city TEXT NOT NULL, since TEXT NOT NULL);
             CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL, category TEXT NOT NULL, price REAL NOT NULL);
             CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers, product_id INTEGER NOT NULL REFERENCES products, quantity INTEGER NOT NULL, total REAL NOT NULL, ordered_at TEXT NOT NULL);",
        )?;
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        const FIRST: [&str; 12] = ["Ada", "Ben", "Chloé", "Dev", "Emma", "Farah", "Goro", "Hana", "Ivan", "Jun", "Kofi", "Lena"];
        const LAST: [&str; 10] = ["Okafor", "Lindqvist", "Tanaka", "García", "Novak", "Reyes", "Haddad", "Schmidt", "Kim", "Moreau"];
        const CITIES: [&str; 8] = ["Lisbon", "Osaka", "Toronto", "Nairobi", "Berlin", "Austin", "Lyon", "Seoul"];
        const CATEGORIES: [&str; 5] = ["Books", "Garden", "Kitchen", "Music", "Tools"];
        let tx = conn.transaction()?;
        {
            let mut add = tx.prepare("INSERT INTO customers VALUES (?, ?, ?, ?)")?;
            for id in 1..=10_000i64 {
                let name = format!("{} {}", FIRST[next(12) as usize], LAST[next(10) as usize]);
                let since = format!("20{:02}-{:02}-{:02}", 15 + next(11), 1 + next(12), 1 + next(28));
                add.execute(params![id, name, CITIES[next(8) as usize], since])?;
            }
            let mut add = tx.prepare("INSERT INTO products VALUES (?, ?, ?, ?)")?;
            let mut prices = Vec::new();
            for id in 1..=200i64 {
                let category = CATEGORIES[next(5) as usize];
                let price = (199 + next(19_800)) as f64 / 100.0;
                prices.push(price);
                add.execute(params![id, format!("{category} item {id}"), category, price])?;
            }
            let mut add = tx.prepare("INSERT INTO orders VALUES (?, ?, ?, ?, ?, ?)")?;
            for id in 1..=orders as i64 {
                let product = 1 + next(200) as i64;
                let quantity = 1 + next(9) as i64;
                let total = (prices[product as usize - 1] * quantity as f64 * 100.0).round() / 100.0;
                let at = format!("202{}-{:02}-{:02} {:02}:{:02}", next(6), 1 + next(12), 1 + next(28), next(24), next(60));
                add.execute(params![id, 1 + next(10_000) as i64, product, quantity, total, at])?;
            }
        }
        tx.execute_batch(
            "CREATE INDEX orders_by_customer ON orders (customer_id);
             CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 1000;",
        )?;
        tx.commit()?;
        drop(conn);
        std::fs::rename(&tmp, path).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
    }
}

// ---- self-checks (not part of the app code count) ----

mod self_checks {
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::time::Instant;

    use appkit_ui::testing::{ax_audit, canvas_drag, canvas_drawing, canvas_png, capture_window, drop_on_view, footprint_mb, fulfil, menu_key, snapshot_windows};
    use appkit_ui::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSAccessibility, NSEventModifierFlags, NSOutlineView, NSSplitView, NSTableView, NSView};
    use objc2_foundation::{NSArray, NSSortDescriptor};

    use super::{BROWSERS, Browser, RUN};

    thread_local! {
        static FAILED: Cell<u32> = const { Cell::new(0) };
    }

    fn check(name: &str, pass: bool, note: String) {
        eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
        if !pass {
            FAILED.with(|f| f.set(f.get() + 1));
        }
    }

    fn views(v: &NSView, out: &mut Vec<Retained<NSView>>) {
        for s in v.subviews().iter() {
            out.push(s.clone());
            views(&s, out);
        }
    }

    fn all(w: &NSWindow) -> Vec<Retained<NSView>> {
        let mut out = Vec::new();
        views(&w.contentView().unwrap(), &mut out);
        out
    }

    /// The results table (not the sidebar, an NSOutlineView).
    fn results(w: &NSWindow) -> Retained<NSTableView> {
        all(w).into_iter().filter_map(|v| v.downcast::<NSTableView>().ok()).find(|t| t.downcast_ref::<NSOutlineView>().is_none()).unwrap()
    }

    fn titles(t: &NSTableView) -> Vec<String> {
        t.tableColumns().iter().map(|c| c.title().to_string()).collect()
    }

    /// The text of a results cell, as the table shows it.
    fn cell(t: &NSTableView, row: isize, col: usize) -> String {
        let column = t.tableColumns().objectAtIndex(col);
        let view: Option<Retained<NSView>> = unsafe { objc2::msg_send![&*t.delegate().unwrap(), tableView: t, viewForTableColumn: &*column, row: row] };
        view.and_then(|v| v.downcast::<objc2_app_kit::NSTextField>().ok()).map(|f| f.stringValue().to_string()).unwrap_or_default()
    }

    fn sidebar_titles(b: &Browser) -> Vec<String> {
        fn walk(nodes: &[TreeNode<super::Item>], out: &mut Vec<String>) {
            for n in nodes {
                out.push(n.value.title.clone());
                walk(&n.children, out);
            }
        }
        let mut out = Vec::new();
        walk(&untrack(|| b.schema.get()).unwrap_or_default(), &mut out);
        out
    }

    /// Waits (polling the run loop) until `done`, at most `ms`, then runs
    /// `then` with the time it took. The first look is on a later pass:
    /// effects of what the caller just set may still be pending.
    fn when(done: impl Fn() -> bool + 'static, ms: u64, then: impl FnOnce(f64) + 'static) {
        let start = Instant::now();
        let done = std::rc::Rc::new(done);
        fn poll(start: Instant, ms: u64, done: std::rc::Rc<dyn Fn() -> bool>, then: Box<dyn FnOnce(f64)>) {
            spawn(|| std::thread::sleep(std::time::Duration::from_millis(10)), move |()| {
                if done() || start.elapsed().as_millis() as u64 > ms {
                    then(start.elapsed().as_secs_f64() * 1e3);
                } else {
                    poll(start, ms, done, then);
                }
            });
        }
        poll(start, ms, done, Box::new(then));
    }

    fn idle(b: Browser) -> impl Fn() -> bool {
        move || untrack(|| !b.results.loading())
    }

    fn run_query(w: &NSWindow, b: Browser, sql: &str) {
        b.query.set(sql.into());
        perform_in(Some(w), RUN);
    }

    fn menu_titles(menu: &objc2_app_kit::NSMenu) -> Vec<String> {
        (0..menu.numberOfItems()).map(|i| menu.itemAtIndex(i).unwrap().title().to_string()).collect()
    }

    /// The app in German (launched with `-AppleLanguages "(de)"`): its own
    /// strings from its table, the framework's from appkit-ui's, plurals.
    pub fn l10n() {
        let app = objc2_app_kit::NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
        let bar = app.mainMenu().unwrap();
        let menus = menu_titles(&bar);
        check("l10n-language", language() == "de", format!("language {:?}", language()));
        check(
            "l10n-menu-bar",
            menus[1..] == ["Ablage", "Bearbeiten", "Abfrage", "Darstellung", "Fenster", "Hilfe"].map(String::from),
            format!("{menus:?}"),
        );
        let file = bar.itemAtIndex(1).unwrap().submenu().unwrap();
        file.update();
        let items = menu_titles(&file);
        let want = ["Neu", "Öffnen …", "Benutzte Dokumente öffnen", "Schließen", "Sichern", "Sichern unter …", "Zurück zur gesicherten Version", "Ergebnisse als CSV exportieren …"];
        check("l10n-file-menu", want.iter().all(|w| items.contains(&w.to_string())), format!("{items:?}"));
        let app_menu = menu_titles(&bar.itemAtIndex(0).unwrap().submenu().unwrap());
        check(
            "l10n-app-menu",
            app_menu.first().map(String::as_str) == Some("Über SQL Browser") && app_menu.last().map(String::as_str) == Some("SQL Browser beenden"),
            format!("{app_menu:?}"),
        );
        let w = windows().into_iter().find(|w| w.isVisible()).unwrap();
        let toolbar: Vec<String> = w.toolbar().map(|t| t.items().iter().map(|i| i.label().to_string()).filter(|l| !l.is_empty()).collect()).unwrap_or_default();
        check("l10n-window", w.title().to_string() == "Ohne Titel" && toolbar == ["Ausführen", "Diagramm", "Inspektor"], format!("title {:?}; toolbar {toolbar:?}", w.title()));
        let b = BROWSERS.with_borrow(|all| all.last().unwrap().0);
        let view = bar.itemAtIndex(4).unwrap().submenu().unwrap();
        view.update();
        let view_items = menu_titles(&view);
        check(
            "l10n-view-menu",
            view_items == ["Seitenleiste ausblenden", "Inspektor ausblenden", "Diagramm einblenden"],
            format!("{view_items:?} (a new window shows its sidebar and inspector, not its chart)"),
        );
        run_query(&w, b, "SELECT 1 AS a");
        let w2 = w.clone();
        when(move || idle(b)() && results(&w2).numberOfRows() == 1, 5000, move |_| {
            let one = untrack(|| b.status());
            run_query(&w, b, "SELECT 1 AS a UNION ALL SELECT 2");
            let w3 = w.clone();
            when(move || idle(b)() && results(&w3).numberOfRows() == 2, 5000, move |_| {
                let two = untrack(|| b.status());
                check("l10n-plural-status", one.starts_with("1 Zeile · Abfrage") && two.starts_with("2 Zeilen · Abfrage"), format!("{one:?}; {two:?}"));
                let failed = FAILED.with(Cell::get);
                eprintln!("[check] done, {failed} failed");
                std::process::exit(if failed == 0 { 0 } else { 1 });
            });
        });
    }

    pub fn run() {
        let dir = std::env::temp_dir().join(format!("appkit-ui-sql-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.sqlite");
        let p = path.clone();
        let start = Instant::now();
        spawn(move || super::sample::create(&p, 1_000_000).map_err(|e| e.to_string()), move |r| {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
            check("sample-db", r.is_ok(), format!("1,000,000 orders written in {:.1} s ({size:.0} MB): {r:?}", start.elapsed().as_secs_f64()));
            open_checks(dir, path);
        });
    }

    fn open_checks(dir: PathBuf, path: PathBuf) {
        let before = footprint_mb();
        let opened = open_document(&path);
        check("open-database", opened.is_ok(), format!("{:?}", opened.as_ref().err()));
        let w = opened.unwrap();
        let b = BROWSERS.with_borrow(|all| all.last().unwrap().0);
        let w2 = w.clone();
        when(move || untrack(|| b.schema.get().is_some()) && idle(b)() && results(&w2).numberOfRows() > 0, 5000, move |ms| {
            let side = sidebar_titles(&b);
            let want = ["Tables", "customers", "orders", "products", "Views", "big_orders", "Indexes", "orders_by_customer (on orders)"];
            check("schema-sidebar", want.iter().all(|t| side.contains(&t.to_string())) && side.contains(&"total  real".to_string()), format!("{side:?}"));
            let t = results(&w);
            check(
                "first-table",
                w.title().to_string() == "sample.sqlite" && untrack(|| b.query.get()) == "SELECT * FROM \"customers\";" && t.numberOfRows() == 10_000 && titles(&t) == ["id", "name", "city", "since"],
                format!("title {:?}; query {:?}; {} rows, columns {:?} after {ms:.0} ms", w.title(), untrack(|| b.query.get()), t.numberOfRows(), titles(&t)),
            );
            let editor = BROWSERS.with_borrow(|all| all.last().unwrap().1.clone());
            let keyword = TextStyle::new().color(TextColor::Pink).bold();
            let (select, name) = (editor.style_at(0), editor.style_at(15));
            check("sql-highlight", select == keyword && name == TextStyle::new(), format!("SELECT {select:?}; the quoted \"customers\" {name:?}"));
            let problems = ax_audit(&w);
            check("accessibility-audit", problems.is_empty(), format!("{problems:?}"));
            let stacked = all(&w).into_iter().filter_map(|v| v.downcast::<NSSplitView>().ok()).any(|s| !s.isVertical());
            check("stacked-split", stacked, "the editor sits over the results in a stacked split".into());
            million(w, b, dir, before);
        });
    }

    fn million(w: Retained<NSWindow>, b: Browser, dir: PathBuf, before: f64) {
        let table = results(&w);
        let ptr = Retained::as_ptr(&table);
        run_query(&w, b, "SELECT * FROM orders");
        let w2 = w.clone();
        when(move || idle(b)() && results(&w2).numberOfRows() == 1_000_000, 30_000, move |ms| {
            let t = results(&w);
            let (query_ms, fill_ms) = untrack(|| (b.results.get().map(|r| r.ms).unwrap_or(0.0), b.fill_ms.get()));
            let grew = footprint_mb() - before;
            check(
                "million-rows",
                t.numberOfRows() == 1_000_000 && titles(&t) == ["id", "customer_id", "product_id", "quantity", "total", "ordered_at"] && cell(&t, 0, 0) == "1",
                format!("{} rows shown {ms:.0} ms after Run (query {query_ms:.0} ms, table {fill_ms:.0} ms); footprint +{grew:.0} MB", t.numberOfRows()),
            );
            check("same-table", Retained::as_ptr(&t) == ptr, "new columns go into the same NSTableView".into());
            let cleared = untrack(|| b.selected.get()).is_none();
            check("typed-query-clears-selection", cleared, "running a query of one's own deselects the sidebar's table".into());
            let numeric = cell(&t, 0, 4).parse::<f64>().is_ok() && cell(&t, 0, 4).contains('.');
            check("number-columns", numeric, format!("total shows as {:?}", cell(&t, 0, 4)));
            // Sort by total, descending, through AppKit's entry point.
            let start = Instant::now();
            let column = t.tableColumns().objectAtIndex(4);
            let d = column.sortDescriptorPrototype().unwrap().reversedSortDescriptor();
            let d: Retained<NSSortDescriptor> = unsafe { objc2::rc::Retained::cast_unchecked(d) };
            t.setSortDescriptors(&NSArray::from_retained_slice(&[d]));
            let sort_ms = start.elapsed().as_secs_f64() * 1e3;
            let (top, next) = (cell(&t, 0, 4).parse::<f64>().unwrap_or(0.0), cell(&t, 1, 4).parse::<f64>().unwrap_or(f64::MAX));
            check("sort-million", top >= next && top > 1000.0, format!("sorted 1,000,000 rows by total in {sort_ms:.0} ms; top {top}, next {next}"));
            if let Ok(snap) = std::env::var("SNAP") {
                snapshot_windows(&snap);
                // As the window server shows it (the sidebar's selection is
                // vibrancy, which in-process snapshots draw black).
                let png = capture_window(&w).and_then(|rep| unsafe { rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()) });
                if let Some(png) = png {
                    png.writeToFile_atomically(&objc2_foundation::NSString::from_str(&format!("{snap}/sql_browser-screen.png")), true);
                }
            }
            errors(w, b, dir);
        });
    }

    fn errors(w: Retained<NSWindow>, b: Browser, dir: PathBuf) {
        run_query(&w, b, "SELEC nothing");
        when(idle(b), 5000, move |_| {
            let status = untrack(|| b.status());
            let kept = results(&w).numberOfRows() == 1_000_000;
            check("query-error", status.contains("syntax error") && kept, format!("status {status:?}; the last result stays: {kept}"));
            cancel(w, b, dir);
        });
    }

    /// A slow query, then a quick one before it ends: the quick one shows
    /// and the slow one was interrupted.
    fn cancel(w: Retained<NSWindow>, b: Browser, dir: PathBuf) {
        let slow = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 500000000) SELECT count(*) FROM n";
        run_query(&w, b, slow);
        let start = Instant::now();
        let w2 = w.clone();
        spawn(|| std::thread::sleep(std::time::Duration::from_millis(200)), move |()| {
            run_query(&w2, b, "SELECT 6 * 7 AS answer");
            when(move || idle(b)() && titles(&results(&w2)) == ["answer"], 10_000, move |_| {
                let t = results(&w);
                let total = start.elapsed().as_secs_f64() * 1e3;
                check("cancel-stale", cell(&t, 0, 0) == "42" && total < 2000.0, format!("the newer query shows {:?} after {total:.0} ms (the slow one alone takes many seconds)", cell(&t, 0, 0)));
                writes(w, b, dir);
            });
        });
    }

    fn writes(w: Retained<NSWindow>, b: Browser, dir: PathBuf) {
        run_query(&w, b, "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT); INSERT INTO notes (body) VALUES ('first'), ('second'), (NULL);");
        when(move || idle(b)() && sidebar_titles(&b).contains(&"notes".to_string()), 5000, move |ms| {
            let status = untrack(|| b.status());
            check("write-refreshes-schema", status.starts_with("3 rows changed") && sidebar_titles(&b).contains(&"notes".to_string()), format!("status {status:?}; notes in the sidebar after {ms:.0} ms"));
            // Selecting a table in the sidebar shows its rows.
            b.selected.set(Some("table:notes".into()));
            let w2 = w.clone();
            when(move || idle(b)() && results(&w2).numberOfRows() == 3, 5000, move |_| {
                let t = results(&w);
                check(
                    "sidebar-select",
                    untrack(|| b.query.get()) == "SELECT * FROM \"notes\";" && titles(&t) == ["id", "body"] && cell(&t, 2, 1) == "NULL",
                    format!("query {:?}; columns {:?}; a NULL shows as {:?}", untrack(|| b.query.get()), titles(&t), cell(&t, 2, 1)),
                );
                let menu = can_perform_in(Some(&w), RUN);
                check("run-command", menu, "Run Query (menu and toolbar) is enabled in a database window".into());
                panes(w, b, dir);
            });
        });
    }

    /// Saves `w` as the window server shows it (needs screen recording).
    fn save_capture(w: &NSWindow, path: &str) {
        let png = capture_window(w).and_then(|rep| unsafe { rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()) });
        if let Some(png) = png {
            png.writeToFile_atomically(&objc2_foundation::NSString::from_str(path), true);
        }
    }

    /// Runs `f` once, after the run loop has handled pending work.
    fn then(f: impl FnOnce() + 'static) {
        spawn(|| std::thread::sleep(std::time::Duration::from_millis(300)), move |()| f());
    }

    /// The window's three-pane split (sidebar, query and results, inspector).
    fn outer(w: &NSWindow) -> Retained<NSSplitView> {
        all(w).into_iter().filter_map(|v| v.downcast::<NSSplitView>().ok()).find(|s| s.isVertical() && s.arrangedSubviews().count() == 3).unwrap()
    }

    fn pane_widths(w: &NSWindow) -> Vec<f64> {
        outer(w).arrangedSubviews().iter().map(|p| if p.isHidden() { 0.0 } else { p.frame().size.width.round() }).collect()
    }

    fn labels(w: &NSWindow) -> Vec<String> {
        all(w).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSTextField>().ok()).map(|f| f.stringValue().to_string()).collect()
    }

    /// The View menu item titled "… `what`", after AppKit validates the menu.
    fn view_item(what: &str) -> String {
        let app = objc2_app_kit::NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
        let menu = app.mainMenu().unwrap().itemWithTitle(&objc2_foundation::NSString::from_str("View")).unwrap().submenu().unwrap();
        menu.update();
        (0..menu.numberOfItems()).map(|i| menu.itemAtIndex(i).unwrap().title().to_string()).find(|t| t.ends_with(what)).unwrap_or_default()
    }

    /// The inspector, collapsing the sidebar and inspector (menu, toolbar
    /// command, a drag), and the history popover.
    fn panes(w: Retained<NSWindow>, b: Browser, dir: PathBuf) {
        let before = pane_widths(&w);
        check("three-panes", before.len() == 3 && before.iter().all(|&x| x > 50.0) && (before[2] - 260.0).abs() <= 2.0, format!("sidebar, content, inspector widths {before:?}"));
        // Select the second row of notes through AppKit, as a click would.
        let t = results(&w);
        t.selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(1), false);
        let w2 = w.clone();
        when(move || untrack(|| b.row.get()) == Some(1), 2000, move |_| {
            let shown = labels(&w2);
            check(
                "inspector-row",
                shown.contains(&"Row 2".to_string()) && shown.contains(&"body".to_string()) && shown.contains(&"second".to_string()),
                format!("the inspector shows {:?}", shown.iter().filter(|l| ["Row 2", "id", "body", "second"].contains(&l.as_str())).collect::<Vec<_>>()),
            );
            // The inspector: toolbar command, then again.
            let title = view_item("Inspector");
            perform_in(Some(&w2), super::TOGGLE_INSPECTOR);
            let w3 = w2.clone();
            then(move || {
                let hidden = pane_widths(&w3);
                let title_after = view_item("Inspector");
                perform_in(Some(&w3), super::TOGGLE_INSPECTOR);
                let w4 = w3.clone();
                let before = before.clone();
                then(move || {
                    let back = pane_widths(&w4);
                    check(
                        "inspector-toggle",
                        hidden[2] == 0.0 && hidden[1] > before[1] + 200.0 && (back[2] - before[2]).abs() <= 2.0 && title == "Hide Inspector" && title_after == "Show Inspector",
                        format!("hidden {hidden:?}; shown again {back:?}; menu {title:?} → {title_after:?}"),
                    );
                    sidebar_checks(w4, b, dir);
                });
            });
        });
    }

    fn sidebar_checks(w: Retained<NSWindow>, b: Browser, dir: PathBuf) {
        // ⌃⌘S through the menu bar hides the sidebar; the content keeps
        // the room, the inspector its width.
        let before = pane_widths(&w);
        let fired = menu_key("s", NSEventModifierFlags::Command | NSEventModifierFlags::Control);
        let w2 = w.clone();
        then(move || {
            let hidden = pane_widths(&w2);
            check(
                "sidebar-shortcut",
                fired && untrack(|| b.no_sidebar.get()) && hidden[0] == 0.0 && (hidden[2] - before[2]).abs() <= 2.0 && view_item("Sidebar") == "Show Sidebar",
                format!("⌃⌘S fired {fired}: {before:?} → {hidden:?}; menu now {:?}", view_item("Sidebar")),
            );
            b.no_sidebar.set(false);
            let w3 = w2.clone();
            then(move || {
                let back = pane_widths(&w3);
                // Dragging the divider shut collapses it and sets the signal.
                outer(&w3).setPosition_ofDividerAtIndex(0.0, 0);
                let w4 = w3.clone();
                then(move || {
                    let dragged = untrack(|| b.no_sidebar.get());
                    check(
                        "sidebar-drag-collapse",
                        (back[0] - before[0]).abs() <= 2.0 && dragged,
                        format!("reopened at {} (was {}); dragged shut → signal {dragged}", back[0], before[0]),
                    );
                    b.no_sidebar.set(false);
                    then(move || popover_checks(w4, b, dir));
                });
            });
        });
    }

    fn popover_windows() -> Vec<Retained<NSWindow>> {
        // AppKit's own popover windows (not in the app's `windows()`).
        let app = objc2_app_kit::NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
        app.windows().iter().filter(|w| w.isVisible() && w.class().name().to_string_lossy().contains("Popover")).collect()
    }

    fn popover_checks(w: Retained<NSWindow>, b: Browser, dir: PathBuf) {
        let live = live_signals();
        b.history_shown.set(true);
        let w2 = w.clone();
        when(|| !popover_windows().is_empty(), 3000, move |ms| {
            let pops = popover_windows();
            let buttons: Vec<Retained<objc2_app_kit::NSButton>> = pops
                .iter()
                .flat_map(|p| all(p))
                .filter_map(|v| v.downcast::<objc2_app_kit::NSButton>().ok())
                .collect();
            let titles: Vec<String> = buttons.iter().map(|b| b.title().to_string()).collect();
            check(
                "history-popover",
                pops.len() == 1 && titles.first().map(String::as_str) == Some("SELECT * FROM \"notes\";") && titles.len() >= 5,
                format!("shown after {ms:.0} ms with {} queries, newest {:?}", titles.len(), titles.first()),
            );
            if let Ok(snap) = std::env::var("SNAP") {
                save_capture(&w2, &format!("{snap}/sql_browser-panes.png"));
                if let Some(p) = pops.first() {
                    save_capture(p, &format!("{snap}/sql_browser-history.png"));
                }
            }
            // Choosing one puts it in the editor and closes the popover.
            let pick = buttons.iter().find(|b| b.title().to_string() == "SELECT * FROM orders").cloned();
            if let Some(p) = &pick {
                unsafe { p.performClick(None) };
            }
            when(|| popover_windows().is_empty(), 3000, move |_| {
                let freed = live_signals() <= live;
                check(
                    "history-pick",
                    pick.is_some() && untrack(|| b.query.get()) == "SELECT * FROM orders" && !untrack(|| b.history_shown.get()) && freed,
                    format!("query now {:?}; closed: {}; content freed: {freed}", untrack(|| b.query.get()), popover_windows().is_empty()),
                );
                // Escape closes it too, clearing the signal.
                b.history_shown.set(true);
                // (Escape once it has finished opening: during its animation
                // it can be ignored.)
                when(|| !popover_windows().is_empty(), 3000, move |_| then(move || {
                    let pop = popover_windows().pop();
                    // An Escape key press in the popover.
                    if let Some(p) = &pop {
                        let esc = objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                            objc2_app_kit::NSEventType::KeyDown,
                            objc2_foundation::NSPoint::ZERO,
                            NSEventModifierFlags::empty(),
                            0.0,
                            p.windowNumber(),
                            None,
                            &objc2_foundation::NSString::from_str("\u{1b}"),
                            &objc2_foundation::NSString::from_str("\u{1b}"),
                            false,
                            53,
                        )
                        .unwrap();
                        // Handed to the popover's content, as its window does
                        // with a key press there (a background app gets none).
                        let root = p.contentView().into_iter().chain(all(p)).find(|v| v.class().name().to_string_lossy() == "AppKitUiRootView");
                        if let Some(root) = root {
                            let _: () = unsafe { objc2::msg_send![&*root, keyDown: &*esc] };
                        }
                    }
                    when(|| popover_windows().is_empty(), 3000, move |_| {
                        check("popover-escape", !untrack(|| b.history_shown.get()) && popover_windows().is_empty(), format!("Escape closed it: signal {}", untrack(|| b.history_shown.get())));
                        chart_checks(w2, b, dir);
                    });
                }));
            });
        });
    }

    /// The chart pane's canvas, while it's shown.
    fn chart_view(w: &NSWindow) -> Option<Retained<NSView>> {
        all(w).into_iter().find(|v| canvas_drawing(v).is_some())
    }

    fn drawing(w: &NSWindow) -> Vec<String> {
        chart_view(w).and_then(|v| canvas_drawing(&v)).unwrap_or_default()
    }

    /// A mouse event over the chart at `x, y` (its own coordinates).
    fn mouse(view: &NSView, kind: objc2_app_kit::NSEventType, x: f64, y: f64) -> Retained<objc2_app_kit::NSEvent> {
        let at = view.convertPoint_toView(objc2_foundation::NSPoint::new(x, y), None);
        objc2_app_kit::NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            at,
            NSEventModifierFlags::empty(),
            0.0,
            view.window().unwrap().windowNumber(),
            None,
            0,
            0,
            0.0,
        )
        .unwrap()
    }

    /// Bars of a grouped query, a line through a million rows, the values
    /// on hover, and VoiceOver's summary.
    fn chart_checks(w: Retained<NSWindow>, b: Browser, dir: PathBuf) {
        perform_in(Some(&w), super::TOGGLE_CHART);
        run_query(&w, b, "SELECT substr(ordered_at, 1, 7) AS month, round(sum(total)) AS sales FROM orders GROUP BY month ORDER BY month");
        let w2 = w.clone();
        when(move || idle(b)() && drawing(&w2).iter().filter(|d| d.starts_with("fill rect") && d.ends_with("Blue")).count() == 72, 10_000, move |ms| {
            let d = drawing(&w);
            let bars = d.iter().filter(|d| d.starts_with("fill rect") && d.ends_with("Blue")).count();
            let label = chart_view(&w).and_then(|v| v.accessibilityLabel()).map(|l| l.to_string()).unwrap_or_default();
            check(
                "chart-bars",
                bars == 72 && untrack(|| !b.no_chart.get()) && d.iter().any(|t| t == "text \"2020-01\" 11pt") && label.starts_with("Bar chart, 72 bars, from "),
                format!("{bars} bars {ms:.0} ms after Run; VoiceOver: {label:?}; drew {:?}", d.iter().take(6).collect::<Vec<_>>()),
            );
            // Hovering a bar highlights it and shows its value.
            let view = chart_view(&w).unwrap();
            let size = view.frame().size;
            let _: () = unsafe { objc2::msg_send![&*view, mouseMoved: &*mouse(&view, objc2_app_kit::NSEventType::MouseMoved, size.width * 0.5, size.height * 0.5)] };
            let w2 = w.clone();
            then(move || {
                let d = drawing(&w2);
                let lit = d.iter().filter(|t| t.ends_with("Indigo")).count();
                let tip = d.iter().find(|t| t.starts_with("text \"202")  && t.contains(": ")).cloned();
                let _: () = unsafe { objc2::msg_send![&*view, mouseExited: &*mouse(&view, objc2_app_kit::NSEventType::MouseMoved, -5.0, -5.0)] };
                let w3 = w2.clone();
                then(move || {
                    let gone = !drawing(&w3).iter().any(|t| t.ends_with("Indigo"));
                    check("chart-hover", lit == 1 && tip.is_some() && gone, format!("hovering: {lit} bar highlighted, readout {tip:?}; leaving clears it: {gone}"));
                    if let Ok(snap) = std::env::var("SNAP") {
                        save_capture(&w3, &format!("{snap}/sql_browser-chart.png"));
                    }
                    line_check(w3, b, dir);
                });
            });
        });
    }

    fn line_check(w: Retained<NSWindow>, b: Browser, dir: PathBuf) {
        run_query(&w, b, "SELECT id, total FROM orders");
        let w2 = w.clone();
        let start = Instant::now();
        when(move || idle(b)() && drawing(&w2).iter().any(|d| d.starts_with("stroke path") && d.ends_with("Blue")), 20_000, move |ms| {
            let d = drawing(&w);
            let points: usize = d.iter().filter(|t| t.starts_with("stroke path") && t.ends_with("Blue")).filter_map(|t| t.split(' ').nth(2)?.parse::<usize>().ok()).sum();
            let width = chart_view(&w).map(|v| v.frame().size.width).unwrap_or(0.0);
            let label = chart_view(&w).and_then(|v| v.accessibilityLabel()).map(|l| l.to_string()).unwrap_or_default();
            let _ = start;
            check(
                "chart-line-million",
                points > 100 && (points as f64) <= width * 4.0 + 8.0 && label.starts_with("Line chart of total, 1000000 points"),
                format!("1,000,000 rows drawn as a {points}-point line ({width:.0} pt wide) {ms:.0} ms after Run; VoiceOver: {label:?}"),
            );
            transfer_checks(w, b, dir);
        });
    }

    fn png_size(png: &[u8]) -> Option<(u32, u32)> {
        (png.len() > 24 && png.starts_with(b"\x89PNG")).then(|| (u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap())))
    }

    /// Exporting: the chart as an image (copied, dragged out as a file),
    /// the results as CSV (quoting, a million rows, the dragged chip), and
    /// a database dropped on the window opening.
    fn transfer_checks(w: Retained<NSWindow>, b: Browser, dir: PathBuf) {
        // The line chart of a million rows, as a PNG and as a dragged file.
        let chart = chart_view(&w).unwrap();
        let png = canvas_png(&chart).unwrap_or_default();
        let size = png_size(&png);
        let dragged = canvas_drag(&chart).map(|item| fulfil(&item, &dir));
        let file = dragged.as_ref().and_then(|r| r.as_ref().ok()).cloned();
        let on_disk = file.as_ref().and_then(|f| std::fs::read(f).ok()).unwrap_or_default();
        let clip = Clipboard::private();
        clip.set_png(&png);
        check(
            "chart-export",
            size.is_some_and(|(pw, ph)| pw as f64 >= chart.frame().size.width && ph as f64 >= chart.frame().size.height)
                && file.as_ref().is_some_and(|f| f.ends_with("chart.png"))
                && on_disk == png
                && clip.png().as_deref() == Some(&png[..]),
            format!("PNG {size:?} ({} KB) for a {:.0}×{:.0} chart; dragged out as {:?}; on a pasteboard and back: {}", png.len() / 1024, chart.frame().size.width, chart.frame().size.height, file.as_ref().map(|f| f.file_name().unwrap().to_owned()), clip.png().is_some()),
        );
        perform_in(Some(&w), super::TOGGLE_CHART);
        // CSV quoting.
        run_query(&w, b, "SELECT 1 AS n, 'x,y' AS comma, 'say \"hi\"' AS quote, NULL AS empty, 'two\nlines' AS multi");
        let w2 = w.clone();
        when(move || idle(b)() && titles(&results(&w2)).len() == 5, 5000, move |_| {
            let r = untrack(|| b.results.get()).unwrap();
            let text = super::csv(&r);
            check(
                "csv-quoting",
                text == "n,comma,quote,empty,multi\n1,\"x,y\",\"say \"\"hi\"\"\",,\"two\nlines\"\n",
                format!("{text:?}"),
            );
            run_query(&w, b, "SELECT * FROM orders");
            let w3 = w.clone();
            when(move || idle(b)() && results(&w3).numberOfRows() == 1_000_000, 20_000, move |_| {
                let r = untrack(|| b.results.get()).unwrap();
                let path = dir.join("orders.csv");
                let t = Instant::now();
                let saved = super::save_csv(&r, &path);
                let ms = t.elapsed().as_secs_f64() * 1e3;
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let lines = text.lines().count();
                check(
                    "csv-export-million",
                    saved.is_ok() && lines == 1_000_001 && text.starts_with("id,customer_id,product_id,quantity,total,ordered_at\n1,"),
                    format!("1,000,000 rows written in {ms:.0} ms ({} MB, {lines} lines)", text.len() / 1_000_000),
                );
                // The chip: a promised results.csv with the same content.
                let chip = all(&w).into_iter().find(|v| canvas_drag(v).is_some_and(|d| matches!(&d, DragItem::Promise { name, .. } if name == "results.csv")));
                let promised = chip.as_ref().and_then(|c| canvas_drag(c)).map(|item| {
                    let sub = dir.join("drop");
                    let _ = std::fs::create_dir_all(&sub);
                    fulfil(&item, &sub)
                });
                let same = promised.as_ref().and_then(|r| r.as_ref().ok()).and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|t| t == text);
                check("csv-drag-chip", chip.is_some() && same, format!("the chip drags out results.csv, written on drop with the same {} MB", text.len() / 1_000_000));
                drop_checks(w, b, dir);
            });
        });
    }

    /// Files dropped on a document window: a database opens, other files
    /// aren't taken.
    fn drop_checks(w: Retained<NSWindow>, b: Browser, dir: PathBuf) {
        let other = dir.join("other.sqlite");
        {
            let conn = rusqlite::Connection::open(&other).unwrap();
            conn.execute_batch("CREATE TABLE things (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO things (name) VALUES ('one');").unwrap();
        }
        let text_file = dir.join("readme.txt");
        std::fs::write(&text_file, "hello").unwrap();
        let content = w.contentView().unwrap();
        let refused = {
            let clip = Clipboard::private();
            clip.set_files(&[text_file]);
            !drop_on_view(&content, clip.pasteboard())
        };
        let taken = {
            let clip = Clipboard::private();
            clip.set_files(std::slice::from_ref(&other));
            drop_on_view(&content, clip.pasteboard())
        };
        when(|| windows().iter().any(|x| x.title().to_string() == "other.sqlite"), 3000, move |ms| {
            let opened = windows().into_iter().find(|x| x.title().to_string() == "other.sqlite");
            check(
                "drop-opens-database",
                refused && taken && opened.is_some(),
                format!("a .txt refused: {refused}; other.sqlite taken: {taken} and opened in its own window after {ms:.0} ms: {}", opened.is_some()),
            );
            if let Some(o) = opened {
                close_window(&o);
            }
            let _ = b;
            scratch(w, dir);
        });
    }

    fn scratch(w: Retained<NSWindow>, dir: PathBuf) {
        let s = new_document().unwrap();
        let b = BROWSERS.with_borrow(|all| all.last().unwrap().0);
        run_query(&s, b, "CREATE TABLE t (x); INSERT INTO t VALUES (1), (2.5), ('three'); SELECT x, typeof(x) AS type FROM t;");
        let s2 = s.clone();
        when(move || idle(b)() && results(&s2).numberOfRows() == 3, 5000, move |_| {
            let t = results(&s);
            let got: Vec<String> = (0..3).map(|r| format!("{}:{}", cell(&t, r, 0), cell(&t, r, 1))).collect();
            check("scratch-database", s.title().to_string().starts_with("Untitled") && got == ["1:integer", "2.5:real", "three:text"], format!("{:?}: {got:?}", s.title()));
            let bad = dir.join("not.sqlite");
            std::fs::write(&bad, "hello").unwrap();
            let err = open_document(&bad).err().map(|e| e.to_string());
            check("not-sqlite", err.as_deref() == Some("not a SQLite database"), format!("{err:?}"));
            teardown(w, s, dir);
        });
    }

    /// Closing a window frees its state (signals and the connection).
    fn teardown(w: Retained<NSWindow>, s: Retained<NSWindow>, dir: PathBuf) {
        let live = live_signals();
        close_window(&s);
        spawn(|| std::thread::sleep(std::time::Duration::from_millis(200)), move |()| {
            let freed = live - live_signals();
            check("close-frees", freed > 5, format!("closing a window freed {freed} signals"));
            let _ = w;
            let _ = std::fs::remove_dir_all(&dir);
            let failed = FAILED.with(Cell::get);
            eprintln!("[check] done, {failed} failed");
            std::process::exit(if failed == 0 { 0 } else { 1 });
        });
    }
}
