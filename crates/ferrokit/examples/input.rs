//! Q1: real input. A small form (fields, an editor, a checkbox, a segmented
//! control, a table, a sidebar, a clickable card, a menu command) driven
//! by clicks and keystrokes that take the user's path through AppKit:
//! hit-testing, the first responder, the field editor, key equivalents.
//! `cargo run -p ferrokit --example input`; `CHECK=all` runs the checks.
//! Window-server input (CGEvents) runs too when the terminal is trusted
//! for Accessibility, and is reported as skipped when it isn't.

use std::rc::Rc;

use ferrokit::*;

const SAVE: Command = Command::new("save-form");

#[derive(Clone, PartialEq)]
struct Place {
    id: String,
    name: String,
}

impl Identifiable for Place {
    type Id = String;
    fn id(&self) -> String {
        self.id.clone()
    }
}

#[derive(Clone, Copy)]
struct Form {
    name: Signal<String>,
    email: Signal<String>,
    notes: Signal<String>,
    subscribe: Signal<bool>,
    plan: Signal<&'static str>,
    row: Signal<Option<usize>>,
    place: Signal<Option<String>>,
    card_clicks: Signal<u32>,
    saves: Signal<u32>,
}

const CITIES: [(&str, &str); 5] = [("Lisbon", "Portugal"), ("Osaka", "Japan"), ("Toronto", "Canada"), ("Nairobi", "Kenya"), ("Berlin", "Germany")];

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    let f = Form {
        name: Signal::new(String::new()),
        email: Signal::new(String::new()),
        notes: Signal::new(String::new()),
        subscribe: Signal::new(false),
        plan: Signal::new("free"),
        row: Signal::new(None),
        place: Signal::new(None),
        card_clicks: Signal::new(0),
        saves: Signal::new(0),
    };
    let editor = TextModel::new(ViewId(1), "");
    let cities = Rc::new(TableModel::new(ViewId(2), vec![
        Column::text("city", "City", |&(c, _): &(&str, &str)| c.to_string()),
        Column::text("country", "Country", |&(_, c): &(&str, &str)| c.to_string()),
    ]));
    cities.set_rows(CITIES.to_vec());
    let places = || {
        vec![TreeNode::branch(Place { id: "places".into(), name: "Places".into() }, vec![
            TreeNode::leaf(Place { id: "home".into(), name: "Home".into() }),
            TreeNode::leaf(Place { id: "work".into(), name: "Work".into() }),
            TreeNode::leaf(Place { id: "gym".into(), name: "Gym".into() }),
        ])]
    };
    if checks {
        self_checks::STATE.with(|s| *s.borrow_mut() = Some(f));
    }
    let form = vstack((
        grid([
            (AnyView::new(text("Name:")), AnyView::new(text_field(&f.name).placeholder("Your name").accessibility_label("Name"))),
            (AnyView::new(text("Email:")), AnyView::new(text_field(&f.email).placeholder("you@example.com").accessibility_label("Email"))),
            (AnyView::new(text("Plan:")), AnyView::new(segmented(&f.plan, vec![("Free", "free"), ("Plus", "plus"), ("Team", "team")]).accessibility_label("Plan"))),
            (AnyView::new(text("")), AnyView::new(checkbox("Send me the newsletter", &f.subscribe))),
        ])
        .align_column(0, Align::End)
        .spacing(8.0, 10.0),
        text_editor(&editor).bind(&f.notes).accessibility_label("Notes").height(90.0),
        hstack((
            table(&cities).on_select(move |rows| f.row.set(rows.first().copied())).accessibility_label("Cities").height(130.0),
            sidebar(places, |p: &Place| p.name.clone()).icon(|p: &Place| (p.id != "places").then(|| "mappin".to_string())).selection(&f.place).width(160.0),
        ))
        .spacing(10.0),
        hstack((
            hstack((image("hand.tap"), text(move || format!("Clicked {} times", f.card_clicks.get())), spacer()))
                .padding(12.0)
                .background(Color::QuaternaryFill)
                .corner_radius(10.0)
                .on_click(move || f.card_clicks.update(|n| *n += 1))
                .accessibility_label("Card"),
            button("Save", move || f.saves.update(|n| *n += 1)).default_action(),
        ))
        .spacing(10.0),
    ))
    .spacing(14.0)
    .padding(20.0);
    App::new("Input")
        .window(Window::new("Sign up").size(640.0, 620.0).view(form).on_command(SAVE, move || f.saves.update(|n| *n += 10)))
        .menu("File", vec![menu_command("Save Draft", SAVE).key("s")])
        .on_launch(move || {
            if checks {
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(500)), move |()| self_checks::run(editor.clone()));
            }
        })
        .run();
}

