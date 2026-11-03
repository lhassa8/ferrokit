//! V5: the advanced components, in a trip briefing: a guide in a web view,
//! a report printed to PDF and shown in a PDF view, a clip in a video
//! player, the Share menu, printing, a reminder notification, a countdown
//! in the menu bar, and the report's Quick Look thumbnail and preview (C8).
//! `cargo run -p ferrokit --example media`; `CHECK=all` runs its
//! self-checks. Notifications need an app bundle
//! (`scripts/bundle-example.sh media`).

use std::path::PathBuf;

use ferrokit::*;

const GUIDE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Lofoten guide</title>
<meta name="color-scheme" content="light dark">
<style>
  body { font: -apple-system-body; margin: 24px 28px; line-height: 1.45; }
  h1 { font: -apple-system-title1; margin: 0 0 6px; }
  p.lede { color: #6e6e73; margin-top: 0; }
  li { margin: 6px 0; }
  a { color: -apple-system-control-accent; }
</style></head>
<body>
  <h1>Lofoten</h1>
  <p class="lede">Islands of fishing villages, beaches and steep peaks above the Arctic Circle.</p>
  <h3>Five days</h3>
  <ol>
    <li><b>Svolvær:</b> ferry from Bodø, the harbour at dusk.</li>
    <li><b>Henningsvær:</b> the football pitch on the rocks; galleries.</li>
    <li><b>Haukland beach:</b> an easy walk round Mannen.</li>
    <li><b>Reine:</b> climb Reinebringen early, before the crowds.</li>
    <li><b>Å:</b> the stockfish museum, then the ferry home.</li>
  </ol>
  <p>Ferries and buses: <a href="https://www.visitnorway.com">Visit Norway</a>.</p>
</body></html>"#;

const DAYS: [(&str, &str, &str); 10] = [
    ("Mon 6 July", "Bodø → Moskenes", "Ferry, 3 h 15 min. Book the car deck ahead."),
    ("Tue 7 July", "Reine", "Reinebringen at 6 am; kayaks in the afternoon."),
    ("Wed 8 July", "Å and Sørvågen", "Stockfish museum; dinner at the bakery."),
    ("Thu 9 July", "Flakstad", "Ramberg beach and the glassworks at Vikten."),
    ("Fri 10 July", "Haukland", "The walk round Mannen; swim if you dare."),
    ("Sat 11 July", "Eggum", "Midnight sun from the old radar station."),
    ("Sun 12 July", "Henningsvær", "Climbing school, galleries, fish soup."),
    ("Mon 13 July", "Svolvær", "The Svolvær Goat, from below."),
    ("Tue 14 July", "Trollfjord", "Boat trip with sea eagles."),
    ("Wed 15 July", "Svolvær → Bodø", "Morning ferry; train south."),
];

/// The report: what's printed, saved as PDF and shown in the PDF view.
fn report() -> impl View {
    let day = |(date, place, note): (&'static str, &'static str, &'static str)| {
        hstack((
            text(date).font(Font::headline()).width(110.0),
            vstack((text(place).font(Font::body().weight(Weight::Semibold)), text(note).color(Color::Secondary))).spacing(2.0).alignment(Align::Start),
            spacer(),
        ))
        .padding(12.0)
        .background(Color::Rgb(0.95, 0.96, 0.98))
        .corner_radius(8.0)
    };
    let days: Vec<AnyView> = DAYS.iter().map(|d| AnyView::new(day(*d))).collect();
    let budget = || vec![("Lodging".to_string(), 1240.0), ("Food".to_string(), 520.0), ("Ferries".to_string(), 310.0), ("Car".to_string(), 450.0)];
    vstack((
        text("Lofoten, July 2026").font(Font::large_title().weight(Weight::Bold)),
        text("Ten days from Bodø: the itinerary and the budget.").color(Color::Secondary),
        bar_chart(budget).value_format(Format::Currency("EUR".into())).min_size(480.0, 200.0),
        vstack(days).spacing(8.0),
        text("Printed with Ferrokit.").font(Font::caption()).color(Color::Tertiary),
    ))
    .spacing(14.0)
    .alignment(Align::Start)
    .padding(8.0)
}

fn report_path() -> PathBuf {
    std::env::temp_dir().join("Lofoten report.pdf")
}

fn reminder(tab: Signal<i32>) -> Notification {
    Notification::new("Ferry to Moskenes")
        .subtitle("Monday 6 July, 15:30")
        .body("Be at the Bodø ferry terminal by 14:45 with the car.")
        .sound()
        .id("ferry")
        .on_click(move || tab.set(1))
}

struct State {
    tab: Signal<i32>,
    page: WebPage,
    pdf_page: Signal<usize>,
    pdf_pages: Signal<usize>,
    find: Signal<String>,
    clip: Signal<Option<PathBuf>>,
    playing: Signal<bool>,
    time: Signal<f64>,
    length: Signal<f64>,
    muted: Signal<bool>,
}

fn guide(s: &State) -> impl View {
    let (p1, p2, p3) = (s.page.clone(), s.page.clone(), s.page.clone());
    vstack((
        hstack((
            button("Back", move || p1.back()).disabled(move || !p2.can_go_back()),
            text(move || p3.title()).font(Font::headline()),
            spacer(),
        )),
        web_view(&s.page).links_open_in_browser().corner_radius(8.0).border(Color::Separator, 1.0),
    ))
    .spacing(8.0)
}

fn report_tab(s: &State) -> impl View {
    let (page, pages, find) = (s.pdf_page, s.pdf_pages, s.find);
    vstack((
        hstack((
            thumbnail(|| report_path().exists().then(report_path)).size(28.0, 36.0),
            button("Quick Look", || quick_look(vec![report_path()], 0)).symbol("eye").tooltip("Preview the report as Finder does"),
            search_field(&find).placeholder("Find in report"),
            spacer(),
            button("Previous", move || page.set(page.get().saturating_sub(1))).disabled(move || page.get() == 0),
            text(move || format!("Page {} of {}", page.get() + 1, pages.get().max(1))).font(Font::body().monospaced_digits()),
            button("Next", move || page.set(page.get() + 1)).disabled(move || page.get() + 1 >= pages.get()),
        )),
        pdf_view(|| report_path().exists().then(report_path)).page(&page).page_count(&pages).highlight(move || find.get()),
    ))
    .spacing(8.0)
}

fn clip_tab(s: &State) -> impl View {
    let (clip, playing, time, length, muted) = (s.clip, s.playing, s.time, s.length, s.muted);
    vstack((
        hstack((
            button("Choose Clip…", move || {
                let Some(w) = focused_window() else { return };
                let options = OpenOptions { extensions: ["mov", "mp4", "m4v", "m4a", "aiff"].map(String::from).to_vec(), ..Default::default() };
                open_panel(&w, options, move |paths| {
                    if let Some(p) = paths.and_then(|p| p.into_iter().next()) {
                        clip.set(Some(p));
                    }
                });
            }),
            button(move || if playing.get() { "Pause".to_string() } else { "Play".to_string() }, move || playing.set(!playing.get()))
                .symbol(move || if playing.get() { "pause.fill".to_string() } else { "play.fill".to_string() }),
            checkbox("Mute", &muted),
            spacer(),
            text(move || format!("{:.0}:{:02.0} / {:.0}:{:02.0}", (time.get() / 60.0).floor(), time.get() % 60.0, (length.get() / 60.0).floor(), length.get() % 60.0))
                .font(Font::body().monospaced_digits())
                .color(Color::Secondary),
        )),
        video_player(move || clip.get()).playing(&playing).time(&time).duration(&length).muted(move || muted.get()),
    ))
    .spacing(8.0)
}

#[path = "shared/tour.rs"]
mod tour;

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    // `DEMO=1`: a fast tour, for recording (`DEMO_CLIP=<movie>` to play one).
    let demo = std::env::var("DEMO").is_ok();
    let s = State {
        tab: Signal::new(0),
        page: WebPage::new(),
        pdf_page: Signal::new(0),
        pdf_pages: Signal::new(0),
        find: Signal::new(String::new()),
        clip: Signal::new(None),
        playing: Signal::new(false),
        time: Signal::new(0.0),
        length: Signal::new(0.0),
        muted: Signal::new(false),
    };
    s.page.load_html(GUIDE, None);
    if let Err(e) = save_pdf(report(), report_path()) {
        eprintln!("couldn't save the report: {e}");
    }
    let tab = s.tab;
    let days_left = Signal::new(3);
    let header = hstack((
        text("Lofoten briefing").font(Font::title2().weight(Weight::Bold)),
        spacer(),
        share_button(|| vec![ShareItem::File(report_path()), ShareItem::Text("Our Lofoten trip, 6–15 July".into())]).tooltip("Share the report"),
        button("Print…", || {
            if let Some(w) = focused_window() {
                print(&w, "Lofoten report", report());
            }
        }),
        button("Remind Me", move || notify(reminder(tab))),
    ))
    .spacing(8.0);
    let mut app = App::new("Briefing")
        .window(Window::new("Briefing").size(760.0, 600.0).view(
            vstack((
                header,
                tabs(&s.tab, vec![("Guide", 0, AnyView::new(guide(&s))), ("Report", 1, AnyView::new(report_tab(&s))), ("Clip", 2, AnyView::new(clip_tab(&s)))]).style(TabStyle::Segmented),
            ))
            .spacing(12.0)
            .padding(20.0),
        ))
        .menu_bar_extra(MenuBarExtra::new("ferry").title(move || format!("{} days", days_left.get())).accessibility_label("Days to the ferry").menu(vec![
            menu_item("Show Report", move || tab.set(1)),
            menu_item("Remind Me", move || notify(reminder(tab))),
            menu_separator(),
            menu_item("Quit Briefing", quit),
        ]));
    let tour = demo.then_some(demo::Tour { tab: s.tab, pdf_page: s.pdf_page, find: s.find, clip: s.clip, playing: s.playing, muted: s.muted });
    if checks {
        ferrokit::testing::hold_notifications();
        // A panel extra too, for its check.
        app = app.menu_bar_extra(MenuBarExtra::new("sun.max").accessibility_label("Weather").panel(vstack((text("Reine"), text("14°, clear").font(Font::title3()))).padding(14.0)));
        self_checks::STATE.with(|c| *c.borrow_mut() = Some((s, days_left)));
    }
    app.on_launch(move || {
        if checks {
            spawn(|| std::thread::sleep(std::time::Duration::from_millis(600)), |()| self_checks::run());
        } else if let Some(t) = tour {
            spawn(|| std::thread::sleep(std::time::Duration::from_millis(500)), move |()| demo::run(t));
        }
    })
    .run();
}

