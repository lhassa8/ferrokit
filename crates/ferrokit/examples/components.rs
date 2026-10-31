//! V4: the rest of the standard controls and the full set of charts, in a
//! trip budget: tabs, a donut and stacked bars, an area chart over dates
//! in dollars, a scatter plot, radio buttons, a token field, a rating, a
//! gauge, a path control, a link and tooltips.
//! `cargo run -p ferrokit --example components`; `CHECK=all` runs its
//! self-checks.

use std::path::PathBuf;

use ferrokit::*;

const DAY: f64 = 86_400.0;
/// 2026-01-01 00:00 UTC.
const START: f64 = 1_767_225_600.0;

fn usd() -> Format {
    Format::Currency("USD".into())
}

fn overview() -> impl View {
    let spending = || vec![("Lodging".to_string(), 1240.0), ("Food".to_string(), 520.0), ("Transport".to_string(), 410.0), ("Activities".to_string(), 330.0)];
    let months = || {
        vec![
            ("Jan".to_string(), vec![820.0, 240.0, 120.0]),
            ("Feb".to_string(), vec![640.0, 260.0, 210.0]),
            ("Mar".to_string(), vec![900.0, 310.0, 160.0]),
            ("Apr".to_string(), vec![700.0, 280.0, 240.0]),
        ]
    };
    hstack((
        pie_chart(spending).donut().value_format(usd()).colors(&[Color::Indigo, Color::Orange, Color::Teal, Color::Pink]).min_size(320.0, 220.0),
        stacked_bar_chart(months).series_names(&["Lodging", "Food", "Transport"]).value_format(usd()).colors(&[Color::Indigo, Color::Orange, Color::Teal]).min_size(300.0, 220.0),
    ))
    .spacing(16.0)
    .padding(4.0)
}

fn trends() -> impl View {
    // A balance falling over 90 days, and two moving averages.
    let balance: Vec<(f64, f64)> = (0..90).map(|d| (START + d as f64 * DAY, 4200.0 - d as f64 * 31.0 + (d as f64 / 6.0).sin() * 180.0)).collect();
    let lodging: Vec<(f64, f64)> = (0..90).map(|d| (START + d as f64 * DAY, 40.0 + (d as f64 / 9.0).sin() * 12.0)).collect();
    let food: Vec<(f64, f64)> = (0..90).map(|d| (START + d as f64 * DAY, 22.0 + (d as f64 / 5.0).cos() * 6.0)).collect();
    vstack((
        area_chart(move || vec![Series::new("Balance", balance.clone()).color(Color::Green)]).value_format(usd()).x_format(Format::Date).min_size(560.0, 170.0),
        line_chart(move || vec![Series::new("Lodging", lodging.clone()).color(Color::Indigo), Series::new("Food", food.clone()).color(Color::Orange)])
            .value_format(usd())
            .x_format(Format::Date)
            .min_size(560.0, 170.0),
    ))
    .spacing(12.0)
}

fn details() -> impl View {
    let rides = vec![(12.0, 18.0), (30.0, 34.0), (5.0, 9.5), (48.0, 61.0), (22.0, 27.0), (65.0, 70.0), (9.0, 14.0), (40.0, 52.0)];
    scatter_chart(move || vec![Series::new("Rides", rides.clone()).color(Color::Purple)]).x_format(Format::Decimals(0)).value_format(usd()).min_size(560.0, 320.0)
}

/// The trip's settings: the controls.
#[derive(Clone, Copy)]
struct Settings {
    currency: Signal<String>,
    tags: Signal<Vec<String>>,
    stars: Signal<f64>,
    storage: Signal<f64>,
    folder: Signal<PathBuf>,
}

