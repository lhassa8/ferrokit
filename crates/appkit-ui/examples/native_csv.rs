//! F2.6 exit check: the CSV browser as a native app. Open a CSV (or the
//! 200,000-row sample), filter it from the toolbar, narrow it by a sidebar
//! group, sort, see a record's details in a sheet, keep notes per record.
//! `cargo run --release -p appkit-ui --example native_csv`; `CHECK=all`
//! runs its self-checks and measurements; `SNAP=<dir>` saves an image.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use appkit_ui::*;

/// Built on a background thread, so `Arc`, not `Rc`.
#[derive(Clone)]
struct Record {
    id: usize,
    fields: Arc<[String]>,
    /// All fields lowercased and joined, for one substring search per row.
    all: Arc<str>,
}

#[derive(Clone, PartialEq)]
struct Group(String);

impl Identifiable for Group {
    type Id = String;
    fn id(&self) -> String {
        self.0.clone()
    }
}

#[derive(Clone)]
struct Browser {
    data: Signal<Option<Rc<data::Data>>>,
    query: Signal<String>,
    group: Signal<Option<String>>,
    selected: Signal<Option<usize>>,
    shown: Signal<usize>,
    filter_ms: Signal<f64>,
    notes: TextModel,
    notes_by_record: Rc<RefCell<HashMap<usize, String>>>,
}

impl Browser {
    fn new() -> Self {
        Browser {
            data: Signal::new(None),
            query: Signal::new(String::new()),
            group: Signal::new(None),
            selected: Signal::new(None),
            shown: Signal::new(0),
            filter_ms: Signal::new(0.0),
            notes: TextModel::new(ViewId(2), ""),
            notes_by_record: Rc::default(),
        }
    }

    fn open(&self, window: &objc2_app_kit::NSWindow) {
        let b = self.clone();
        open_panel(window, OpenOptions { extensions: vec!["csv".into()], ..Default::default() }, move |paths| {
            if let Some(path) = paths.and_then(|p| p.into_iter().next()) {
                b.load(Some(path));
            }
        });
    }

    /// Reads and parses on a background thread.
    fn load(&self, path: Option<std::path::PathBuf>) {
        let b = self.clone();
        spawn(move || data::load(path), move |d| {
            b.group.set(None);
            b.selected.set(None);
            b.data.set(d.ok().map(Rc::new));
        });
    }

    fn details(&self, window: &objc2_app_kit::NSWindow, row: usize) {
        let Some(d) = self.data.get_untracked() else { return };
        let r = &d.records[row];
        let rows: Vec<_> = d.headers.iter().zip(r.fields.iter()).map(|(h, v)| (text(format!("{h}:")), text(v.clone()))).collect();
        let handle: Rc<RefCell<Option<SheetHandle>>> = Rc::default();
        let h = handle.clone();
        let close = button("Close", move || h.borrow().iter().for_each(SheetHandle::close)).default_action();
        let view = vstack((grid(rows).align_column(0, Align::End), hstack((spacer(), close)))).padding(20.0);
        *handle.borrow_mut() = Some(sheet(window, Window::new("Details").size(360.0, 300.0).view(view), || {}));
    }
}

fn table_view(b: &Browser, d: Rc<data::Data>) -> AnyView {
    let model = Rc::new(TableModel::new(ViewId(1), d.columns()));
    let (b1, b2, b3, m1, m2) = (b.clone(), b.clone(), b.clone(), model.clone(), model.clone());
    // Re-filter when the query or the group changes.
    let filter = watch(move || {
        let t = Instant::now();
        let needle = b1.query.get().to_lowercase();
        let group = b1.group.get();
        let rows = d.filter(&needle, group.as_deref());
        b1.shown.set(rows.len());
        m1.set_rows(rows);
        b1.filter_ms.set(t.elapsed().as_secs_f64() * 1e3);
    });
    let view = table(&model)
        .on_select(move |rows| b2.selected.set(rows.first().map(|&i| m2.rows()[i].id)))
        .on_activate({
            let m = model.clone();
            move |row| b3.details(&windows()[0], m.rows()[row].id)
        })
        .row_menu("Show Details", {
            let (b, m) = (b.clone(), model.clone());
            move |row| b.details(&windows()[0], m.rows()[row].id)
        });
    AnyView::new(vstack((filter, view)).spacing(0.0))
}

