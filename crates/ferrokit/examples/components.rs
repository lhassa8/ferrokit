//! V4: the rest of the standard controls and the full set of charts, in a
//! trip budget: tabs, a donut and stacked bars, an area chart over dates
//! in dollars, a scatter plot, a histogram, a heatmap, sparklines and
//! threshold rules (C6), radio buttons, a token field, a rating, a gauge,
//! a path control, a link and tooltips.
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

/// What's selected and zoomed in the charts (C9).
#[derive(Clone, Copy)]
struct Picks {
    month: Signal<Option<usize>>,
    share: Signal<Option<usize>>,
    zoom: Signal<Option<(f64, f64)>>,
    day: Signal<Option<f64>>,
}

fn overview(k: Picks) -> impl View {
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
        pie_chart(spending).donut().value_format(usd()).colors(&[Color::Indigo, Color::Orange, Color::Teal, Color::Pink]).selection(&k.share).min_size(320.0, 220.0),
        stacked_bar_chart(months)
            .series_names(&["Lodging", "Food", "Transport"])
            .value_format(usd())
            .colors(&[Color::Indigo, Color::Orange, Color::Teal])
            .rule(Rule::y(1300.0).label("Budget"))
            .selection(&k.month)
            .min_size(300.0, 220.0),
    ))
    .spacing(16.0)
    .padding(4.0)
}

fn trends(k: Picks) -> impl View {
    // A balance falling over 90 days, and two moving averages.
    let balance: Vec<(f64, f64)> = (0..90).map(|d| (START + d as f64 * DAY, 4200.0 - d as f64 * 31.0 + (d as f64 / 6.0).sin() * 180.0)).collect();
    let lodging: Vec<(f64, f64)> = (0..90).map(|d| (START + d as f64 * DAY, 40.0 + (d as f64 / 9.0).sin() * 12.0)).collect();
    let food: Vec<(f64, f64)> = (0..90).map(|d| (START + d as f64 * DAY, 22.0 + (d as f64 / 5.0).cos() * 6.0)).collect();
    vstack((
        area_chart(move || vec![Series::new("Balance", balance.clone()).color(Color::Green)])
            .value_format(usd())
            .x_format(Format::Date)
            .rule(Rule::y(1500.0).label("Keep in reserve").color(Color::Orange))
            .rule(Rule::x(START + 59.0 * DAY).label("Flights").color(Color::Secondary))
            .zoomable()
            .x_range(&k.zoom)
            .min_size(560.0, 170.0),
        line_chart(move || vec![Series::new("Lodging", lodging.clone()).color(Color::Indigo), Series::new("Food", food.clone()).color(Color::Orange)])
            .value_format(usd())
            .x_format(Format::Date)
            .x_selection(&k.day)
            .min_size(560.0, 170.0),
    ))
    .spacing(12.0)
}

fn details() -> impl View {
    let rides = vec![(12.0, 18.0), (30.0, 34.0), (5.0, 9.5), (48.0, 61.0), (22.0, 27.0), (65.0, 70.0), (9.0, 14.0), (40.0, 52.0)];
    scatter_chart(move || vec![Series::new("Rides", rides.clone()).color(Color::Purple)]).x_format(Format::Decimals(0)).value_format(usd()).min_size(560.0, 320.0)
}

