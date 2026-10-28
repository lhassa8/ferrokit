//! CSV browser: the M2/M3 demo. A real NSTableView inside an Iced app, with an
//! Iced filter field, an Iced dropdown that opens over the table, a details
//! modal, File → Open through a native sheet, and a native notes editor
//! (NSTextView: spell check, IME, find, undo) for the selected record.
//!
//!   cargo run --release -p iced-appkit --example csv_browser [file.csv]
//!
//! Without a file it generates 200,000 sample orders.
//! Env: DEMO_AUTO=1 runs a short scripted tour (filter, sort, open a record).
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use iced::widget::{button, center, column, container, opaque, pick_list, row, stack, text, text_input};
use iced::{Color, Element, Length, Point, Size, Subscription, Task, Theme, window};
use iced_appkit::table::{Column, Sort, TableModel};
use iced_appkit::text::TextModel;
use iced_appkit::{Event, ViewId, host};

const FILTER: &str = "filter";
const SAMPLE_ROWS: usize = 200_000;

/// One CSV record. `id` is its position in the full data set. For filtering,
/// `lower` holds the fields lowercased once and `all` all of them joined
/// (one substring search per row instead of one per field).
#[derive(Debug, Clone)]
struct Record {
    id: usize,
    fields: Rc<[String]>,
    lower: Rc<[String]>,
    all: Rc<str>,
}

impl Record {
    fn new(id: usize, fields: Vec<String>) -> Self {
        let lower: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
        let all = lower.join("\u{1f}");
        Record { id, fields: fields.into(), lower: lower.into(), all: all.into() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Number,
    Bool,
}

struct Data {
    name: String,
    headers: Vec<String>,
    kinds: Vec<Kind>,
    records: Vec<Record>,
    load_time: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SearchIn {
    All,
    Column(usize, String),
}

impl std::fmt::Display for SearchIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchIn::All => write!(f, "All columns"),
            SearchIn::Column(_, name) => write!(f, "{name}"),
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Opened(window::Id),
    Attached(bool),
    Native(Event),
    Filter(String),
    SearchIn(SearchIn),
    Activated(usize),
    Sorted(Sort),
    Toggled(usize, String, bool),
    CloseDetails,
    Selected(Vec<usize>),
    NotesChanged(String),
    Open,
    Loaded(Result<(String, Vec<Vec<String>>, Duration), String>),
    Auto(u32),
}

struct App {
    data: Option<Data>,
    model: Option<TableModel<Record>>,
    filter: String,
    search_in: SearchIn,
    sort: Option<Sort>,
    details: Option<usize>,
    filter_time: Duration,
    error: Option<String>,
    /// Each data set gets a new table (its columns differ).
    next_table: u64,
    notes: TextModel,
    /// Notes per record id, and the record the editor shows.
    notes_by_record: std::collections::HashMap<usize, String>,
    notes_for: Option<usize>,
}

const NOTES: ViewId = ViewId(1_000);
const NO_RECORD: &str = "Select a row to write notes about it.";

thread_local! {
    /// A path chosen in the Open panel, picked up on the next native event.
    static CHOSEN: RefCell<Option<String>> = const { RefCell::new(None) };
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let app = App {
            data: None,
            model: None,
            filter: String::new(),
            search_in: SearchIn::All,
            sort: None,
            details: None,
            filter_time: Duration::ZERO,
            error: None,
            next_table: 1,
            notes: {
                let notes = TextModel::new(NOTES, NO_RECORD);
                notes.set_editable(false);
                notes.set_font(None, 13.0);
                notes
            },
            notes_by_record: std::collections::HashMap::new(),
            notes_for: None,
        };
        (app, Task::none())
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Opened(id) => return iced_appkit::attach(id, Message::Attached),
            Message::Attached(ok) => {
                assert!(ok, "attach failed");
                host::install_edit_menu();
                let path = std::env::args().nth(1);
                let mut tasks = vec![load(path)];
                if std::env::var_os("DEMO_AUTO").is_some() {
                    tasks.push(Task::perform(async { std::thread::sleep(Duration::from_millis(1500)) }, |_| Message::Auto(0)));
                }
                return Task::batch(tasks);
            }
            Message::Native(event) => {
                if let Some(path) = CHOSEN.with_borrow_mut(Option::take) {
                    return Task::batch([iced_appkit::update(&event), load(Some(path))]);
                }
                return iced_appkit::update(&event);
            }
            Message::Loaded(Ok((name, rows, load_time))) => self.set_data(name, rows, load_time),
            Message::Loaded(Err(error)) => self.error = Some(error),
            Message::Filter(filter) => {
                self.filter = filter;
                self.apply_filter();
            }
            Message::SearchIn(search_in) => {
                self.search_in = search_in;
                self.apply_filter();
            }
            Message::Activated(row) => self.details = Some(row),
            Message::Sorted(sort) => self.sort = Some(sort),
            Message::Toggled(row, column, value) => self.toggle(row, &column, value),
            Message::CloseDetails => self.details = None,
            Message::Selected(rows) => self.show_notes(rows.first().copied()),
            Message::NotesChanged(text) => {
                if let Some(id) = self.notes_for {
                    self.notes_by_record.insert(id, text);
                }
            }
            Message::Open => open_panel(),
            Message::Auto(step) => return self.auto(step),
        }
        Task::none()
    }