fn content(b: &Browser) -> impl View {
    let (b1, b2, b3, b4) = (b.clone(), b.clone(), b.clone(), b.clone());
    // The notes editor shows the selected record's note and saves edits to it.
    let show_note = watch(move || {
        let note = b3.selected.get().and_then(|id| b3.notes_by_record.borrow().get(&id).cloned());
        b3.notes.set_text(&note.unwrap_or_default());
        b3.notes.set_editable(b3.selected.get().is_some());
    });
    split((
        sidebar(move || b1.data.get().map(|d| d.group_tree()).unwrap_or_default(), |g: &Group| g.0.clone())
            .icon(|g: &Group| (!g.0.starts_with("By ")).then(|| "mappin".to_string()))
            .selection(&b.group),
        vstack((
            dynamic(move || match b2.data.get() {
                Some(d) => {
                    let key = Rc::as_ptr(&d) as usize;
                    table_view(&b2, d).id(key)
                }
                None => AnyView::new(text("Open a CSV file (⌘O) or wait for the sample.")).id(0usize),
            }),
            label(move || {
                let total = b4.data.with(|d| d.as_ref().map_or(0, |d| d.records.len()));
                format!("Showing {} of {} rows · filter {:.0} ms", b4.shown.get(), total, b4.filter_ms.get())
            }),
            divider(),
            text_editor(&b.notes).on_change({
                let b = b.clone();
                move |t| {
                    if let Some(id) = b.selected.get_untracked() {
                        b.notes_by_record.borrow_mut().insert(id, t);
                    }
                }
            })
            .height(110.0)
            .accessibility_label("Notes"),
            show_note,
        ))
        .padding(8.0),
    ))
    .position(180.0)
}

fn main() {
    let b = Browser::new();
    let (b1, b2) = (b.clone(), b.clone());
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    App::new("CSV Browser")
        .window(
            Window::new("Orders")
                .size(1000.0, 640.0)
                .toolbar(vec![
                    toolbar_button("open", "Open", "folder", move || b1.open(&windows()[0])),
                    toolbar_space(),
                    toolbar_search("search", &b.query),
                ])
                .view(content(&b)),
        )
        .on_launch(move || {
            if checks {
                self_checks::run(b2.clone());
            } else {
                b2.load(None);
            }
        })
        .run();
}

// ---- data (CSV parsing and the sample, from the Phase 1 demo) ----

mod data {
    use appkit_ui::{Column, TreeNode};

    use super::{Group, Record};

    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum Kind {
        Text,
        Number { decimals: usize },
        Bool,
    }

    pub struct Data {
        pub headers: Vec<String>,
        pub kinds: Vec<Kind>,
        pub records: Vec<Record>,
        /// The column used for sidebar groups, and its distinct values.
        pub groups: Option<(usize, Vec<String>)>,
        pub load_ms: f64,
    }

