//! Q3: the API review's changes, checked. One name per modifier, taking a
//! value or a closure (`text`, `.color`, `.background`, `.opacity`,
//! `.disabled`, `button`, `menu_item`); `font` and `color` after any
//! modifier; truncating text; minimum sizes; button symbols; accessibility
//! values; a lone `for_each` in a stack.
//! `cargo run -p ferrokit --example values`; `CHECK=all` runs the checks.

use ferrokit::*;

#[derive(Clone, PartialEq)]
struct Row {
    id: u32,
    name: String,
}

impl Identifiable for Row {
    type Id = u32;
    fn id(&self) -> u32 {
        self.id
    }
}

#[derive(Clone, Copy)]
struct State {
    count: Signal<i32>,
    playing: Signal<bool>,
    rows: Signal<Vec<Row>>,
}

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    let s = State {
        count: Signal::new(0),
        playing: Signal::new(false),
        rows: Signal::new((1..=3).map(|id| Row { id, name: format!("Row {id}") }).collect()),
    };
    if checks {
        self_checks::STATE.with(|c| c.set(Some(s)));
    }
    let count = s.count;
    let view = vstack((
        // Text: fixed, and computed with a computed colour.
        text("Values").font(Font::title2().weight(Weight::Bold)),
        text(move || format!("Count {}", count.get())).color(move || if count.get() < 0 { Color::Red } else { Color::Primary }).accessibility_label("Count"),
        // A background and an opacity from signals.
        text("Card").padding(10.0).background(move || if count.get() > 0 { Color::Green } else { Color::QuaternaryFill }).opacity(move || if count.get() > 1 { 0.5 } else { 1.0 }).accessibility_label("Card"),
        // Font and colour after another modifier.
        text("Styled after padding").padding(4.0).font(Font::headline()).color(Color::Orange),
        // Truncating text in a narrow row.
        hstack((text("A very long name that cannot fit in this narrow row at all").truncate(), text("end"))).width(160.0).accessibility_label("Narrow row"),
        // Two truncating texts sharing a narrow row; the first keeps its minimum.
        hstack((text("Status: 12,909 rows in orders, sorted by total").truncate().min_width(110.0), text("Another long neighbour that wants the room too").truncate())).width(220.0),
        // A row inside a row: the inner one's fixed label keeps its width,
        // and a padded box keeps its text's minimum.
        vstack((
            hstack((hstack((text("Fixed label").width(90.0), text("Inner text that is far too long for here").truncate())), text("Outer text that is far too long as well").truncate())).width(220.0),
            hstack((text("Boxed: a long text in a padded box").truncate().min_width(100.0).background(Color::QuaternaryFill).padding(4.0), text("Beside the box, also much too long").truncate())).width(200.0),
        )),
        // A minimum width.
        text("Min").min_width(120.0).background(Color::QuaternaryFill),
        // A button whose title and symbol follow a signal, and one disabled.
        hstack((
            button(move || if s.playing.get() { "Pause".to_string() } else { "Play".to_string() }, move || s.playing.set(!s.playing.get()))
                .symbol(move || if s.playing.get() { "pause.fill".to_string() } else { "play.fill".to_string() }),
            button("Never", || {}).disabled(true),
            text("Tap me").padding(6.0).background(Color::QuaternaryFill).on_click(move || count.update(|c| *c += 1)).accessibility_value(move || format!("{} taps", count.get())),
        ))
        .spacing(8.0),
        // A lone for_each.
        vstack(for_each(move || s.rows.get(), |r: Row| text(r.name))),
    ))
    .spacing(10.0)
    .alignment(Align::Start)
    .padding(20.0);
    App::new("Values")
        .window(Window::new("Values").size(420.0, 620.0).view(view))
        .menu("Count", vec![menu_item(move || format!("Reset {}", count.get()), move || count.set(0)).disabled(move || count.get() == 0).checked(move || count.get() > 0)])
        .on_launch(move || {
            if checks {
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(400)), |()| self_checks::run());
            }
        })
        .run();
}