fn settings(s: Settings) -> impl View {
    grid([
        (AnyView::new(text("Currency:")), AnyView::new(radio_group(&s.currency, vec![("US dollar", "USD".to_string()), ("Euro", "EUR".to_string()), ("Yen", "JPY".to_string())]).horizontal())),
        (AnyView::new(text("Tags:")), AnyView::new(token_field(&s.tags).suggestions(&["Hiking", "Hotels", "Islands", "Food", "Ferries"]).placeholder("Add tags"))),
        (AnyView::new(text("Rating:")), AnyView::new(rating(&s.stars, 5).accessibility_label("Rating"))),
        (
            AnyView::new(text("Photo storage:")),
            AnyView::new(hstack((gauge(move || s.storage.get(), 100.0).warning(75.0).critical(90.0).accessibility_label("Photo storage"), text(move || format!("{:.0}%", s.storage.get())).font(Font::caption()).color(Color::Secondary)))),
        ),
        (AnyView::new(text("Export to:")), AnyView::new(path_control(&s.folder).accessibility_label("Export folder"))),
        (
            AnyView::new(text("")),
            AnyView::new(hstack((
                link("Ferrokit guide", "https://github.com/lhassa8/ferrokit"),
                spacer(),
                button("Reset", move || s.stars.set(0.0)).tooltip("Clears the rating"),
            ))),
        ),
    ])
    .align_column(0, Align::End)
    .spacing(10.0, 12.0)
    .padding(8.0)
}

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    let page = Signal::new(0);
    let pref = Signal::new("general");
    let s = Settings {
        currency: Signal::new("USD".to_string()),
        tags: Signal::new(vec!["Hiking".to_string(), "Islands".to_string()]),
        stars: Signal::new(4.0),
        storage: Signal::new(62.0),
        folder: Signal::new(std::env::temp_dir()),
    };
    if checks {
        self_checks::STATE.with(|c| c.set(Some((page, s))));
    }
    App::new("Components")
        .window(Window::new("Trip budget").size(700.0, 480.0).view(
            vstack((
                text("Lofoten, summer 2026").font(Font::title2().weight(Weight::Bold)),
                tabs(&page, vec![
                    ("Overview", 0, AnyView::new(overview())),
                    ("Trends", 1, AnyView::new(trends())),
                    ("Rides", 2, AnyView::new(details())),
                    ("Settings", 3, AnyView::new(settings(s))),
                ])
                .style(TabStyle::Segmented),
            ))
            .spacing(12.0)
            .alignment(Align::Start)
            .padding(20.0),
        ))
        .window(Window::new("Preferences").size(380.0, 200.0).view(
            tabs(&pref, vec![
                ("General", "general", AnyView::new(checkbox("Show totals in the menu bar", &Signal::new(true)).padding(12.0))),
                ("Sync", "sync", AnyView::new(text("Sync is off").padding(12.0))),
            ])
            .padding(16.0),
        ))
        .on_launch(move || {
            if checks {
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(500)), |()| self_checks::run());
            }
        })
        .run();
}