    fn set_data(&mut self, name: String, mut rows: Vec<Vec<String>>, load_time: Duration) {
        if rows.is_empty() {
            self.error = Some("empty file".into());
            return;
        }
        let headers = rows.remove(0);
        let kinds = infer_kinds(&headers, &rows);
        let records: Vec<Record> = rows.into_iter().enumerate().map(|(id, f)| Record::new(id, f)).collect();
        let columns = headers
            .iter()
            .zip(&kinds)
            .enumerate()
            .map(|(i, (title, kind))| {
                let id = format!("c{i}");
                match kind {
                    Kind::Number => Column::number(&id, title, move |r: &Record| r.fields.get(i).and_then(|v| v.parse().ok()).unwrap_or(f64::NAN))
                        .decimals(if records_have_fraction(i) { 2 } else { 0 })
                        .width(90.0),
                    Kind::Bool => Column::check(&id, title, move |r: &Record| r.fields.get(i).is_some_and(|v| is_true(v))),
                    Kind::Text => Column::text(&id, title, move |r: &Record| r.fields.get(i).cloned().unwrap_or_default()).width(150.0),
                }
            })
            .collect();
        // A new model per data set (its columns differ); the old one's native
        // view is removed when it's dropped.
        let model = TableModel::new(ViewId(self.next_table), columns);
        self.next_table += 1;
        model.set_rows(records.clone());
        self.model = Some(model);
        self.data = Some(Data { name, headers, kinds, records, load_time });
        self.filter.clear();
        self.search_in = SearchIn::All;
        self.sort = None;
        self.error = None;
    }

    /// Points the notes editor at a record (by model row), or at nothing.
    fn show_notes(&mut self, row: Option<usize>) {
        let record = row.and_then(|r| self.model.as_ref()?.rows().get(r).map(|rec| rec.id));
        if record == self.notes_for {
            return;
        }
        self.notes_for = record;
        match record {
            Some(id) => {
                self.notes.set_text(self.notes_by_record.get(&id).map_or("", String::as_str));
                self.notes.set_editable(true);
            }
            None => {
                self.notes.set_text(NO_RECORD);
                self.notes.set_editable(false);
            }
        }
    }

    fn apply_filter(&mut self) {
        let (Some(data), Some(model)) = (&self.data, &self.model) else { return };
        let t = Instant::now();
        let needle = self.filter.to_lowercase();
        let matches = |r: &Record| -> bool {
            if needle.is_empty() {
                return true;
            }
            let hit = |v: &String| v.contains(needle.as_str());
            match &self.search_in {
                SearchIn::All => r.all.contains(needle.as_str()),
                SearchIn::Column(i, _) => r.lower.get(*i).is_some_and(hit),
            }
        };
        let rows: Vec<Record> = data.records.iter().filter(|r| matches(r)).cloned().collect();
        let matched = t.elapsed();
        model.set_rows(rows);
        if std::env::var_os("DEMO_AUTO").is_some() {
            eprintln!("[demo] match {matched:?}, set_rows {:?}", t.elapsed() - matched);
        }
        self.show_notes(None);
        self.filter_time = t.elapsed();
    }

    fn toggle(&mut self, row: usize, column: &str, value: bool) {
        let (Some(data), Some(model)) = (&mut self.data, &self.model) else { return };
        let Some(i) = column.strip_prefix('c').and_then(|i| i.parse::<usize>().ok()) else { return };
        let old = model.rows()[row].clone();
        let mut fields = old.fields.to_vec();
        fields[i] = if value { "true".into() } else { "false".into() };
        let record = Record::new(old.id, fields);
        data.records[record.id] = record.clone();
        model.update_row(row, record);
    }