mod self_checks {
    use std::cell::Cell;

    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindow};

    use super::State;

    thread_local! {
        pub static STATE: Cell<Option<State>> = const { Cell::new(None) };
    }

    fn views(w: &NSWindow) -> Vec<Retained<NSView>> {
        fn all(v: &NSView, out: &mut Vec<Retained<NSView>>) {
            for s in v.subviews().iter() {
                out.push(s.clone());
                all(&s, out);
            }
        }
        let mut out = Vec::new();
        all(&w.contentView().unwrap(), &mut out);
        out
    }

    fn field(w: &NSWindow, text: &str) -> Option<Retained<objc2_app_kit::NSTextField>> {
        views(w).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSTextField>().ok()).find(|f| f.stringValue().to_string().starts_with(text))
    }

    fn surface_behind(w: &NSWindow, v: &NSView) -> Option<Retained<NSView>> {
        let f = v.frame();
        views(w).into_iter().find(|s| s.class().name().to_string_lossy() == "FerrokitSurface" && s.frame().origin.x <= f.origin.x && s.frame().origin.y <= f.origin.y && s.frame().size.width >= f.size.width)
    }

    fn later(ms: u64, f: impl FnOnce() + 'static) {
        spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
    }

    fn check(failed: &Cell<u32>, name: &str, pass: bool, note: String) {
        eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
        failed.set(failed.get() + u32::from(!pass));
    }

    pub fn run() {
        let w = windows().into_iter().find(|w| w.title().to_string() == "Values").unwrap();
        let s = STATE.with(Cell::get).unwrap();
        let failed = std::rc::Rc::new(Cell::new(0));
        let f = failed.clone();
        let c = move |n: &str, p: bool, note: String| check(&f, n, p, note);
        let c = std::rc::Rc::new(c);
        // Before.
        let count = field(&w, "Count").unwrap();
        let card = field(&w, "Card").unwrap();
        let card_bg = surface_behind(&w, &card).and_then(|v| v.layer()).and_then(|l| l.backgroundColor()).map(|c| objc2_core_graphics::CGColor::alpha(Some(&c)));
        c("text-fixed-and-computed", count.stringValue().to_string() == "Count 0" && field(&w, "Values").is_some(), format!("{:?}", count.stringValue()));
        let styled = field(&w, "Styled after padding").unwrap();
        let font = styled.font().map(|f| (f.pointSize(), f.fontDescriptor().symbolicTraits().contains(objc2_app_kit::NSFontDescriptorSymbolicTraits::TraitBold)));
        let color = styled.textColor().and_then(|c| Color::from_ns(&c));
        c("font-and-color-after-modifier", font.is_some_and(|(_, bold)| bold) && color == Some(Color::Orange), format!("font {font:?}, colour {color:?}"));
        let long = field(&w, "A very long name").unwrap();
        let end = field(&w, "end").unwrap();
        let fits = long.frame().size.width + end.frame().size.width <= 161.0 && long.lineBreakMode() == objc2_app_kit::NSLineBreakMode::ByTruncatingTail;
        c("truncate", fits, format!("long {:.0} + end {:.0} in 160", long.frame().size.width, end.frame().size.width));
        let status = field(&w, "Status: 12,909").unwrap().frame().size.width;
        let other = field(&w, "Another long neighbour").unwrap().frame().size.width;
        c("truncate-min-width", (110.0..=111.0).contains(&status) && other > 50.0 && status + other <= 221.0, format!("status {status:.0} (minimum 110), neighbour {other:.0}, in 220"));
        let (label, inner, outer) = (field(&w, "Fixed label").unwrap().frame(), field(&w, "Inner text").unwrap().frame(), field(&w, "Outer text").unwrap().frame());
        let boxed = field(&w, "Boxed:").unwrap().frame().size.width;
        c(
            "truncate-nested-floors",
            (label.size.width - 90.0).abs() < 1.0 && inner.origin.x >= label.origin.x + 89.0 && outer.origin.x >= inner.origin.x + inner.size.width - 0.5 && outer.size.width > 20.0 && (100.0..=101.0).contains(&boxed),
            format!("label {:.0} wide; inner at {:.0}, {:.0} wide; outer at {:.0}, {:.0} wide; the boxed text {boxed:.0} (minimum 100)", label.size.width, inner.origin.x, inner.size.width, outer.origin.x, outer.size.width),
        );
        let min = field(&w, "Min").unwrap();
        let min_bg = surface_behind(&w, &min).map(|v| v.frame().size.width);
        c("min-width", min_bg.is_some_and(|wd| wd >= 119.5), format!("{min_bg:?}"));
        let rows = (1..=3).all(|i| field(&w, &format!("Row {i}")).is_some());
        c("lone-for-each", rows, "three rows from vstack(for_each(..))".into());
        let buttons: Vec<Retained<objc2_app_kit::NSButton>> = views(&w).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSButton>().ok()).collect();
        let play = buttons.iter().find(|b| b.title().to_string() == "Play").cloned();
        let never = buttons.iter().find(|b| b.title().to_string() == "Never").cloned();
        c("button-disabled-fixed", never.as_ref().is_some_and(|b| !b.isEnabled()), String::new());
        let had_image = play.as_ref().is_some_and(|b| b.image().is_some());
        // Change the signals: everything follows.
        s.count.set(2);
        s.playing.set(true);
        let c2 = c.clone();
        let failed2 = failed.clone();
        later(300, move || {
            let red = count.textColor().and_then(|c| Color::from_ns(&c));
            c2("text-follows", count.stringValue().to_string() == "Count 2" && red == Some(Color::Primary), format!("{:?}, {red:?}", count.stringValue()));
            let bg = surface_behind(&w, &card).and_then(|v| v.layer()).and_then(|l| l.backgroundColor()).map(|c| objc2_core_graphics::CGColor::alpha(Some(&c)));
            let faded = card.alphaValue();
            c2("background-and-opacity-follow", bg.is_some() && bg != card_bg && (faded - 0.5).abs() < 0.01, format!("background alpha {card_bg:?} → {bg:?}; opacity {faded}"));
            let titled = play.as_ref().map(|b| b.title().to_string());
            c2("button-title-and-symbol", had_image && titled.as_deref() == Some("Pause") && play.as_ref().is_some_and(|b| b.image().is_some()), format!("{titled:?}"));
            // The menu item's title, check and enabled state, as the menu opens.
            let app = objc2_app_kit::NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
            let menu = app.mainMenu().unwrap().itemWithTitle(&objc2_foundation::NSString::from_str("Count")).unwrap().submenu().unwrap();
            menu.update();
            let item = menu.itemAtIndex(0).unwrap();
            c2(
                "menu-item-values",
                item.title().to_string() == "Reset 2" && item.isEnabled() && item.state() == objc2_app_kit::NSControlStateValueOn,
                format!("{:?}, enabled {}, state {}", item.title(), item.isEnabled(), item.state()),
            );
            // The clickable view's accessibility value.
            let tap = views(&w).into_iter().find(|v| v.class().name().to_string_lossy() == "FerrokitInteraction");
            let value = tap.as_ref().and_then(|v| objc2_app_kit::NSAccessibility::accessibilityValue(&**v)).map(|v| unsafe { objc2::msg_send![&*v, description] }).map(|d: Retained<objc2_foundation::NSString>| d.to_string());
            c2("accessibility-value", value.as_deref() == Some("2 taps"), format!("{value:?}"));
            eprintln!("[check] done, {} failed", failed2.get());
            std::process::exit(if failed2.get() == 0 { 0 } else { 1 });
        });
    }
}