    impl Data {
        pub fn columns(&self) -> Vec<Column<Record>> {
            self.headers
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    let id = format!("c{i}");
                    match self.kinds[i] {
                        Kind::Text => Column::text(&id, h, move |r: &Record| r.fields[i].clone()),
                        Kind::Number { decimals } => {
                            Column::number(&id, h, move |r: &Record| r.fields[i].parse().unwrap_or(0.0)).decimals(decimals).width(80.0)
                        }
                        Kind::Bool => Column::check(&id, h, move |r: &Record| is_true(&r.fields[i])),
                    }
                })
                .collect()
        }

        pub fn filter(&self, needle: &str, group: Option<&str>) -> Vec<Record> {
            let gi = self.groups.as_ref().map(|g| g.0);
            self.records
                .iter()
                .filter(|r| group.is_none_or(|g| gi.is_some_and(|i| r.fields[i] == g)) && r.all.contains(needle))
                .cloned()
                .collect()
        }

        pub fn group_tree(&self) -> Vec<TreeNode<Group>> {
            let Some((i, values)) = &self.groups else { return Vec::new() };
            vec![TreeNode::branch(
                Group(format!("By {}", self.headers[*i])),
                values.iter().map(|v| TreeNode::leaf(Group(v.clone()))).collect(),
            )]
        }
    }

    fn is_true(v: &str) -> bool {
        matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1")
    }

    pub fn load(path: Option<std::path::PathBuf>) -> Result<Data, String> {
        let t = std::time::Instant::now();
        let rows = match path {
            Some(p) => parse_csv(&std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?),
            None => sample(),
        };
        let mut rows = rows.into_iter();
        let headers = rows.next().ok_or("empty file")?;
        let body: Vec<Vec<String>> = rows.filter(|r| r.len() == headers.len()).collect();
        let kinds = infer_kinds(&headers, &body);
        // Sidebar groups: a column with a grouping name, else the text column
        // with the fewest distinct values (2–50).
        let distinct = |i: usize| {
            let mut v: Vec<String> = body.iter().map(|r| r[i].clone()).collect();
            v.sort();
            v.dedup();
            v
        };
        let candidates: Vec<(usize, Vec<String>)> = (0..headers.len())
            .filter(|&i| kinds[i] == Kind::Text)
            .map(|i| (i, distinct(i)))
            .filter(|(_, v)| (2..=50).contains(&v.len()))
            .collect();
        let named = ["city", "category", "region", "country", "status"];
        let groups = candidates
            .iter()
            .find(|(i, _)| named.contains(&headers[*i].to_lowercase().as_str()))
            .or_else(|| candidates.iter().min_by_key(|(_, v)| v.len()))
            .cloned();
        let records = body
            .into_iter()
            .enumerate()
            .map(|(id, fields)| {
                let all = fields.iter().map(|f| f.to_lowercase()).collect::<Vec<_>>().join("\u{1f}");
                Record { id, fields: fields.into(), all: all.into() }
            })
            .collect();
        Ok(Data { headers, kinds, records, groups, load_ms: t.elapsed().as_secs_f64() * 1e3 })
    }

    fn infer_kinds(headers: &[String], rows: &[Vec<String>]) -> Vec<Kind> {
        let sample = &rows[..rows.len().min(5_000)];
        (0..headers.len())
            .map(|i| {
                let values: Vec<&str> = sample.iter().map(|r| r[i].trim()).filter(|v| !v.is_empty()).collect();
                if values.is_empty() {
                    Kind::Text
                } else if values.iter().all(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "false" | "yes" | "no")) {
                    Kind::Bool
                } else if values.iter().all(|v| v.parse::<f64>().is_ok()) {
                    Kind::Number { decimals: if values.iter().any(|v| v.contains('.')) { 2 } else { 0 } }
                } else {
                    Kind::Text
                }
            })
            .collect()
    }

    /// RFC 4180 CSV: quoted fields, doubled quotes, CRLF or LF line ends.
    fn parse_csv(text: &str) -> Vec<Vec<String>> {
        let (mut rows, mut row, mut field, mut quoted) = (Vec::new(), Vec::new(), String::new(), false);
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match (quoted, c) {
                (true, '"') if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                (true, '"') => quoted = false,
                (true, c) => field.push(c),
                (false, '"') if field.is_empty() => quoted = true,
                (false, ',') => row.push(std::mem::take(&mut field)),
                (false, '\r') => {}
                (false, '\n') => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                (false, c) => field.push(c),
            }
        }
        if !field.is_empty() || !row.is_empty() {
            row.push(field);
            rows.push(row);
        }
        rows
    }

    /// 200,000 sample orders, deterministic (same generator as Phase 1).
    fn sample() -> Vec<Vec<String>> {
        const CUSTOMERS: &[&str] = &["Aiko Tanaka", "Ben Okafor", "Chloé Martin", "Dmitri Ivanov", "Elif Yılmaz", "Femi Adeyemi", "Greta Lindqvist", "Hugo Moreau", "Isabel Costa", "Jonas Weber", "Kenji Sato", "Lucía Romero", "Mei Chen", "Nils Berg", "Olivia Brown", "Pedro Alves"];
        const CITIES: &[&str] = &["Paris", "Tokyo", "Berlin", "Lagos", "São Paulo", "Oslo", "Toronto", "Seoul", "Madrid", "Melbourne", "Nairobi", "Istanbul"];
        const PRODUCTS: &[&str] = &["Desk lamp", "Notebook", "Fountain pen", "Monitor arm", "Keyboard", "Headphones", "Webcam", "Chair mat", "USB hub", "Mouse"];
        let mut rows = vec![["Order", "Date", "Customer", "City", "Product", "Total", "Quantity", "Paid"].map(String::from).to_vec()];
        let mut seed: u64 = 7;
        for i in 0..200_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let r = (seed >> 33) as usize;
            let quantity = 1 + r % 9;
            rows.push(vec![
                format!("{}", 100_000 + i),
                format!("2026-{:02}-{:02}", 1 + (r >> 4) % 12, 1 + (r >> 8) % 28),
                CUSTOMERS[(r >> 12) % CUSTOMERS.len()].into(),
                CITIES[(r >> 16) % CITIES.len()].into(),
                PRODUCTS[(r >> 20) % PRODUCTS.len()].into(),
                format!("{:.2}", quantity as f64 * (5.0 + ((r >> 3) % 400) as f64 / 4.0)),
                quantity.to_string(),
                if (r >> 28).is_multiple_of(4) { "false" } else { "true" }.into(),
            ]);
        }
        rows
    }
}