mod self_checks {
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use ferrokit::testing::{ax_audit, ax_tree, canvas_drawing, has_ax, send_action};
    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSAccessibility, NSButton, NSLevelIndicator, NSPathControl, NSSegmentedControl, NSTabView, NSTabViewType, NSTextField, NSTokenField, NSView, NSWindow};
    use objc2_foundation::{NSArray, NSString};

    use super::Settings;

    thread_local! {
        pub static STATE: Cell<Option<(Signal<i32>, Settings)>> = const { Cell::new(None) };
    }

    type Check = Rc<dyn Fn(&str, bool, String)>;

    pub fn run() {
        let ws = windows();
        let w = ws.iter().find(|w| w.title().to_string() == "Trip budget").unwrap().clone();
        let prefs = ws.iter().find(|w| w.title().to_string() == "Preferences").unwrap().clone();
        let (page, s) = STATE.with(Cell::get).unwrap();
        let failed = Rc::new(Cell::new(0));
        let check: Check = {
            let failed = failed.clone();
            Rc::new(move |name: &str, pass: bool, note: String| {
                eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
                failed.set(failed.get() + u32::from(!pass));
            })
        };
        let problems = [ax_audit(&w), ax_audit(&prefs)].concat();
        check("accessibility", problems.is_empty(), format!("{problems:?}"));
        // (Changes made here land when this callback returns: each phase
        // reads the last one's a moment later.)
        let meter = levels(&w).into_iter().find(|l| l.levelIndicatorStyle() == objc2_app_kit::NSLevelIndicatorStyle::ContinuousCapacity).unwrap();
        let before = meter.doubleValue();
        page.set(3);
        s.storage.set(93.0);
        later_ms(60, move || controls(w, prefs, page, s, check, failed, before));
    }

    fn controls(w: Retained<NSWindow>, prefs: Retained<NSWindow>, page: Signal<i32>, s: Settings, check: Check, failed: Rc<Cell<u32>>, before: f64) {
        snap(&w, "settings");
        snap(&prefs, "prefs");
        // Tabs, segmented: a segmented control over a tabless tab view,
        // both ways bound to the page.
        let tv = all(&w).into_iter().find_map(|v| v.downcast::<NSTabView>().ok()).unwrap();
        let seg = all(&w).into_iter().find_map(|v| v.downcast::<NSSegmentedControl>().ok());
        let shown = tv.selectedTabViewItem().map(|t| tv.indexOfTabViewItem(&t));
        // The settings pane is laid out once shown.
        let radio_w = buttons(&w).iter().find(|b| b.title().to_string() == "Euro").map(|b| b.frame().size.width).unwrap_or(0.0);
        tv.selectTabViewItemAtIndex(1);
        let back = page.get();
        check(
            "tabs-segmented",
            tv.tabViewType() == NSTabViewType::NoTabsNoBorder && seg.as_ref().is_some_and(|s| s.segmentCount() == 4) && shown == Some(3) && back == 1 && radio_w > 20.0,
            format!("tabless {:?}; segments {:?}; page 3 shows {shown:?}; choosing tab 1 sets the page to {back}; settings laid out ({radio_w} wide)", tv.tabViewType(), seg.map(|s| s.segmentCount())),
        );
        let ptv = all(&prefs).into_iter().find_map(|v| v.downcast::<NSTabView>().ok()).unwrap();
        ptv.selectTabViewItemAtIndex(1);
        let sync = all(&prefs).into_iter().filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.stringValue().to_string() == "Sync is off").is_some_and(|f| f.window().is_some() && f.frame().size.width > 0.0);
        check(
            "tabs-bordered",
            ptv.tabViewType() == NSTabViewType::TopTabsBezelBorder && ptv.numberOfTabViewItems() == 2 && has_ax(&ax_tree(&prefs), "AXTabGroup", "", "") && sync,
            format!("type {:?}, {} tabs, a tab group to VoiceOver; the Sync pane shown and laid out {sync}", ptv.tabViewType(), ptv.numberOfTabViewItems()),
        );

        // Radio group: a click sets the value (the state follows below).
        let euro = buttons(&w).into_iter().find(|b| b.title().to_string() == "Euro").unwrap();
        // SAFETY: a click on a button of this window.
        unsafe { euro.performClick(None) };
        let picked = s.currency.get();
        s.currency.set("JPY".into());

        // Token field: set from the signal, edits back, suggestions.
        let tokens = all(&w).into_iter().find_map(|v| v.downcast::<NSTokenField>().ok()).unwrap();
        let shown_tokens = token_strings(&tokens);
        let strings = NSArray::from_retained_slice(&[NSString::from_str("Hiking"), NSString::from_str("Ferries")]);
        // SAFETY: an array of strings is a token field's value.
        unsafe { tokens.setObjectValue(Some(&strings)) };
        post_notification(&tokens);
        let edited = s.tags.get();
        let completions = completions(&tokens, "f");
        check(
            "token-field",
            shown_tokens == ["Hiking", "Islands"] && edited == ["Hiking", "Ferries"] && completions == ["Food", "Ferries"],
            format!("shows {shown_tokens:?}; edited to {edited:?}; completions for 'f' {completions:?}"),
        );

        // Rating (editable) and gauge (shown, with warning and critical).
        let all_levels = levels(&w);
        let stars = all_levels.iter().find(|l| l.levelIndicatorStyle() == objc2_app_kit::NSLevelIndicatorStyle::Rating).unwrap();
        let shown_stars = stars.doubleValue();
        stars.setDoubleValue(2.0);
        send_action(stars);
        check("rating", shown_stars == 4.0 && s.stars.get() == 2.0 && stars.isEditable(), format!("shows {shown_stars}; click sets {}; editable {}", s.stars.get(), stars.isEditable()));
        let meter = all_levels.iter().find(|l| l.levelIndicatorStyle() == objc2_app_kit::NSLevelIndicatorStyle::ContinuousCapacity).unwrap();
        check(
            "gauge",
            before == 62.0 && meter.doubleValue() == 93.0 && meter.warningValue() == 75.0 && meter.criticalValue() == 90.0 && !meter.isEditable(),
            format!("{before} → {}; warning {}, critical {}", meter.doubleValue(), meter.warningValue(), meter.criticalValue()),
        );

        // Path control: shows the folder; choosing a parent sets it.
        let path = all(&w).into_iter().find_map(|v| v.downcast::<NSPathControl>().ok()).unwrap();
        let shown_path = path.URL().and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string()));
        let canon = |p: &PathBuf| p.canonicalize().unwrap_or(p.clone());
        let shows = shown_path.as_ref().is_some_and(|p| canon(p) == canon(&std::env::temp_dir()));
        path.setURL(Some(&objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str("/Users"))));
        send_action(&path);
        // Tall enough for its folder names (measured with a path in it).
        let tall = path.frame().size.height >= 20.0;
        check("path-control", shows && tall && s.folder.get() == std::path::Path::new("/Users"), format!("shows {shown_path:?}, {} pt tall; choosing /Users sets {:?}", path.frame().size.height, s.folder.get()));

        // Link and tooltip.
        let link = all(&w).into_iter().filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.stringValue().to_string() == "Ferrokit guide").unwrap();
        let attrs = link.attributedStringValue();
        // SAFETY: reading an attribute at index 0 of a non-empty string.
        let url = unsafe { attrs.attribute_atIndex_effectiveRange(objc2_app_kit::NSLinkAttributeName, 0, std::ptr::null_mut()) };
        let url = url.and_then(|u| u.downcast::<objc2_foundation::NSURL>().ok()).and_then(|u| u.absoluteString()).map(|s| s.to_string());
        let role = link.accessibilityRole().map(|r| r.to_string());
        check(
            "link",
            url.as_deref() == Some("https://github.com/lhassa8/ferrokit") && link.isSelectable() && role.as_deref() == Some("AXLink"),
            format!("url {url:?}; VoiceOver role {role:?}"),
        );
        let reset = buttons(&w).into_iter().find(|b| b.title().to_string() == "Reset").unwrap();
        let tip = reset.toolTip().map(|t| t.to_string());
        check("tooltip", tip.as_deref() == Some("Clears the rating"), format!("{tip:?}"));
        page.set(0);
        later_ms(60, move || {
            let yen_on = buttons(&w).into_iter().find(|b| b.title().to_string() == "Yen").is_some_and(|b| b.state() == objc2_app_kit::NSControlStateValueOn);
            let euro_off = euro.state() == objc2_app_kit::NSControlStateValueOff;
            check("radio-group", picked == "EUR" && yen_on && euro_off, format!("click Euro → {picked}; set Yen → Yen on {yen_on}, Euro off {euro_off}"));
            charts(w, page, check, failed);
        });
    }

    fn charts(w: Retained<NSWindow>, page: Signal<i32>, check: Check, failed: Rc<Cell<u32>>) {
        snap(&w, "overview");
        let drawings: Vec<(Retained<NSView>, Vec<String>, String)> = all(&w)
            .into_iter()
            .filter_map(|v| canvas_drawing(&v).map(|d| (v.clone(), d, v.accessibilityLabel().map(|s| s.to_string()).unwrap_or_default())))
            .collect();
        let find = |prefix: &str| drawings.iter().find(|(_, _, l)| l.starts_with(prefix)).cloned();
        let donut = find("Donut chart");
        check(
            "chart-donut",
            donut.as_ref().is_some_and(|(_, d, l)| d.iter().filter(|o| o.starts_with("fill path")).count() == 4 && l.contains("4 slices") && l.contains("Lodging") && l.contains('$') && d.iter().any(|t| t.contains("Total"))),
            format!("{:?}", donut.as_ref().map(|(_, d, l)| (l.clone(), d.iter().filter(|o| o.starts_with("fill path")).count()))),
        );
        let stacked = find("Stacked bar chart");
        let segments = stacked.as_ref().map(|(_, d, _)| d.iter().filter(|o| o.starts_with("fill rect") && (o.ends_with("Indigo") || o.ends_with("Orange") || o.ends_with("Teal")) && !o.contains("10x10")).count());
        let legend = stacked.as_ref().is_some_and(|(_, d, _)| ["\"Lodging\"", "\"Food\"", "\"Transport\""].iter().all(|n| d.iter().any(|o| o.contains(n))));
        check("chart-stacked", segments == Some(12) && legend, format!("{segments:?} segments (12); legend {legend}; {:?}", stacked.map(|s| s.2)));
        let images = drawings.iter().filter(|(v, _, l)| v.accessibilityRole().is_some_and(|r| r.to_string() == "AXImage") && !l.is_empty()).count();
        check("charts-voiceover", images == 5, format!("{images} of 5 charts are images with a description"));
        page.set(1);
        later_ms(80, move || {
            let w2 = w.clone();
            let get = move |prefix: &str| all(&w2).into_iter().filter_map(|v| canvas_drawing(&v).map(|d| (v, d))).find(|(v, _)| v.accessibilityLabel().is_some_and(|l| l.to_string().starts_with(prefix)));
            snap(&w, "trends");
            let area = get("Area chart");
            let texts: Vec<String> = area.as_ref().map(|(_, d)| d.iter().filter(|o| o.starts_with("text")).cloned().collect()).unwrap_or_default();
            let months = ["Jan", "Feb", "Mar"].iter().all(|m| texts.iter().any(|o| o.starts_with(&format!("text \"{m}"))));
            let money = texts.iter().any(|o| o.contains('$'));
            let filled = area.as_ref().is_some_and(|(_, d)| d.iter().any(|o| o.starts_with("fill path") && o.ends_with("Green")));
            check("chart-area-dates", months && money && filled, format!("month ticks Jan, Feb, Mar {months}; dollar ticks {money}; filled {filled}; {texts:?}"));
            let line = get("Line chart");
            let colours = line.as_ref().is_some_and(|(_, d)| d.iter().any(|o| o.starts_with("stroke path") && o.ends_with("Indigo")) && d.iter().any(|o| o.starts_with("stroke path") && o.ends_with("Orange")));
            let legend = line.as_ref().is_some_and(|(_, d)| d.iter().any(|o| o.contains("\"Lodging\"")) && d.iter().any(|o| o.contains("\"Food\"")));
            check("chart-series-colours", colours && legend, format!("series in their own colours {colours}; legend {legend}"));
            page.set(2);
            later_ms(80, move || {
                snap(&w, "rides");
                page.set(3);
                let scatter = get("Scatter chart");
                let w3 = w.clone();
                let dots = scatter.as_ref().map(|(_, d)| d.iter().filter(|o| o.starts_with("fill ellipse") && o.ends_with("Purple")).count());
                check("chart-scatter", dots == Some(8), format!("{dots:?} dots (8); {:?}", scatter.map(|s| s.0.accessibilityLabel())));
                later_ms(300, move || {
                    snap(&w3, "settings-later");
                    eprintln!("[check] done, {} failed", failed.get());
                    std::process::exit(if failed.get() == 0 { 0 } else { 1 });
                });
            });
        });
    }

    /// Saves a capture of `w` as `<SNAP>/components-<name>.png` when SNAP is set.
    fn snap(w: &NSWindow, name: &str) {
        let Ok(dir) = std::env::var("SNAP") else { return };
        let Some(rep) = ferrokit::testing::capture_window(w) else { return };
        // SAFETY: encoding a bitmap as PNG with no options.
        if let Some(png) = unsafe { rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()) } {
            png.writeToFile_atomically(&NSString::from_str(&format!("{dir}/components-{name}.png")), true);
        }
    }

    fn levels(w: &NSWindow) -> Vec<Retained<NSLevelIndicator>> {
        all(w).into_iter().filter_map(|v| v.downcast::<NSLevelIndicator>().ok()).collect()
    }

    fn later_ms(ms: u64, f: impl FnOnce() + 'static) {
        spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
    }

    fn post_notification(field: &NSTextField) {
        // SAFETY: AppKit's notification name, posted for the field.
        unsafe { objc2_foundation::NSNotificationCenter::defaultCenter().postNotificationName_object(objc2_app_kit::NSControlTextDidChangeNotification, Some(field)) };
    }

    fn token_strings(field: &NSTokenField) -> Vec<String> {
        field
            .objectValue()
            .and_then(|o| o.downcast::<NSArray>().ok())
            .map(|a| a.iter().filter_map(|o| o.downcast::<NSString>().ok()).map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }

    /// What the token field's delegate offers for `typed`.
    fn completions(field: &NSTokenField, typed: &str) -> Vec<String> {
        // SAFETY: the delegate is ferrokit's, which implements the method.
        let delegate = field.delegate().unwrap();
        let mut selected: isize = -1;
        let result: Option<Retained<NSArray>> = unsafe {
            objc2::msg_send![&*delegate, tokenField: field, completionsForSubstring: &*NSString::from_str(typed), indexOfToken: 0isize, indexOfSelectedItem: &mut selected]
        };
        result.map(|a| a.iter().filter_map(|o| o.downcast::<NSString>().ok()).map(|s| s.to_string()).collect()).unwrap_or_default()
    }

    fn buttons(w: &NSWindow) -> Vec<Retained<NSButton>> {
        all(w).into_iter().filter_map(|v| v.downcast::<NSButton>().ok()).collect()
    }

    /// Every view in the window, including the tab panes' (which are in
    /// the tab view's items, not all in the hierarchy at once).
    fn all(w: &NSWindow) -> Vec<Retained<NSView>> {
        fn walk(v: &NSView, out: &mut Vec<Retained<NSView>>) {
            for s in v.subviews().iter() {
                walk(&s, out);
                out.push(s.clone());
                if let Ok(tv) = s.clone().downcast::<NSTabView>() {
                    for item in tv.tabViewItems().iter() {
                        if let Some(pane) = item.view(objc2::MainThreadMarker::new().unwrap()).filter(|p| unsafe { p.superview() }.is_none()) {
                            walk(&pane, out);
                            out.push(pane);
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        if let Some(c) = w.contentView() {
            walk(&c, &mut out);
        }
        out
    }
}