mod self_checks {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use ferrokit::testing::input::{self, Key, Mod};
    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindow};

    use super::Form;

    thread_local! {
        pub static STATE: RefCell<Option<Form>> = const { RefCell::new(None) };
    }

    type Check = Rc<dyn Fn(&str, bool, String)>;

    fn all(v: &NSView, out: &mut Vec<Retained<NSView>>) {
        for s in v.subviews().iter() {
            out.push(s.clone());
            all(&s, out);
        }
    }

    fn views(w: &NSWindow) -> Vec<Retained<NSView>> {
        let mut out = Vec::new();
        all(&w.contentView().unwrap(), &mut out);
        out
    }

    /// The view VoiceOver names `name`.
    fn named(w: &NSWindow, name: &str) -> Option<Retained<NSView>> {
        views(w).into_iter().find(|v| objc2_app_kit::NSAccessibility::accessibilityLabel(&**v).is_some_and(|l| l.to_string() == name))
    }

    fn first<T: objc2::DowncastTarget>(w: &NSWindow) -> Option<Retained<T>> {
        views(w).into_iter().find_map(|v| v.downcast::<T>().ok())
    }

    fn later(ms: u64, f: impl FnOnce() + 'static) {
        spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
    }

    /// Runs `steps` in order, `ms` apart: each sends input, the next looks at it.
    fn steps(ms: u64, mut list: Vec<Box<dyn FnOnce()>>) {
        if list.is_empty() {
            return;
        }
        let step = list.remove(0);
        step();
        later(ms, move || steps(ms, list));
    }

    /// The first responder's text-field or text-view owner.
    fn focused(w: &NSWindow) -> String {
        let Some(r) = w.firstResponder() else { return "none".into() };
        let Ok(view) = r.downcast::<NSView>() else { return "other".into() };
        // A field's editor is a text view whose delegate is the field.
        let field = view.clone().downcast::<objc2_app_kit::NSTextView>().ok().and_then(|tv| tv.delegate()).and_then(|d| d.downcast::<objc2_app_kit::NSTextField>().ok());
        match field {
            Some(f) => objc2_app_kit::NSAccessibility::accessibilityLabel(&*f).map(|l| l.to_string()).unwrap_or_default(),
            None => view.class().name().to_string_lossy().into_owned(),
        }
    }

    pub fn run(editor: TextModel) {
        let w = windows().into_iter().find(|w| w.title().to_string() == "Sign up").unwrap();
        let f = STATE.with(|s| s.borrow_mut().take()).unwrap();
        let failed = Rc::new(Cell::new(0));
        let check: Check = {
            let failed = failed.clone();
            Rc::new(move |name: &str, pass: bool, note: String| {
                eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
                failed.set(failed.get() + u32::from(!pass));
            })
        };
        input::ready(&w);
        later(300, move || {
            let ready = input::ready(&w);
            check("input-ready", ready, format!("app active and window key: {ready}"));
            if !ready {
                return finish(failed);
            }
            let (c, w2) = (check.clone(), w.clone());
            let name = named(&w, "Name").unwrap();
            let (c2, c3, c4, c5, c6, c7, c8, c9, c10, c11) = (c.clone(), c.clone(), c.clone(), c.clone(), c.clone(), c.clone(), c.clone(), c.clone(), c.clone(), c.clone());
            let (w3, w4, w5, w6, w7, w8, w9, w10, w11) = (w.clone(), w.clone(), w.clone(), w.clone(), w.clone(), w.clone(), w.clone(), w.clone(), w.clone());
            let editor2 = editor.clone();
            steps(250, vec![
                // A click into the name field, then typing (capitals with ⇧).
                Box::new(move || {
                    input::click(&name);
                }),
                Box::new(move || {
                    input::type_text(&w2, "Ada Lovelace");
                }),
                Box::new(move || {
                    c("type-field", untrack(|| f.name.get()) == "Ada Lovelace", format!("{:?}", untrack(|| f.name.get())));
                    // Delete takes off the last letter; Tab moves on to Email.
                    input::key(&w3, Key::Delete, &[]);
                    input::key(&w3, Key::Tab, &[]);
                }),
                Box::new(move || {
                    let after = focused(&w4);
                    c2("delete-and-tab", untrack(|| f.name.get()) == "Ada Lovelac" && after == "Email", format!("name {:?}; focus on {after:?}", untrack(|| f.name.get())));
                    input::type_text(&w4, "ada@example.com");
                    // ⌘A then typing replaces it all.
                    input::key(&w4, Key::Char('a'), &[Mod::Command]);
                    input::type_text(&w4, "a@b.c");
                }),
                Box::new(move || {
                    c3("select-all-replace", untrack(|| f.email.get()) == "a@b.c", format!("{:?}", untrack(|| f.email.get())));
                    // The editor: a click, two lines, then ⌘Z.
                    if let Some(ed) = named(&w5, "Notes") {
                        input::click(&ed);
                    }
                }),
                Box::new(move || {
                    input::type_text(&w6, "First line\nSecond");
                }),
                Box::new(move || {
                    let typed = untrack(|| f.notes.get());
                    c4("type-editor", typed == "First line\nSecond" && editor2.text() == typed, format!("{typed:?}"));
                    input::key(&w7, Key::Char('z'), &[Mod::Command]);
                }),
                Box::new(move || {
                    let undone = untrack(|| f.notes.get());
                    c5("undo-typing", undone.len() < "First line\nSecond".len(), format!("after ⌘Z {undone:?}"));
                    // The checkbox, a segment, the card and Save, by clicks.
                    if let Some(b) = views(&w8).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSButton>().ok()).find(|b| b.title().to_string() == "Send me the newsletter") {
                        input::click(&b);
                    }
                    if let Some(seg) = first::<objc2_app_kit::NSSegmentedControl>(&w8) {
                        let width = seg.bounds().size.width;
                        input::click_at(&seg, width * 5.0 / 6.0, seg.bounds().size.height / 2.0);
                    }
                    if let Some(card) = named(&w8, "Card").filter(|v| v.class().name().to_string_lossy() == "FerrokitInteraction") {
                        input::click(&card);
                    }
                    if let Some(save) = views(&w8).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSButton>().ok()).find(|b| b.title().to_string() == "Save") {
                        input::click(&save);
                    }
                }),
                Box::new(move || {
                    let (sub, plan, cards, saves) = untrack(|| (f.subscribe.get(), f.plan.get(), f.card_clicks.get(), f.saves.get()));
                    c6("click-controls", sub && plan == "team" && cards == 1 && saves == 1, format!("checkbox {sub}, plan {plan}, card {cards}, Save {saves}"));
                    // ⌘S through the menu's key equivalent.
                    input::key(&w9, Key::Char('s'), &[Mod::Command]);
                    // The table: a click on the second row, then ↓.
                    if let Some(t) = views(&w9).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSTableView>().ok()).find(|t| t.downcast_ref::<objc2_app_kit::NSOutlineView>().is_none()) {
                        let r = t.rectOfRow(1);
                        input::click_at(&t, r.origin.x + 20.0, r.origin.y + r.size.height / 2.0);
                    }
                }),
                Box::new(move || {
                    c7("key-equivalent", untrack(|| f.saves.get()) == 11, format!("saves {}", untrack(|| f.saves.get())));
                    let clicked = untrack(|| f.row.get());
                    input::key(&w10, Key::Down, &[]);
                    later(200, move || {
                        c8("table-click-and-arrow", clicked == Some(1) && untrack(|| f.row.get()) == Some(2), format!("click {clicked:?}, then ↓ {:?}", untrack(|| f.row.get())));
                    });
                }),
                Box::new(move || {
                    // The sidebar: a click on Home, then ↓ to Work.
                    if let Some(o) = first::<objc2_app_kit::NSOutlineView>(&w11) {
                        let row = (0..o.numberOfRows()).find(|&r| o.itemAtRow(r).is_some() && o.levelForRow(r) == 1).unwrap_or(1);
                        let r = o.rectOfRow(row);
                        input::click_at(&o, r.origin.x + 40.0, r.origin.y + r.size.height / 2.0);
                    }
                }),
                Box::new({
                    let w = w.clone();
                    move || {
                        let clicked = untrack(|| f.place.get());
                        input::key(&w, Key::Down, &[]);
                        let (c, w) = (c9.clone(), w.clone());
                        later(200, move || {
                            c("sidebar-click-and-arrow", clicked.as_deref() == Some("home") && untrack(|| f.place.get()).as_deref() == Some("work"), format!("click {clicked:?}, then ↓ {:?}", untrack(|| f.place.get())));
                            server_input(w, f, c10, c11, failed);
                        });
                    }
                }),
            ]);
        });
    }

    /// Window-server input, when the process may post it.
    fn server_input(w: Retained<NSWindow>, f: Form, check: Check, _check2: Check, failed: Rc<Cell<u32>>) {
        if !input::server_available() {
            eprintln!("[check] server-input SKIP this process isn't trusted for Accessibility (System Settings › Privacy & Security › Accessibility)");
            return finish(failed);
        }
        f.name.set(String::new());
        if let Some(name) = named(&w, "Name") {
            input::server_click(&name);
        }
        later(300, move || {
            input::server_type("Grace");
            later(400, move || {
                check("server-input", untrack(|| f.name.get()) == "Grace", format!("{:?}", untrack(|| f.name.get())));
                finish(failed);
            });
        });
    }

    fn finish(failed: Rc<Cell<u32>>) {
        eprintln!("[check] done, {} failed", failed.get());
        std::process::exit(if failed.get() == 0 { 0 } else { 1 });
    }
}