/// C6: what rides cost, when the money went, and the week's trends.
fn analysis() -> impl View {
    let costs: Vec<f64> = (0..60).map(|i| 8.0 + ((i * 37) % 23) as f64 + ((i * 11) % 7) as f64 * 4.0).collect();
    let average = costs.iter().sum::<f64>() / costs.len() as f64;
    let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let spent = move || {
        let mut cells = Vec::new();
        for week in 1..=8 {
            for (d, day) in days.iter().enumerate() {
                // Week 3 has no Sunday yet: a missing cell.
                if week == 3 && d == 6 {
                    continue;
                }
                cells.push((format!("W{week}"), day.to_string(), (20 + (week * 13 + d * 29) % 90 + if d >= 5 { 60 } else { 0 }) as f64));
            }
        }
        cells
    };
    let food = vec![22.0, 31.0, 18.0, 27.0, 40.0, 35.0, 44.0];
    let rides = vec![12.0, 9.0, 15.0, 7.0, 6.0, 5.0, 4.0];
    let trend = |name: &'static str, values: Vec<f64>, color: Color| {
        let last = *values.last().unwrap();
        hstack((text(name).fill_width(), sparkline(move || values.clone()).colors(&[color]).value_format(usd()).min_size(120.0, 24.0), text(format!("${last:.0}")).font(Font::body().monospaced_digits())))
            .spacing(10.0)
    };
    vstack((
        hstack((
            histogram(move || costs.clone()).bins(6).value_format(usd()).colors(&[Color::Purple]).rule(Rule::x(average).label("Average")).min_size(280.0, 190.0),
            heatmap(spent).value_format(usd()).colors(&[Color::Teal]).min_size(280.0, 190.0),
        ))
        .spacing(16.0),
        trend("Food per day", food, Color::Orange),
        trend("Rides per day", rides, Color::Purple),
    ))
    .spacing(10.0)
    .padding(4.0)
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
    let k = Picks { month: Signal::new(None), share: Signal::new(None), zoom: Signal::new(None), day: Signal::new(None) };
    if checks {
        self_checks::STATE.with(|c| c.set(Some((page, s))));
        self_checks::PICKS.with(|c| c.set(Some(k)));
    }
    App::new("Components")
        .window(Window::new("Trip budget").size(700.0, 480.0).view(
            vstack((
                text("Lofoten, summer 2026").font(Font::title2().weight(Weight::Bold)),
                tabs(&page, vec![
                    ("Overview", 0, AnyView::new(overview(k))),
                    ("Trends", 1, AnyView::new(trends(k))),
                    ("Rides", 2, AnyView::new(details())),
                    ("Settings", 3, AnyView::new(settings(s))),
                    ("Analysis", 4, AnyView::new(analysis())),
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

    use ferrokit::testing::{ax_audit, ax_tree, canvas_click, canvas_drawing, canvas_magnify, canvas_scroll, has_ax, send_action};
    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSAccessibility, NSButton, NSLevelIndicator, NSPathControl, NSSegmentedControl, NSTabView, NSTabViewType, NSTextField, NSTokenField, NSView, NSWindow};
    use objc2_foundation::{NSArray, NSString};

    use super::{Picks, Settings};

    thread_local! {
        pub static STATE: Cell<Option<(Signal<i32>, Settings)>> = const { Cell::new(None) };
        pub static PICKS: Cell<Option<Picks>> = const { Cell::new(None) };
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
            tv.tabViewType() == NSTabViewType::NoTabsNoBorder && seg.as_ref().is_some_and(|s| s.segmentCount() == 5) && shown == Some(3) && back == 1 && radio_w > 20.0,
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
        check("chart-stacked", segments == Some(12) && legend, format!("{segments:?} segments (12); legend {legend}; {:?}", stacked.as_ref().map(|s| &s.2)));
        // The budget rule: a red line across, labelled, read by VoiceOver,
        // and taken into the range (the tallest month is 1,370).
        let rule = stacked.as_ref().is_some_and(|(_, d, l)| d.iter().any(|o| o.starts_with("stroke path 2 points Red")) && d.iter().any(|o| o == "text \"Budget\" 11pt") && l.contains("Budget at $1,300"));
        check("chart-rule", rule, format!("{:?}", stacked.as_ref().map(|s| s.2.clone())));
        // C9: clicking a bar selects its month; clicking off clears it.
        let k = PICKS.with(Cell::get).unwrap();
        if let Some((v, _, _)) = &stacked {
            let (cw, ch) = (v.frame().size.width, v.frame().size.height);
            // Mar, the third of four columns (past the axis labels).
            canvas_click(v, 55.0 + (cw - 67.0) * 0.625, ch * 0.7);
        }
        if let Some((v, _, _)) = &donut {
            // The ring's left edge, at nine o'clock: three quarters round from
            // twelve, in Transport (from 70.4% to 86.8%).
            let ch = v.frame().size.height;
            canvas_click(v, 18.0, ch / 2.0);
        }
        // (VoiceOver's labels follow when this callback returns.)
        let (bars, ring) = (stacked.as_ref().map(|s| s.0.clone()), donut.as_ref().map(|s| s.0.clone()));
        let check2 = check.clone();
        later_ms(60, move || {
            let check = check2;
            if let (Ok(dir), Some(v)) = (std::env::var("SNAP"), &bars) {
                let _ = std::fs::write(format!("{dir}/components-bars-selected.png"), ferrokit::testing::canvas_png(v).unwrap_or_default());
            }
            let said = |v: &Option<Retained<NSView>>| v.as_ref().and_then(|v| v.accessibilityLabel()).map(|l| l.to_string()).unwrap_or_default();
            let (bar_said, slice_said) = (said(&bars), said(&ring));
            let picked = k.month.get();
            if let Some(v) = &bars {
                canvas_click(v, 2.0, 2.0);
            }
            check("chart-select", picked == Some(2) && bar_said.ends_with("; selected Mar ($1,370)") && k.month.get().is_none(), format!("clicked Mar → {picked:?}; VoiceOver {bar_said:?}; off the bars → {:?}", k.month.get()));
            check("chart-select-slice", k.share.get() == Some(2) && slice_said.ends_with("; selected Transport ($410)"), format!("{:?}; {slice_said:?}", k.share.get()));
            k.share.set(None);
        });
        let images = drawings.iter().filter(|(v, _, l)| v.accessibilityRole().is_some_and(|r| r.to_string() == "AXImage") && !l.is_empty()).count();
        check("charts-voiceover", images == 9, format!("{images} of 9 charts are images with a description"));
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
            let label = area.as_ref().and_then(|(v, _)| v.accessibilityLabel()).map(|l| l.to_string()).unwrap_or_default();
            let rules = area.as_ref().is_some_and(|(_, d)| d.iter().any(|o| o.starts_with("stroke path 2 points Orange")) && d.iter().any(|o| o.starts_with("stroke path 2 points Secondary") && o != "stroke path 2 points Separator"));
            check("chart-rules-dates", rules && label.contains("Keep in reserve at $1,500") && label.contains("Flights at ") && label.ends_with("2026"), label);
            let line = get("Line chart");
            let colours = line.as_ref().is_some_and(|(_, d)| d.iter().any(|o| o.starts_with("stroke path") && o.ends_with("Indigo")) && d.iter().any(|o| o.starts_with("stroke path") && o.ends_with("Orange")));
            let legend = line.as_ref().is_some_and(|(_, d)| d.iter().any(|o| o.contains("\"Lodging\"")) && d.iter().any(|o| o.contains("\"Food\"")));
            check("chart-series-colours", colours && legend, format!("series in their own colours {colours}; legend {legend}"));
            // C9: pinch to zoom the area chart, scroll sideways along it,
            // pinch out to see it all; click the line chart to pick a day.
            let k = PICKS.with(Cell::get).unwrap();
            let full = 89.0 * super::DAY;
            if let Some((v, _)) = &area {
                let (cw, ch) = (v.frame().size.width, v.frame().size.height);
                let pinched = canvas_magnify(v, 1.0, cw / 2.0, ch / 2.0);
                let zoomed = k.zoom.get();
                let half = zoomed.is_some_and(|(a, b)| ((b - a) / full - 0.5).abs() < 0.01);
                let vertical = canvas_scroll(v, 0.0, -40.0, cw / 2.0, ch / 2.0);
                let moved = canvas_scroll(v, -100.0, 0.0, cw / 2.0, ch / 2.0);
                let later = k.zoom.get().zip(zoomed).is_some_and(|(n, o)| n.0 > o.0 && ((n.1 - n.0) - (o.1 - o.0)).abs() < 1.0);
                let out = canvas_magnify(v, -0.95, cw / 2.0, ch / 2.0);
                check(
                    "chart-zoom",
                    pinched && half && !vertical && moved && later && out && k.zoom.get().is_none(),
                    format!("pinched {pinched}: half the days {half}; vertical scroll passed on {}; sideways moved later {later}; pinched out → {:?}", !vertical, k.zoom.get()),
                );
                k.zoom.set(Some((super::START + 10.0 * super::DAY, super::START + 20.0 * super::DAY)));
            }
            if let Some((v, _)) = &line {
                let (cw, ch) = (v.frame().size.width, v.frame().size.height);
                canvas_click(v, cw / 2.0, ch / 2.0);
                let day = k.day.get();
                let midway = day.is_some_and(|d| ((d - super::START) / super::DAY - 44.5).abs() < 3.0 && (d - super::START) % super::DAY == 0.0);
                check("chart-select-x", midway, format!("clicked the middle → day {:?}", day.map(|d| (d - super::START) / super::DAY)));
            }
            let w5 = w.clone();
            later_ms(80, move || {
                snap(&w5, "trends-zoomed");
                page.set(2);
            });
            later_ms(160, move || {
                snap(&w, "rides");
                page.set(3);
                let scatter = get("Scatter chart");
                let w3 = w.clone();
                let dots = scatter.as_ref().map(|(_, d)| d.iter().filter(|o| o.starts_with("fill ellipse") && o.ends_with("Purple")).count());
                check("chart-scatter", dots == Some(8), format!("{dots:?} dots (8); {:?}", scatter.map(|s| s.0.accessibilityLabel())));
                // The zoom and day set before: drawn, and read by VoiceOver.
                let k = PICKS.with(Cell::get).unwrap();
                let area = get("Area chart");
                let label = area.as_ref().and_then(|(v, _)| v.accessibilityLabel()).map(|l| l.to_string()).unwrap_or_default();
                let ticks = area.as_ref().map(|(_, d)| d.iter().filter(|o| o.starts_with("text \"Jan") || o.starts_with("text \"Feb")).count()).unwrap_or(0);
                let line = get("Line chart");
                let marked = line.as_ref().is_some_and(|(_, d)| d.iter().any(|o| o.starts_with("stroke path 2 points Accent")));
                let read = line.as_ref().and_then(|(v, _)| v.accessibilityLabel()).is_some_and(|l| l.to_string().contains("; selected x "));
                check("chart-zoom-drawn", label.contains("; showing x from Jan 1") && ticks >= 3 && marked && read, format!("{label:?}; {ticks} day ticks; day marked {marked}, read {read}"));
                k.zoom.set(None);
                page.set(4);
                later_ms(120, move || analysis_checks(w3, check, failed));
            });
        });
    }

    fn analysis_checks(w: Retained<NSWindow>, check: Check, failed: Rc<Cell<u32>>) {
        snap(&w, "analysis");
        let w2 = w.clone();
        let get = move |prefix: &str| all(&w2).into_iter().filter_map(|v| canvas_drawing(&v).map(|d| (v, d))).filter(|(v, _)| v.accessibilityLabel().is_some_and(|l| l.to_string().starts_with(prefix))).collect::<Vec<_>>();
        let label = |v: &NSView| v.accessibilityLabel().map(|l| l.to_string()).unwrap_or_default();
        // Histogram: 60 rides in bins on round dollars, the average marked.
        let hist = get("Histogram").pop();
        let bars = hist.as_ref().map_or(0, |(_, d)| d.iter().filter(|o| o.starts_with("fill rect") && o.ends_with("Purple")).count());
        let hist_label = hist.as_ref().map(|(v, _)| label(v)).unwrap_or_default();
        let average = hist.as_ref().is_some_and(|(_, d)| d.iter().any(|o| o == "text \"Average\" 11pt") && d.iter().any(|o| o.starts_with("stroke path 2 points Red")));
        check("chart-histogram", (4..=8).contains(&bars) && hist_label.starts_with("Histogram of 60 values") && hist_label.contains('$') && hist_label.contains("Average at $") && average, format!("{bars} bars; {hist_label}"));
        // Heatmap: 8 weeks by 7 days, one cell missing, the scale above.
        let heat = get("Heatmap").pop();
        let teal = |o: &&String| o.starts_with("fill rect") && o.ends_with("Teal");
        let cells = heat.as_ref().map_or(0, |(_, d)| d.iter().filter(teal).filter(|o| !o.contains(" 10x10 ")).count());
        let scale = heat.as_ref().map_or(0, |(_, d)| d.iter().filter(teal).filter(|o| o.contains(" 10x10 ")).count());
        let heat_label = heat.as_ref().map(|(v, _)| label(v)).unwrap_or_default();
        let labels = heat.as_ref().is_some_and(|(_, d)| ["\"Mon\"", "\"Sun\"", "\"W1\""].iter().all(|n| d.iter().any(|o| o.contains(n))));
        check("chart-heatmap", cells == 55 && scale == 5 && labels && heat_label.starts_with("Heatmap, 7 rows by 8 columns"), format!("{cells} cells (55), scale of {scale}; labels {labels}; {heat_label}"));
        // Sparklines: a line and a dot each, no axes, a fixed height.
        let sparks = get("Sparkline");
        let shapes = sparks.iter().all(|(_, d)| d.iter().any(|o| o.starts_with("stroke path 7 points")) && d.iter().filter(|o| o.starts_with("fill ellipse")).count() == 1 && !d.iter().any(|o| o.starts_with("text")));
        let heights: Vec<f64> = sparks.iter().map(|(v, _)| v.frame().size.height).collect();
        let first = sparks.first().map(|(v, _)| label(v)).unwrap_or_default();
        check(
            "sparkline",
            sparks.len() == 2 && shapes && heights.iter().all(|h| *h == 24.0) && first == "Sparkline, 7 values from $18 to $44, last $44",
            format!("{} sparklines; line and dot, no text {shapes}; heights {heights:?}; {first}", sparks.len()),
        );
        let problems = ax_audit(&w);
        check("analysis-accessibility", problems.is_empty(), format!("{problems:?}"));
        later_ms(300, move || {
            snap(&w, "analysis-later");
            eprintln!("[check] done, {} failed", failed.get());
            std::process::exit(if failed.get() == 0 { 0 } else { 1 });
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