mod self_checks {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use ferrokit::testing::{ax_audit, click_notification, hold_notifications, menu_bar_extras, sent_notifications, shared_items};
    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSButton, NSView, NSWindow};
    use objc2_foundation::NSString;

    use super::State;

    thread_local! {
        pub static STATE: RefCell<Option<(State, Signal<i32>)>> = const { RefCell::new(None) };
    }

    type Check = Rc<dyn Fn(&str, bool, String)>;

    fn later_ms(ms: u64, f: impl FnOnce() + 'static) {
        spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
    }

    /// Calls `f` every 100 ms until it returns true or `ms` pass, then `then`.
    fn wait_for(ms: u64, f: impl Fn() -> bool + 'static, then: impl FnOnce(bool) + 'static) {
        if f() || ms == 0 {
            let ok = f();
            then(ok);
            return;
        }
        later_ms(100, move || wait_for(ms.saturating_sub(100), f, then));
    }

    pub fn run() {
        let w = windows().into_iter().find(|w| w.title().to_string() == "Briefing").unwrap();
        let (s, days_left) = STATE.with(|c| c.borrow_mut().take()).unwrap();
        let failed = Rc::new(Cell::new(0));
        let check: Check = {
            let failed = failed.clone();
            Rc::new(move |name: &str, pass: bool, note: String| {
                eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
                failed.set(failed.get() + u32::from(!pass));
            })
        };
        let problems = ax_audit(&w);
        check("accessibility", problems.is_empty(), format!("{problems:?}"));
        hold_notifications();
        snap(&w, "guide");

        // Printing to PDF: pages, text, and backgrounds drawn for print.
        let data = pdf_data(super::report());
        if let Ok(dir) = std::env::var("SNAP") {
            let _ = std::fs::write(format!("{dir}/media-printed.pdf"), &data);
        }
        let doc = pdf_document(&data);
        let (pages, body) = doc.as_ref().map(|d| (unsafe { d.pageCount() }, unsafe { d.string() }.map(|s| s.to_string()).unwrap_or_default())).unwrap_or_default();
                let has = ["Lofoten, July 2026", "Reinebringen", "Trollfjord", "Lodging"].iter().all(|t| body.contains(t));
        // Each label's text once (AppKit's own labels print theirs twice).
        let once = body.matches("Reinebringen").count();
        check("print-pdf", data.starts_with(b"%PDF") && pages >= 2 && has && once == 1, format!("{} bytes, {pages} pages (2 or more); text {has}; written {once} time(s)", data.len()));
        let card = pdf_data(text("Card").padding(30.0).background(Color::Rgb(0.9, 0.1, 0.1)).corner_radius(10.0));
        let red = pdf_pixel(&card, 0.09, 0.07);
        // Half-inch margins: the card starts 36 points in; its corner is cut away.
        let corner = pdf_pixel(&card, 36.8 / 612.0, 36.8 / 792.0);
        let is_red = |p: Option<(f64, f64, f64)>| p.is_some_and(|(r, g, b)| r > 0.8 && g < 0.3 && b < 0.3);
        check("print-backgrounds", is_red(red) && !is_red(corner), format!("card fill {red:?}; rounded corner {corner:?}"));

        // The web view: the page's title in the label, script results.
        let page = s.page.clone();
        let c = check.clone();
        wait_for(4000, { let p = page.clone(); move || p.title() == "Lofoten guide" && !p.is_loading() }, move |ok| {
            let labels = text_of(&w);
            c("web-title", ok && labels.iter().any(|l| l == "Lofoten guide"), format!("title {:?}, loading {}; labels show it {}", page.title(), page.is_loading(), labels.iter().any(|l| l == "Lofoten guide")));
            let c2 = c.clone();
            page.evaluate("document.querySelectorAll('li').length + ' ' + document.body.innerText.includes('Reine')", move |r| {
                c2("web-script", r.as_deref() == Ok("5 true"), format!("{r:?} (\"5 true\")"));
            });
            let back = buttons(&w).into_iter().find(|b| b.title().to_string() == "Back");
            c("web-back", back.is_some_and(|b| !b.isEnabled()) && !page.can_go_back(), "Back disabled with no history".into());
            let w2 = w.clone();
            later_ms(300, move || pdf_phase(w2, s, days_left, c, failed));
        });
    }

    fn pdf_phase(w: Retained<NSWindow>, s: State, days_left: Signal<i32>, check: Check, failed: Rc<Cell<u32>>) {
        s.tab.set(1);
        let (page, pages, find) = (s.pdf_page, s.pdf_pages, s.find);
        later_ms(300, move || {
            snap(&w, "report");
            let view = pdf_views(&w).into_iter().next();
            check("pdf-pages", pages.get() >= 2, format!("{} pages", pages.get()));
            page.set(1);
            let w2 = w.clone();
            later_ms(300, move || {
                let shown = view.as_ref().and_then(|v| unsafe { Some(v.document()?.indexForPage(&*v.currentPage()?)) });
                let label = text_of(&w2).into_iter().find(|t| t.starts_with("Page "));
                check("pdf-go-to-page", shown == Some(1) && label.as_deref() == Some(&format!("Page 2 of {}", pages.get())), format!("shown {shown:?}; label {label:?}"));
                find.set("ferry".into());
                later_ms(300, move || {
                    let found = view.as_ref().and_then(|v| unsafe { v.highlightedSelections() }).map_or(0, |a| a.count());
                    // "Ferry, 3 h 15 min" and "Morning ferry".
                    check("pdf-find", found == 2, format!("{found} matches highlighted (2)"));
                    snap(&w2, "report-find");
                    // A match far down page 1, from page 2: it's scrolled into view.
                    find.set(String::new());
                    page.set(1);
                    later_ms(300, move || find.set("Eggum".into()));
                    // Where the first match is, in the view: by its bounds, or
                    // by its first character when PDFKit gives it none.
                    let v2 = view.clone();
                    let place = move || {
                        v2.as_ref().and_then(|v| unsafe {
                            let first = v.highlightedSelections()?.firstObject()?;
                            let page = first.pages().firstObject()?;
                            let mut r = first.boundsForPage(&page);
                            let bounded = r.origin.y.is_finite();
                            if !bounded && first.numberOfTextRangesOnPage(&page) > 0 {
                                r = page.characterBoundsAtIndex(first.rangeAtIndex_onPage(0, &page).location as _);
                            }
                            let r = v.convertRect_fromPage(r, &page);
                            let h = v.bounds().size.height;
                            let shown = v.currentPage().and_then(|c| Some(v.document()?.indexForPage(&c)));
                            // Its middle: goToSelection can leave a point of it past the top.
                            let mid = r.origin.y + r.size.height / 2.0;
                            // Sometimes PDFKit has no position for any of the match's text
                            // (macOS 26 and 27): then all find can do is show its page.
                            let unplaced = !bounded && (r.size.height <= 0.0 || !r.origin.y.is_finite());
                            let seen = if unplaced { shown == Some(0) } else { (0.0..=h).contains(&mid) };
                            let shown = if unplaced { format!("{shown:?} (PDFKit gave the match no position, so its page is what's checked)") } else { format!("{shown:?}") };
                            Some((seen, format!("match at y {:.0}, height {:.0}, in a view {h:.0} high; bounds from PDFKit {bounded}; page shown {shown}", r.origin.y, r.size.height)))
                        })
                    };
                    let place2 = place.clone();
                    later_ms(300, move || {
                        wait_for(3000, move || place().is_some_and(|(seen, _)| seen), move |seen| {
                            let detail = place2().map_or("no match highlighted".into(), |(_, d)| d);
                            check("pdf-find-shows-match", seen, format!("first match in view: {seen}; {detail}"));
                            find.set(String::new());
                            quick_look_phase(w2, s, days_left, check, failed);
                        });
                    });
                });
            });
        });
    }

    /// C8: the report's thumbnail, then the Quick Look panel on it.
    fn quick_look_phase(w: Retained<NSWindow>, s: State, days_left: Signal<i32>, check: Check, failed: Rc<Cell<u32>>) {
        use ferrokit::testing::{quick_look_state, thumbnails_made};
        let c = check.clone();
        wait_for(5000, || thumbnails_made() >= 1, move |made| {
            // The PDF's first page, white and taller than wide, not its icon.
            let view = all_in(&w.contentView().unwrap()).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSImageView>().ok()).find(|v| objc2_app_kit::NSAccessibility::accessibilityLabel(&**v).is_some_and(|l| l.to_string() == "Lofoten report.pdf"));
            let size = view.as_ref().and_then(|v| v.image()).map(|i| i.size());
            let page_shaped = size.is_some_and(|s| s.height > s.width * 1.2);
            c("thumbnail", made && page_shaped, format!("made {made}; image {size:?} (a page, taller than wide); VoiceOver reads the file's name {}", view.is_some()));
            let button = buttons(&w).into_iter().find(|b| b.title().to_string() == "Quick Look").unwrap();
            // SAFETY: a click on a button of this window.
            unsafe { button.performClick(None) };
            let c2 = c.clone();
            wait_for(3000, || quick_look_state().1.is_some(), move |open| {
                let (files, shown) = quick_look_state();
                if let Ok(dir) = std::env::var("SNAP") {
                    ferrokit::testing::snapshot_windows(&dir);
                }
                c2("quick-look", open && files == [super::report_path()] && shown == Some(0) && is_quick_look_open(), format!("open {open}; files {files:?}; showing {shown:?}"));
                close_quick_look();
                let c3 = c2.clone();
                wait_for(2000, || !is_quick_look_open(), move |closed| {
                    c3("quick-look-close", closed, format!("closed {closed}"));
                    clip_phase(w, s, days_left, c3, failed);
                });
            });
        });
    }

    fn clip_phase(w: Retained<NSWindow>, s: State, days_left: Signal<i32>, check: Check, failed: Rc<Cell<u32>>) {
        // A sound file made with `say`: AVKit plays sound as it plays video.
        let path = std::env::temp_dir().join(format!("ferrokit-media-{}.aiff", std::process::id()));
        let made = std::process::Command::new("say").arg("-o").arg(&path).arg("Welcome to Lofoten. The ferry leaves at half past three.").status().is_ok_and(|s| s.success());
        s.tab.set(2);
        s.muted.set(true);
        s.clip.set(made.then(|| path.clone()));
        let (playing, time, length) = (s.playing, s.time, s.length);
        later_ms(300, move || {
            check("video-duration", made && length.get() > 1.0, format!("made {made}; {:.2} s", length.get()));
            playing.set(true);
            later_ms(900, move || {
                snap(&w, "clip");
                let player = player_views(&w).into_iter().next();
                let rate = player.as_ref().and_then(|p| unsafe { p.player() }).map(|p| unsafe { p.rate() });
                check("video-plays", playing.get() && time.get() > 0.2 && rate.is_some_and(|r| r > 0.0), format!("playing {}, {:.2} s, rate {rate:?}", playing.get(), time.get()));
                playing.set(false);
                later_ms(300, move || {
                    let rate = player.as_ref().and_then(|p| unsafe { p.player() }).map(|p| unsafe { p.rate() });
                    check("video-pause", rate == Some(0.0), format!("rate {rate:?}"));
                    time.set(0.0);
                    later_ms(200, move || {
                        check("video-seek", time.get() < 0.3, format!("{:.2} s", time.get()));
                        let _ = std::fs::remove_file(&path);
                        services_phase(w, s, days_left, check, failed);
                    });
                });
            });
        });
    }

    fn services_phase(w: Retained<NSWindow>, s: State, days_left: Signal<i32>, check: Check, failed: Rc<Cell<u32>>) {
        s.tab.set(0);
        // Share.
        let share = buttons(&w).into_iter().find(|b| shared_items(b).is_some());
        let items = share.as_ref().and_then(|b| shared_items(b));
        let want = vec![ShareItem::File(super::report_path()), ShareItem::Text("Our Lofoten trip, 6–15 July".into())];
        let label = share.as_ref().and_then(|b| objc2_app_kit::NSAccessibility::accessibilityLabel(&**b)).map(|l| l.to_string());
        check("share", items.as_ref() == Some(&want) && label.as_deref() == Some("Share"), format!("{items:?}; label {label:?}"));

        // A notification, and its click.
        let remind = buttons(&w).into_iter().find(|b| b.title().to_string() == "Remind Me").unwrap();
        // SAFETY: clicking a button on the main thread.
        unsafe { remind.performClick(None) };
        let sent = sent_notifications();
        let n = sent.first();
        check(
            "notification",
            n.is_some_and(|n| n.title == "Ferry to Moskenes" && n.subtitle == "Monday 6 July, 15:30" && n.body.contains("14:45") && n.sound && n.id == "ferry"),
            format!("{sent:?}"),
        );
        let clicked = click_notification("ferry");
        later_ms(150, move || {
            check("notification-click", clicked && s.tab.get() == 1, format!("ran {clicked}; tab {}", s.tab.get()));

            // The menu-bar extras.
            let extras = menu_bar_extras();
            let mtm = objc2::MainThreadMarker::new().unwrap();
            let (item, _) = &extras[0];
            let title = item.button(mtm).map(|b| b.title().to_string());
            let menu = item.menu(mtm);
            let titles: Vec<String> = menu.as_ref().map(|m| m.itemArray().iter().map(|i| i.title().to_string()).collect()).unwrap_or_default();
            days_left.set(2);
            let item2 = item.clone();
            later_ms(100, move || {
                let after = item2.button(mtm).map(|b| b.title().to_string());
                check(
                    "menu-bar-extra",
                    title.as_deref() == Some(" 3 days") && after.as_deref() == Some(" 2 days") && titles == ["Show Report", "Remind Me", "", "Quit Briefing"],
                    format!("title {title:?} then {after:?}; menu {titles:?}"),
                );
                s.tab.set(0);
                if let Some(m) = &menu {
                    m.performActionForItemAtIndex(0);
                }
                let (panel_item, popover) = extras[1].clone();
                if let Some(b) = panel_item.button(mtm) {
                    // SAFETY: as above.
                    unsafe { b.performClick(None) };
                }
                later_ms(400, move || {
                    check("menu-bar-menu", s.tab.get() == 1, format!("Show Report chose tab {}", s.tab.get()));
                    let shown = popover.as_ref().is_some_and(|p| p.isShown());
                    let inside = popover.as_ref().and_then(|p| p.contentViewController()).map(|c| texts_in(&c.view())).unwrap_or_default();
                    check("menu-bar-panel", shown && inside.iter().any(|t| t == "14°, clear"), format!("shown {shown}; {inside:?}"));
                    if let Some(p) = &popover {
                        p.close();
                    }
                    print_phase(w, check, failed);
                });
            });
        });
    }

    fn print_phase(w: Retained<NSWindow>, check: Check, failed: Rc<Cell<u32>>) {
        // The Print sheet opens on the window; it's cancelled unattended.
        let print_button = buttons(&w).into_iter().find(|b| b.title().to_string() == "Print…").unwrap();
        w.makeKeyAndOrderFront(None);
        // SAFETY: as above.
        unsafe { print_button.performClick(None) };
        later_ms(1200, move || {
            let sheet = w.attachedSheet();
            check("print-sheet", sheet.is_some(), format!("sheet {:?}", sheet.as_ref().map(|s| s.title().to_string())));
            if let Some(sheet) = sheet {
                w.endSheet_returnCode(&sheet, objc2_app_kit::NSModalResponseCancel);
            }
            later_ms(400, move || {
                eprintln!("[check] done, {} failed", failed.get());
                std::process::exit(if failed.get() == 0 { 0 } else { 1 });
            });
        });
    }

    fn pdf_document(data: &[u8]) -> Option<Retained<objc2_pdf_kit::PDFDocument>> {
        // SAFETY: PDFKit's initialiser; None when unreadable.
        unsafe { objc2_pdf_kit::PDFDocument::initWithData(objc2::AnyThread::alloc(), &objc2_foundation::NSData::with_bytes(data)) }
    }

    /// The colour at (x, y), as fractions of the first page from its top
    /// left, drawn at 4 pixels per point.
    fn pdf_pixel(data: &[u8], x: f64, y: f64) -> Option<(f64, f64, f64)> {
        let doc = pdf_document(data)?;
        // SAFETY: PDFKit's page drawing into a fresh bitmap.
        unsafe {
            let page = doc.pageAtIndex(0)?;
            let bounds = page.boundsForBox(objc2_pdf_kit::PDFDisplayBox::MediaBox);
            let size = objc2_foundation::NSSize::new(bounds.size.width * 4.0, bounds.size.height * 4.0);
            let image = page.thumbnailOfSize_forBox(size, objc2_pdf_kit::PDFDisplayBox::MediaBox);
            let tiff = image.TIFFRepresentation()?;
            let rep = objc2_app_kit::NSBitmapImageRep::imageRepWithData(&tiff)?;
            let (px, py) = ((rep.pixelsWide() as f64 * x) as isize, (rep.pixelsHigh() as f64 * y) as isize);
            let c = rep.colorAtX_y(px, py)?;
            let c = c.colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace())?;
            Some((c.redComponent(), c.greenComponent(), c.blueComponent()))
        }
    }

    /// Saves a capture of `w` as `<SNAP>/media-<name>.png` when SNAP is set.
    fn snap(w: &NSWindow, name: &str) {
        let Ok(dir) = std::env::var("SNAP") else { return };
        let Some(rep) = ferrokit::testing::capture_window(w) else { return };
        // SAFETY: encoding a bitmap as PNG with no options.
        if let Some(png) = unsafe { rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()) } {
            png.writeToFile_atomically(&NSString::from_str(&format!("{dir}/media-{name}.png")), true);
        }
    }

    fn texts_in(view: &NSView) -> Vec<String> {
        all_in(view).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSTextField>().ok()).map(|f| f.stringValue().to_string()).collect()
    }

    fn text_of(w: &NSWindow) -> Vec<String> {
        w.contentView().map(|c| texts_in(&c)).unwrap_or_default()
    }

    fn buttons(w: &NSWindow) -> Vec<Retained<NSButton>> {
        w.contentView().map(|c| all_in(&c)).unwrap_or_default().into_iter().filter_map(|v| v.downcast::<NSButton>().ok()).collect()
    }

    fn pdf_views(w: &NSWindow) -> Vec<Retained<objc2_pdf_kit::PDFView>> {
        w.contentView().map(|c| all_in(&c)).unwrap_or_default().into_iter().filter_map(|v| v.downcast::<objc2_pdf_kit::PDFView>().ok()).collect()
    }

    fn player_views(w: &NSWindow) -> Vec<Retained<objc2_av_kit::AVPlayerView>> {
        w.contentView().map(|c| all_in(&c)).unwrap_or_default().into_iter().filter_map(|v| v.downcast::<objc2_av_kit::AVPlayerView>().ok()).collect()
    }

    /// Every view under `root`, including the hidden tab panes'.
    fn all_in(root: &NSView) -> Vec<Retained<NSView>> {
        fn walk(v: &NSView, out: &mut Vec<Retained<NSView>>) {
            for s in v.subviews().iter() {
                out.push(s.clone());
                walk(&s, out);
                if let Ok(tv) = s.clone().downcast::<objc2_app_kit::NSTabView>() {
                    for item in tv.tabViewItems().iter() {
                        if let Some(pane) = item.view(objc2::MainThreadMarker::new().unwrap()).filter(|p| unsafe { p.superview() }.is_none()) {
                            out.push(pane.clone());
                            walk(&pane, out);
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(root, &mut out);
        out
    }

}

mod demo {
    use ferrokit::*;

    use super::tour::{at, place, play};

    pub struct Tour {
        pub tab: Signal<i32>,
                pub pdf_page: Signal<usize>,
        pub find: Signal<String>,
        pub clip: Signal<Option<std::path::PathBuf>>,
        pub playing: Signal<bool>,
        pub muted: Signal<bool>,
    }

    pub fn run(t: Tour) {
        let Some(w) = windows().into_iter().find(|w| w.title().to_string() == "Briefing") else { return };
        set_appearance(Appearance::Light);
        place(&w, 900.0, 700.0);
        let Tour { tab, pdf_page, find, clip, playing, muted } = t;
        // The guide, a web page; then the report, a PDF: its pages, then a word found.
        let mut steps = vec![at(2200, move || animated(|| tab.set(1)))];
        steps.push(at(1000, move || pdf_page.set(1)));
        steps.push(at(900, move || pdf_page.set(0)));
        steps.push(at(700, move || find.set("ferry".into())));
        // The clip: a movie playing.
        steps.push(at(1500, move || {
            find.set(String::new());
            if let Some(movie) = std::env::var_os("DEMO_CLIP") {
                muted.set(true);
                clip.set(Some(movie.into()));
            }
            animated(|| tab.set(2));
        }));
        steps.push(at(600, move || playing.set(true)));
        steps.push(at(4000, move || playing.set(false)));
        steps.push(at(800, || {}));
        play(steps);
    }
}