    /// Scripted tour for unattended runs: filter, narrow the search column,
    /// sort by a number column, open a record, close it.
    fn auto(&mut self, step: u32) -> Task<Message> {
        let next = |ms| Task::perform(async move { std::thread::sleep(Duration::from_millis(ms)) }, move |_| Message::Auto(step + 1));
        match step {
            0 => {
                self.filter = "paris".into();
                self.apply_filter();
                eprintln!("[demo] filter 'paris': {} rows in {:?}", self.model.as_ref().map_or(0, |m| m.len()), self.filter_time);
                next(800)
            }
            1 => {
                if let Some(model) = &self.model {
                    model.set_sort(Some(Sort { column: "c5".into(), ascending: false }));
                    self.sort = model.sort();
                    if let Some(top) = model.row_at(0) {
                        model.select(&[top]);
                    }
                }
                next(800)
            }
            2 => {
                self.details = self.model.as_ref().and_then(|m| m.selection().first().copied());
                eprintln!("[demo] details for row {:?}", self.details);
                next(std::env::var("DEMO_HOLD_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500))
            }
            3 => {
                self.details = None;
                let top = self.model.as_ref().and_then(|m| m.selection().first().copied());
                self.show_notes(top);
                self.notes.replace_selection("Call the customer about delivery. Recieved twice?");
                next(800)
            }
            4 => {
                eprintln!("[demo] notes for {:?}: {:?}", self.notes_for, self.notes_by_record.get(&self.notes_for.unwrap_or(usize::MAX)));
                next(500)
            }
            _ => {
                eprintln!("[demo] tour done ({} notes)", self.notes_by_record.len());
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let toolbar = {
            let mut options = vec![SearchIn::All];
            if let Some(data) = &self.data {
                options.extend(data.headers.iter().enumerate().map(|(i, h)| SearchIn::Column(i, h.clone())));
            }
            row![
                button("Open CSV…").on_press(Message::Open),
                text_input("Filter rows…", &self.filter).id(FILTER).on_input(Message::Filter).width(Length::Fill),
                text("in").size(13),
                pick_list(options, Some(self.search_in.clone()), Message::SearchIn).width(180),
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center)
        };
        let status = match (&self.data, &self.model) {
            (Some(data), Some(model)) => format!(
                "{}: showing {} of {} rows{}{} · {} selected · loaded in {:.0?}, filter {:.0?}",
                data.name,
                model.len(),
                data.records.len(),
                self.sort.as_ref().map_or(String::new(), |s| {
                    let title = s.column.strip_prefix('c').and_then(|i| i.parse::<usize>().ok()).and_then(|i| data.headers.get(i)).cloned().unwrap_or_default();
                    format!(" · sorted by {title} {}", if s.ascending { "↑" } else { "↓" })
                }),
                if self.filter.is_empty() { String::new() } else { format!(" matching “{}”", self.filter) },
                model.selection().len(),
                data.load_time,
                self.filter_time,
            ),
            _ => self.error.clone().unwrap_or_else(|| "Loading…".into()),
        };
        let table: Element<'_, Message> = match &self.model {
            Some(model) => iced_appkit::table(model)
                .on_select(Message::Selected)
                .on_activate(Message::Activated)
                .on_sort(Message::Sorted)
                .on_toggle(Message::Toggled)
                .into(),
            None => center(text("Loading…")).into(),
        };
        let notes_title = match (self.notes_for, &self.data) {
            (Some(id), Some(data)) => format!("Notes · {} {}", data.headers.first().map_or("", String::as_str), data.records[id].fields.first().map_or("", String::as_str)),
            _ => "Notes".to_string(),
        };
        let notes = column![
            text(notes_title).size(13),
            iced_appkit::text_editor(&self.notes).on_change(Message::NotesChanged),
            text(format!("{} records with notes", self.notes_by_record.values().filter(|n| !n.is_empty()).count())).size(11),
        ]
        .spacing(6)
        .width(280);
        let base = column![toolbar, row![table, notes].spacing(10), text(status).size(12)].spacing(10).padding(14);
        let content: Element<'_, Message> = match (self.details, &self.data, &self.model) {
            (Some(row), Some(data), Some(model)) => {
                let record = model.rows().get(row).cloned();
                let mut fields = column![text("Record").size(20)].spacing(6);
                if let Some(record) = record {
                    for ((h, v), kind) in data.headers.iter().zip(record.fields.iter()).zip(&data.kinds) {
                        let v = if *kind == Kind::Bool { if is_true(v) { "Yes" } else { "No" } } else { v.as_str() };
                        fields = fields.push(row![text(h.clone()).width(120).size(13), text(v.to_string()).size(13)].spacing(10));
                    }
                }
                fields = fields.push(button("Close").on_press(Message::CloseDetails));
                let dialog = container(fields).padding(24).width(420).style(container::rounded_box);
                let dim = container(center(dialog)).width(Length::Fill).height(Length::Fill).style(|_: &Theme| container::Style {
                    background: Some(Color { a: 0.45, ..Color::BLACK }.into()),
                    ..Default::default()
                });
                stack![base, iced_appkit::cover(opaque(dim))].into()
            }
            _ => base.into(),
        };
        iced_appkit::root(content).into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![window::open_events().map(Message::Opened), iced_appkit::events().map(Message::Native)];
        if self.details.is_some() {
            subs.push(iced::keyboard::listen().filter_map(|event| match event {
                iced::keyboard::Event::KeyPressed { key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape), .. } => {
                    Some(Message::CloseDetails)
                }
                _ => None,
            }));
        }
        Subscription::batch(subs)
    }
}

fn is_true(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1")
}

thread_local! {
    static FRACTION_COLUMNS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

fn records_have_fraction(i: usize) -> bool {
    FRACTION_COLUMNS.with_borrow(|f| f.contains(&i))
}

/// A column is a number column if every non-empty value parses as a number,
/// a checkbox column if every value is true/false/yes/no.
fn infer_kinds(headers: &[String], rows: &[Vec<String>]) -> Vec<Kind> {
    let sample = &rows[..rows.len().min(5_000)];
    let mut fractions = Vec::new();
    let kinds = (0..headers.len())
        .map(|i| {
            let values: Vec<&str> = sample.iter().filter_map(|r| r.get(i)).map(|v| v.trim()).filter(|v| !v.is_empty()).collect();
            if values.is_empty() {
                Kind::Text
            } else if values.iter().all(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "false" | "yes" | "no")) {
                Kind::Bool
            } else if values.iter().all(|v| v.parse::<f64>().is_ok()) {
                if values.iter().any(|v| v.contains('.')) {
                    fractions.push(i);
                }
                Kind::Number
            } else {
                Kind::Text
            }
        })
        .collect();
    FRACTION_COLUMNS.with_borrow_mut(|f| *f = fractions);
    kinds
}

/// RFC 4180 CSV: quoted fields, doubled quotes, CRLF or LF line ends.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
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

/// 200,000 sample orders, deterministic.
fn sample() -> Vec<Vec<String>> {
    const CUSTOMERS: &[&str] = &["Aiko Tanaka", "Ben Okafor", "Chloé Martin", "Dmitri Ivanov", "Elif Yılmaz", "Femi Adeyemi", "Greta Lindqvist", "Hugo Moreau", "Isabel Costa", "Jonas Weber", "Kenji Sato", "Lucía Romero", "Mei Chen", "Nils Berg", "Olivia Brown", "Pedro Alves"];
    const CITIES: &[&str] = &["Paris", "Tokyo", "Berlin", "Lagos", "São Paulo", "Oslo", "Toronto", "Seoul", "Madrid", "Melbourne", "Nairobi", "Istanbul"];
    const PRODUCTS: &[&str] = &["Desk lamp", "Notebook", "Fountain pen", "Monitor arm", "Keyboard", "Headphones", "Webcam", "Chair mat", "USB hub", "Mouse"];
    let mut rows = vec![["Order", "Date", "Customer", "City", "Product", "Total", "Quantity", "Paid"].map(String::from).to_vec()];
    let mut seed: u64 = 7;
    for i in 0..SAMPLE_ROWS {
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

/// Loads a CSV file (or the sample) off the main thread.
fn load(path: Option<String>) -> Task<Message> {
    Task::perform(
        async move {
            let t = Instant::now();
            match path {
                Some(path) => {
                    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
                    let name = std::path::Path::new(&path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned());
                    Ok((name, parse_csv(&text), t.elapsed()))
                }
                None => Ok(("Sample orders".to_string(), sample(), t.elapsed())),
            }
        },
        Message::Loaded,
    )
}

/// File → Open as a sheet on the window (no nested modal run loop).
fn open_panel() {
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    let Some(window) = host::content_view().and_then(|v| v.window()) else { return };
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    let panel_ref = panel.clone();
    let handler = block2::RcBlock::new(move |response: isize| {
        if response == NSModalResponseOK
            && let Some(path) = panel_ref.URLs().firstObject().and_then(|u| u.path()) {
                CHOSEN.with_borrow_mut(|c| *c = Some(path.to_string()));
                host::request_redraw();
            }
    });
    panel.beginSheetModalForWindow_completionHandler(&window, &handler);
}

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("CSV browser · iced-appkit")
        .theme(|_: &App| iced_appkit::theme::system())
        .subscription(App::subscription)
        .window(window::Settings { size: Size::new(1100.0, 760.0), position: window::Position::Specific(Point::new(160.0, 30.0)), ..Default::default() })
        .run()
}
