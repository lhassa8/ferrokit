//! G3 exit check: a grid browser. The apps in a folder with their Finder
//! icons, filtered by a search field; or 100,000 colour swatches. Select
//! with the mouse or arrow keys; double-click or Return opens.
//! `cargo run --release -p appkit-ui --example gallery [folder]`;
//! `CHECK=all` runs its self-checks (and opens nothing).

use std::path::PathBuf;

use appkit_ui::*;

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    File(PathBuf),
    Swatch(u32),
}

impl Identifiable for Entry {
    type Id = String;
    fn id(&self) -> String {
        match self {
            Entry::File(p) => p.to_string_lossy().into_owned(),
            Entry::Swatch(n) => format!("swatch {n}"),
        }
    }
}

fn describe(e: &Entry) -> CollectionItem {
    match e {
        Entry::File(p) => {
            let name = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            CollectionItem::new(name, ItemImage::FileIcon(p.clone()))
        }
        Entry::Swatch(n) => {
            let hue = (*n % 360) as f64 / 360.0;
            let (r, g, b) = hsv(hue, 0.55 + 0.4 * ((n / 360) % 2) as f64, 0.9);
            CollectionItem::new(format!("#{n}"), ItemImage::Color(Color::rgb(r, g, b))).subtitle(format!("{:.0}°", hue * 360.0))
        }
    }
}

fn hsv(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

fn files(folder: &PathBuf) -> Vec<Entry> {
    let mut list: Vec<PathBuf> = std::fs::read_dir(folder).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    list.retain(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')));
    list.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    list.into_iter().map(Entry::File).collect()
}

#[derive(Clone)]
struct Gallery {
    mode: Signal<u8>,
    query: Signal<String>,
    selected: Signal<Vec<String>>,
    opened: Signal<Vec<String>>,
    all: std::rc::Rc<Vec<Entry>>,
}

impl Gallery {
    fn items(&self) -> Vec<Entry> {
        if self.mode.get() == 1 {
            return (0..100_000).map(Entry::Swatch).collect();
        }
        let q = self.query.get().to_lowercase();
        self.all.iter().filter(|e| q.is_empty() || describe(e).title.to_lowercase().contains(&q)).cloned().collect()
    }
}

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|c| c == "all");
    let folder = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/System/Applications"));
    let g = Gallery {
        mode: Signal::new(0),
        query: Signal::new(String::new()),
        selected: Signal::new(Vec::new()),
        opened: Signal::new(Vec::new()),
        all: std::rc::Rc::new(files(&folder)),
    };
    let (g1, g2, g3) = (g.clone(), g.clone(), g.clone());
    let grid = collection(move || g1.items(), describe).item_size(110.0, 104.0).selection(&g.selected).multiple().on_activate(move |e| {
        g2.opened.update(|o| o.push(e.id()));
        if let (Entry::File(p), false) = (&e, checks) {
            let _ = std::process::Command::new("open").arg(p).spawn();
        }
    });
    let status = label(move || {
        let n = g3.selected.with(Vec::len);
        if n == 0 { String::new() } else { format!("{n} selected") }
    });
    App::new("Gallery")
        .window(
            Window::new("Gallery").size(760.0, 560.0).view(
                vstack((
                    hstack((
                        segmented(&g.mode, vec![("Apps", 0u8), ("Swatches", 1)]).accessibility_label("Show"),
                        spacer(),
                        status,
                        search_field(&g.query).placeholder("Filter").accessibility_label("Filter").width(220.0),
                    ))
                    .padding(8.0),
                    grid.accessibility_label("Items"),
                )),
            ),
        )
        .on_launch(move || {
            if checks {
                self_checks::run(g.clone(), folder.clone());
            }
        })
        .run();
}

// ---- self-checks (not part of the app code count) ----

mod self_checks {
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::time::Instant;