// ---- self-checks and measurements ----

mod self_checks {
    use std::cell::Cell;
    use std::rc::Rc;

    use appkit_ui::testing::{footprint_mb, peak_footprint_mb, post_edit, snapshot_windows};
    use appkit_ui::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSOutlineView, NSSearchToolbarItem, NSTableView, NSTextField, NSView};
    use objc2_foundation::NSString;

    use super::Browser;

    thread_local! {
        static FAILED: Cell<u32> = const { Cell::new(0) };
    }

    fn check(name: &str, pass: bool, note: String) {
        eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
        if !pass {
            FAILED.with(|f| f.set(f.get() + 1));
        }
    }

    /// Runs `f` after two run-loop passes (model events arrive on the pass
    /// after AppKit's callback).
    fn after(f: impl Fn() + 'static) {
        let f = Rc::new(f);
        later(move || {
            let f = f.clone();
            later(move || f())
        });
    }

    fn all_views(v: &NSView, out: &mut Vec<Retained<NSView>>) {
        for c in v.subviews().iter() {
            out.push(c.clone());
            all_views(&c, out);
        }
    }

    fn views() -> Vec<Retained<NSView>> {
        let mut out = Vec::new();
        all_views(&windows()[0].contentView().unwrap(), &mut out);
        out
    }

    /// The orders table (not the sidebar, which is an NSOutlineView).
    fn orders() -> Retained<NSTableView> {
        views()
            .into_iter()
            .filter_map(|v| v.downcast::<NSTableView>().ok())
            .find(|t| t.downcast_ref::<NSOutlineView>().is_none())
            .unwrap()
    }

    pub fn run(b: Browser) {
        let f0 = footprint_mb();
        b.load(None);
        let t = std::time::Instant::now();
        wait_loaded(b, f0, t);
    }

    fn wait_loaded(b: Browser, f0: f64, t: std::time::Instant) {
        later(move || {
            if b.data.get_untracked().is_none() && t.elapsed().as_secs() < 20 {
                std::thread::sleep(std::time::Duration::from_millis(5));
                wait_loaded(b.clone(), f0, t);
                return;
            }
            let shown_ms = t.elapsed().as_secs_f64() * 1e3;
            let d = b.data.get_untracked().unwrap();
            let rows = orders().numberOfRows();
            check(
                "load",
                // Timing is informational on CI runners (shared virtual machines).
                rows == 200_000 && d.headers.len() == 8 && (shown_ms < 1000.0 || std::env::var_os("CI").is_some()),
                format!("200k rows parsed in {:.0} ms on a background thread; in the table {shown_ms:.0} ms after Open (budget 1000); {rows} rows; footprint {f0:.0} → {:.0} MB", d.load_ms, footprint_mb()),
            );
            interact(b.clone());
        });
    }

    fn interact(b: Browser) {
        let w = windows()[0].clone();
        w.makeKeyAndOrderFront(None);
        // Toolbar search.
        let search = w.toolbar().unwrap().items().iter().find_map(|i| i.downcast::<NSSearchToolbarItem>().ok()).unwrap().searchField();
        let mut times = Vec::new();
        for q in ["p", "pa", "par", "pari", "paris"] {
            post_edit(&search, q);
            times.push(b.filter_ms.get_untracked().round());
        }
        let paris = b.shown.get_untracked();
        let worst = times.iter().copied().fold(0.0, f64::max);
        // Timing is informational on CI runners (shared virtual machines).
        let ci = std::env::var_os("CI").is_some();
        check("search", paris == 16_632 && orders().numberOfRows() == 16_632 && (worst < 100.0 || ci), format!("'paris' → {paris} rows; per keystroke {times:?} ms (budget 100)"));

        // Sidebar group (plus the search: no Tokyo order mentions Paris).
        batch(|| b.group.set(Some("Tokyo".into())));
        let both = b.shown.get_untracked();
        post_edit(&search, "");
        let tokyo = b.shown.get_untracked();
        let outline = views().into_iter().find_map(|v| v.downcast::<NSOutlineView>().ok()).unwrap();
        let sel_row = outline.selectedRow();
        let selected_title = (sel_row >= 0)
            .then(|| outline.viewAtColumn_row_makeIfNecessary(0, sel_row, true))
            .flatten()
            .map(|v| {
                let mut out = Vec::new();
                all_views(&v, &mut out);
                out.into_iter().filter_map(|x| x.downcast::<NSTextField>().ok()).map(|f| f.stringValue().to_string()).collect::<Vec<_>>()
            });
        check(
            "sidebar-group",
            both == 0 && tokyo == 16_429 && selected_title.as_ref().is_some_and(|t| t.iter().any(|s| s == "Tokyo")),
            format!("Tokyo + 'paris' → {both}; Tokyo → {tokyo}; sidebar shows {selected_title:?} selected"),
        );

        // Sort by Total, descending (as a header click).
        let tv = orders();
        let t = std::time::Instant::now();
        let desc = objc2_foundation::NSSortDescriptor::sortDescriptorWithKey_ascending(Some(&NSString::from_str("c5")), false);
        tv.setSortDescriptors(&objc2_foundation::NSArray::from_retained_slice(&[desc]));
        tv.displayIfNeeded();
        let sort_ms = t.elapsed().as_secs_f64() * 1e3;
        let first = tv
            .viewAtColumn_row_makeIfNecessary(tv.columnWithIdentifier(&NSString::from_str("c5")), 0, true)
            .and_then(|v| v.downcast::<NSTextField>().ok())
            .map(|f| f.stringValue().to_string());
        check("sort", first.as_deref() == Some("942.75") && sort_ms < 300.0, format!("16,429 rows by Total, descending, in {sort_ms:.1} ms (budget 300); first {first:?}"));

        // Details: select row 0, Return → sheet with the record; Close.
        tv.selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(0), false);
        let ev = objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            objc2_app_kit::NSEventType::KeyDown, objc2_foundation::NSPoint::new(0.0, 0.0), objc2_app_kit::NSEventModifierFlags::empty(), 0.0,
            w.windowNumber(), None, &NSString::from_str("\r"), &NSString::from_str("\r"), false, 36,
        ).unwrap();
        tv.keyDown(&ev);
        after(move || {
            let w = windows()[0].clone();
            let sheet = w.attachedSheet();
            let labels: Vec<String> = sheet.as_ref().map(|s| {
                let mut out = Vec::new();
                all_views(&s.contentView().unwrap(), &mut out);
                out.into_iter().filter_map(|v| v.downcast::<NSTextField>().ok()).map(|f| f.stringValue().to_string()).collect()
            }).unwrap_or_default();
            let ok = labels.iter().any(|l| l == "Customer:") && labels.iter().any(|l| l == "942.75");
            if let Some(s) = &sheet {
                let mut out = Vec::new();
                all_views(&s.contentView().unwrap(), &mut out);
                if let Some(close) = out.into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSButton>().ok()).find(|b| b.title().to_string() == "Close") {
                    unsafe { close.performClick(None) };
                }
            }
            check("details-sheet", sheet.is_some() && ok, format!("Return on the top row opened a sheet with {} labels incl. Customer: and 942.75: {ok}", labels.len()));
            notes(b.clone());
        });
    }

    fn notes(b: Browser) {
        let first = b.selected.get_untracked();
        b.notes.replace_selection("Call back on Monday");
        let b2 = b.clone();
        after(move || {
            let b = b2.clone();
            let tv = orders();
            tv.selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(1), false);
            let b3 = b.clone();
            after(move || {
                let b = b3.clone();
                let other_empty = b.notes.text().is_empty() && b.selected.get_untracked() != first;
                orders().selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(0), false);
                let b4 = b.clone();
                after(move || {
                    let b = b4.clone();
                    let back = b.notes.text();
                    check(
                        "notes",
                        first.is_some() && other_empty && back == "Call back on Monday",
                        format!("note on {first:?}; another row shows an empty note: {other_empty}; back → {back:?}"),
                    );
                    measure(b.clone());
                });
            });
        });
    }

    fn measure(b: Browser) {
        let w = windows()[0].clone();
        batch(|| b.group.set(None));
        // Resize: the layout pass with the full app on screen.
        let n0 = layout_passes();
        let mut total = Vec::new();
        for k in 0..200 {
            let t = std::time::Instant::now();
            w.setContentSize(objc2_foundation::NSSize::new(900.0 + (k % 40) as f64 * 8.0, 640.0));
            w.displayIfNeeded();
            total.push(t.elapsed().as_secs_f64() * 1e3);
        }
        let times = layout_times_us();
        let new = ((layout_passes() - n0) as usize).min(times.len());
        let mut layout: Vec<f64> = times[times.len() - new..].iter().map(|us| us / 1e3).collect();
        layout.sort_by(|a, b| a.total_cmp(b));
        total.sort_by(|a, b| a.total_cmp(b));
        let p = |v: &[f64], q: usize| v[(v.len() - 1) * q / 100];
        check(
            "resize",
            // Timing is informational on CI runners (shared virtual machines).
            p(&layout, 95) < 2.0 || std::env::var_os("CI").is_some(),
            format!("{} layout passes: p50 {:.2} ms, p95 {:.2} ms (budget 2); with AppKit's display p50 {:.1} ms, p95 {:.1} ms", layout.len(), p(&layout, 50), p(&layout, 95), p(&total, 50), p(&total, 95)),
        );
        // Scroll: 100 jumps through 200k rows, with display.
        let tv = orders();
        let mut worst = 0.0f64;
        let t = std::time::Instant::now();
        for row in (0..200_000).step_by(2_000) {
            let s = std::time::Instant::now();
            tv.scrollRowToVisible(row);
            tv.displayIfNeeded();
            worst = worst.max(s.elapsed().as_secs_f64() * 1e3);
        }
        check("scroll", worst < 16.7, format!("100 jumps: {:.0} ms total, worst {worst:.1} ms", t.elapsed().as_secs_f64() * 1e3));
        let problems = appkit_ui::testing::ax_audit(&windows()[0]);
        check("accessibility-audit", problems.is_empty(), format!("controls VoiceOver can't name: {problems:?}"));
        let (now, peak) = (footprint_mb(), peak_footprint_mb());
        check("memory", peak < 400.0, format!("footprint now {now:.0} MB, peak {peak:.0} MB (Iced demo 420, WaterUI 1,300–2,300)"));
        let finish = || {
            let failed = FAILED.with(Cell::get);
            eprintln!("[check] done, {failed} failed");
            std::process::exit(if failed == 0 { 0 } else { 1 });
        };
        let Ok(dir) = std::env::var("SNAP") else { return finish() };
        snapshot_windows(&dir);
        // On screen: rows scrolled half a row under the header (which should
        // blur them), as the window server shows it.
        tv.scrollRowToVisible(0);
        if let Some(clip) = tv.enclosingScrollView().map(|s| s.contentView()) {
            let mut b = clip.bounds();
            b.origin.y += 30.0;
            clip.setBoundsOrigin(b.origin);
        }
        let w = tv.window().unwrap();
        spawn(|| std::thread::sleep(std::time::Duration::from_millis(500)), move |()| {
            if let Some(rep) = appkit_ui::testing::capture_window(&w) {
                let png = unsafe { rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()) };
                png.unwrap().writeToFile_atomically(&objc2_foundation::NSString::from_str(&format!("{dir}/screen.png")), true);
            }
            finish();
        });
    }
}
