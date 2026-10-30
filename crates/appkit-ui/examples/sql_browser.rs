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
        }
    }

    fn run(&self) {
        let sql = self.query.get_untracked();
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
            return "Running…".into();
        }
        match self.results.state() {
            Load::Loading => "Running…".into(),
            Load::Failed(e) => format!("Error: {e}"),
            Load::Ready(r) if r.names.is_empty() && r.changed > 0 => format!("{} rows changed · {:.0} ms", r.changed, r.ms),
            Load::Ready(r) if r.names.is_empty() => format!("Done · {:.0} ms", r.ms),
            Load::Ready(r) => format!("{} rows · query {:.0} ms · table {:.0} ms", thousands(r.rows), r.ms, self.fill_ms.get()),
        }
    }
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
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
    BROWSERS.with_borrow_mut(|all| all.push(b));
    let editor = TextModel::new(ViewId(1), "");
    editor.set_font(Some("Menlo"), 12.0);
    editor.set_spell_checking(false);
    let model = std::rc::Rc::new(TableModel::new(ViewId(2), Vec::new()));
    let m = model.clone();
    // A new result replaces the columns and rows of the same native table.
    let fill = watch(move || {
        let Some(r) = b.results.get() else { return };
        let start = Instant::now();
        m.set_columns(columns(&r));
        m.set_rows((0..r.rows as u32).collect());
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
        sidebar(move || b.schema.get().unwrap_or_default(), |i: &Item| i.title.clone())
            .icon(|i: &Item| i.icon.map(String::from))
            .selection(&b.selected),
        split((
            text_editor(&editor).bind(&b.query).accessibility_label("Query"),
            vstack((
                table(&model).accessibility_label("Results"),
                label(move || b.status()).padding(6.0),
                fill,
                select,
                first,
            ))
            .spacing(0.0),
        ))
        .stacked()
        .position(110.0),
    ))
    .position(220.0)
    .on_command(RUN, move || b.run())
}

fn find<'a>(nodes: &'a [TreeNode<Item>], id: &str) -> Option<&'a Item> {
    nodes.iter().find_map(|n| if n.value.id == id { Some(&n.value) } else { find(&n.children, id) })
}

thread_local! {
    /// Every window's state, for the self-checks.
    static BROWSERS: std::cell::RefCell<Vec<Browser>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn database() -> DocumentType<Db> {
    DocumentType::new(&["sqlite", "db", "sqlite3"], Db::scratch, database_view)
        .read(Db::open)
        .toolbar(|_| vec![toolbar_command("run", "Run", "play.fill", RUN)])
        .size(1100.0, 700.0)
}

fn main() {
    let check = std::env::var("CHECK").unwrap_or_default();
    let file = std::env::args().nth(1).map(std::path::PathBuf::from);
    App::new("SQL Browser")
        .documents(database())
        .menu("Query", vec![menu_command("Run Query", RUN).key("r")])
        .on_launch(move || {
            if check == "all" {
                self_checks::run();
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

    use appkit_ui::testing::{ax_audit, capture_window, footprint_mb, snapshot_windows};
    use appkit_ui::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSOutlineView, NSSplitView, NSTableView, NSView};
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
        let b = BROWSERS.with_borrow(|all| *all.last().unwrap());
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
                scratch(w, dir);
            });
        });
    }

    fn scratch(w: Retained<NSWindow>, dir: PathBuf) {
        let s = new_document().unwrap();
        let b = BROWSERS.with_borrow(|all| *all.last().unwrap());
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