    use appkit_ui::testing::{ax_audit, ax_elements, heap_in_use_mb};
    use appkit_ui::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSCollectionView, NSCollectionViewScrollPosition, NSEvent, NSEventModifierFlags, NSEventType, NSIndexPathNSCollectionViewAdditions, NSView};
    use objc2_foundation::{NSIndexPath, NSPoint, NSSet, NSString};

    use super::{Entry, Gallery};

    thread_local! {
        static FAILED: Cell<u32> = const { Cell::new(0) };
    }

    fn check(name: &str, pass: bool, note: String) {
        eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
        if !pass {
            FAILED.with(|f| f.set(f.get() + 1));
        }
    }

    fn grid() -> Retained<NSCollectionView> {
        fn find(v: &NSView) -> Option<Retained<NSCollectionView>> {
            for s in v.subviews().iter() {
                if let Ok(c) = s.clone().downcast::<NSCollectionView>() {
                    return Some(c);
                }
                if let Some(c) = find(&s) {
                    return Some(c);
                }
            }
            None
        }
        find(&windows()[0].contentView().unwrap()).unwrap()
    }

    fn selected_rows(cv: &NSCollectionView) -> Vec<usize> {
        let mut r: Vec<usize> = cv.selectionIndexPaths().iter().map(|p| p.item() as usize).collect();
        r.sort_unstable();
        r
    }

    /// A key press sent straight to the grid (as AppKit delivers one).
    fn key(cv: &NSCollectionView, code: u16, chars: &str) {
        let e = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(0.0, 0.0),
            NSEventModifierFlags::empty(),
            0.0,
            cv.window().map_or(0, |w| w.windowNumber()),
            None,
            &NSString::from_str(chars),
            &NSString::from_str(chars),
            false,
            code,
        )
        .unwrap();
        cv.keyDown(&e);
    }

    fn then(f: impl FnOnce() + 'static) {
        spawn(|| std::thread::sleep(std::time::Duration::from_millis(150)), move |()| f());
    }

    pub fn run(g: Gallery, folder: PathBuf) {
        let cv = grid();
        let expected = std::fs::read_dir(&folder).map(|d| d.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).count()).unwrap_or(0);
        let shown = cv.numberOfItemsInSection(0) as usize;
        check("load", expected > 5 && shown == expected, format!("{folder:?}: {shown} items shown of {expected} entries"));

        // Arrow keys move the selection (AppKit's own handling) → the signal.
        cv.window().unwrap().makeFirstResponder(Some(&cv));
        let first = NSSet::from_retained_slice(&[NSIndexPath::indexPathForItem_inSection(0, 0)]);
        cv.selectItemsAtIndexPaths_scrollPosition(&first, NSCollectionViewScrollPosition::None);
        key(&cv, 124, "\u{f703}");
        then(move || {
            let after_key = g.selected.get_untracked();
            let second = match &g.all[1] {
                Entry::File(p) => p.to_string_lossy().into_owned(),
                _ => String::new(),
            };
            check("select-keys", after_key == vec![second.clone()] && selected_rows(&cv) == [1], format!("→ moved the selection to item 1; signal {:?}", after_key.iter().map(|s| s.rsplit('/').next().unwrap_or("")).collect::<Vec<_>>()));
            // Signal → selection (applied when this pass's batch ends).
            let third = g.all[2].id_string();
            batch(|| g.selected.set(vec![third.clone()]));
            then(move || {
                check("select-signal", selected_rows(&cv) == [2], format!("signal → selected rows {:?}", selected_rows(&cv)));
                // Keyed update: filter so item 2 moves; it stays selected.
                let name = super::describe(&g.all[2]).title;
                let q: String = name.chars().take(3).collect();
                batch(|| g.query.set(q.clone()));
                then(move || {
                    let rows = selected_rows(&cv);
                    let now_at = (0..cv.numberOfItemsInSection(0) as usize).find(|&i| g.items_at(i) == Some(third.clone()));
                    check(
                        "keyed-update",
                        now_at.is_some() && rows == vec![now_at.unwrap()] && g.selected.get_untracked() == vec![third.clone()],
                        format!("filter {q:?}: {} items; {name:?} now at {now_at:?}, selected rows {rows:?}", cv.numberOfItemsInSection(0)),
                    );
                    // Return activates the selection.
                    key(&cv, 36, "\r");
                    let opened = g.opened.get_untracked();
                    check("activate-return", opened == vec![third.clone()], format!("Return → opened {:?}", opened.iter().map(|s| s.rsplit('/').next().unwrap_or("")).collect::<Vec<_>>()));
                    batch(|| g.query.set(String::new()));
                    then(move || accessibility(g, cv));
                });
            });
        });
    }

    fn accessibility(g: Gallery, cv: Retained<NSCollectionView>) {
        let w = cv.window().unwrap();
        let problems = ax_audit(&w);
        let names: Vec<String> = ax_elements(&w).into_iter().filter(|e| e.role == "AXImage").map(|e| e.name).collect();
        let first = super::describe(&g.all[0]).title;
        check("accessibility", problems.is_empty() && names.first() == Some(&first) && names.len() >= 4, format!("unnamed {problems:?}; items read as {:?}…", names.iter().take(3).collect::<Vec<_>>()));
        scale(g, cv);
    }

    /// 100,000 swatches: built, scrolled end to end; cells are reused.
    fn scale(g: Gallery, cv: Retained<NSCollectionView>) {
        let (cells0, heap0) = (collection_cells_made(), heap_in_use_mb());
        let t = Instant::now();
        batch(|| g.mode.set(1));
        then(move || scale_measure(g, cv, cells0, heap0, t));
    }

    fn scale_measure(g: Gallery, cv: Retained<NSCollectionView>, cells0: u64, heap0: f64, t: Instant) {
        let _ = t;
        // A full reload of the 100,000 items, laid out and drawn.
        let t = Instant::now();
        cv.reloadData();
        cv.layoutSubtreeIfNeeded();
        cv.displayIfNeeded();
        let load_ms = t.elapsed().as_secs_f64() * 1e3;
        let n = cv.numberOfItemsInSection(0);
        let t = Instant::now();
        let mut worst = 0.0f64;
        for step in (0..n).step_by(((n / 100) as usize).max(1)) {
            let s = Instant::now();
            let path = NSSet::from_retained_slice(&[NSIndexPath::indexPathForItem_inSection(step, 0)]);
            cv.scrollToItemsAtIndexPaths_scrollPosition(&path, NSCollectionViewScrollPosition::Top);
            cv.displayIfNeeded();
            worst = worst.max(s.elapsed().as_secs_f64() * 1e3);
        }
        let scroll_ms = t.elapsed().as_secs_f64() * 1e3;
        let (cells, heap) = (collection_cells_made() - cells0, heap_in_use_mb() - heap0);
        let ci = std::env::var_os("CI").is_some();
        check(
            "scale",
            n == 100_000 && cells < 400 && heap < 60.0 && (ci || (load_ms < 500.0 && worst < 33.0)),
            format!("100,000 swatches: reload, layout and draw {load_ms:.1} ms; 100 jumps end to end in {scroll_ms:.0} ms (worst {worst:.1} ms); {cells} cells made (reused); heap +{heap:.1} MB{}", if ci { " (timing informational on CI)" } else { "" }),
        );
        double_click(g, cv);
    }

    /// A double-click on a cell (sent to its view) activates that item.
    fn double_click(g: Gallery, cv: Retained<NSCollectionView>) {
        cv.scrollToItemsAtIndexPaths_scrollPosition(&NSSet::from_retained_slice(&[NSIndexPath::indexPathForItem_inSection(0, 0)]), NSCollectionViewScrollPosition::Top);
        cv.displayIfNeeded();
        let item = cv.itemAtIndex(5).unwrap();
        let view = item.view();
        let r = view.convertRect_toView(view.bounds(), None);
        let at = NSPoint::new(r.origin.x + r.size.width / 2.0, r.origin.y + r.size.height / 2.0);
        let w = cv.window().unwrap();
        let before = g.opened.get_untracked().len();
        for clicks in [1, 2] {
            let down = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                NSEventType::LeftMouseDown, at, NSEventModifierFlags::empty(), 0.0, w.windowNumber(), None, 0, clicks, 1.0,
            )
            .unwrap();
            view.mouseDown(&down);
        }
        let opened = g.opened.get_untracked();
        check("activate-double-click", opened.len() == before + 1 && opened.last().map(String::as_str) == Some("swatch 5"), format!("double-click on item 5 → opened {:?}", opened.last()));
        let failed = FAILED.with(Cell::get);
        eprintln!("[check] done, {failed} failed");
        std::process::exit(if failed == 0 { 0 } else { 1 });
    }

    impl Entry {
        fn id_string(&self) -> String {
            Identifiable::id(self)
        }
    }

    impl Gallery {
        fn items_at(&self, i: usize) -> Option<String> {
            untrack_items(self).get(i).map(Identifiable::id)
        }
    }

    fn untrack_items(g: &Gallery) -> Vec<Entry> {
        appkit_ui::untrack(|| g.items())
    }
}
