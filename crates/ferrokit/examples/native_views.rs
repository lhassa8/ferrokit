//! Custom native views (L3): views Ferrokit doesn't have, wrapped with
//! `native(..)`. Apple's map view (`MKMapView`) follows a city picker and a
//! map style, with a delegate the view keeps; a clock dial this app draws
//! itself (an `NSView` subclass) shows the city's local time.
//! `cargo run -p ferrokit --example native_views`; `CHECK=all` runs the checks
//! (`SNAP=<dir>` also saves the window).

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

use ferrokit::*;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBezierPath, NSColor, NSView};
use objc2_core_location::CLLocationCoordinate2D;
use objc2_foundation::{NSCopying, NSObject, NSObjectProtocol, NSPoint, NSRect, NSString};
use objc2_map_kit::{MKCoordinateRegion, MKHybridMapConfiguration, MKMapConfiguration, MKMapView, MKMapViewDelegate, MKStandardMapConfiguration};

struct City {
    name: &'static str,
    latitude: f64,
    longitude: f64,
    /// Hours ahead of UTC (in November).
    utc_offset: f64,
}

const CITIES: [City; 4] = [
    City { name: "London", latitude: 51.5072, longitude: -0.1276, utc_offset: 0.0 },
    City { name: "Tokyo", latitude: 35.6762, longitude: 139.6503, utc_offset: 9.0 },
    City { name: "New York", latitude: 40.7128, longitude: -74.006, utc_offset: -5.0 },
    City { name: "Sydney", latitude: -33.8688, longitude: 151.2093, utc_offset: 11.0 },
];

/// The city's local time now, in hours (0 to 24).
fn local_hours(city: &City) -> f64 {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    ((secs / 3600.0) + city.utc_offset).rem_euclid(24.0)
}

fn region(city: &City) -> MKCoordinateRegion {
    let center = CLLocationCoordinate2D { latitude: city.latitude, longitude: city.longitude };
    // SAFETY: a plain C function over values.
    unsafe { MKCoordinateRegion::with_distance(center, 40_000.0, 40_000.0) }
}

// ---- The map's delegate: where the map is now, as text ----

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NativeViewsMapDelegate"]
    #[ivars = Signal<String>]
    struct MapDelegate;

    unsafe impl NSObjectProtocol for MapDelegate {}

    unsafe impl MKMapViewDelegate for MapDelegate {
        #[unsafe(method(mapView:regionDidChangeAnimated:))]
        fn region_changed(&self, map: &MKMapView, _animated: bool) {
            // SAFETY: the map's current region.
            let c = unsafe { map.region() }.center;
            let ns = if c.latitude >= 0.0 { "N" } else { "S" };
            let ew = if c.longitude >= 0.0 { "E" } else { "W" };
            self.ivars().set(format!("{:.2}°{ns}, {:.2}°{ew}", c.latitude.abs(), c.longitude.abs()));
        }
    }
);

impl MapDelegate {
    fn new(mtm: MainThreadMarker, center: Signal<String>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(center);
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

// ---- A view this app draws itself: a clock dial ----

thread_local! {
    /// Dials alive now (for the teardown check).
    static DIALS: Cell<i64> = const { Cell::new(0) };
}

struct DialState {
    hours: Cell<f64>,
}

impl Drop for DialState {
    fn drop(&mut self) {
        let _ = DIALS.try_with(|d| d.set(d.get() - 1));
    }
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "NativeViewsDial"]
    #[ivars = DialState]
    struct Dial;

    impl Dial {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let b = self.bounds();
            let r = b.size.width.min(b.size.height) / 2.0 - 4.0;
            let c = NSPoint::new(b.origin.x + b.size.width / 2.0, b.origin.y + b.size.height / 2.0);
            let face = NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(c.x - r, c.y - r), objc2_foundation::NSSize::new(2.0 * r, 2.0 * r)));
            NSColor::controlBackgroundColor().setFill();
            face.fill();
            NSColor::separatorColor().setStroke();
            face.setLineWidth(2.0);
            face.stroke();
            // The hour hand (twelve at the top; the view isn't flipped).
            let angle = std::f64::consts::FRAC_PI_2 - self.ivars().hours.get().rem_euclid(12.0) / 12.0 * std::f64::consts::TAU;
            let hand = NSBezierPath::bezierPath();
            hand.moveToPoint(c);
            hand.lineToPoint(NSPoint::new(c.x + angle.cos() * r * 0.6, c.y + angle.sin() * r * 0.6));
            hand.setLineWidth(4.0);
            NSColor::controlAccentColor().setStroke();
            hand.stroke();
        }

        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            true
        }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Option<Retained<NSString>> {
            // SAFETY: AppKit's constant.
            Some(unsafe { objc2_app_kit::NSAccessibilityImageRole }.copy())
        }
    }
);

impl Dial {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        DIALS.with(|d| d.set(d.get() + 1));
        let this = Self::alloc(mtm).set_ivars(DialState { hours: Cell::new(0.0) });
        // SAFETY: NSView's designated initialiser.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }
}

// ---- The window ----

#[derive(Clone, Copy)]
struct State {
    city: Signal<usize>,
    style: Signal<usize>,
    center: Signal<String>,
    map_removed: Signal<bool>,
}

fn content(s: State) -> impl View {
    let (city, style, center, map_removed) = (s.city, s.style, s.center, s.map_removed);
    let names: Vec<(&str, usize)> = CITIES.iter().enumerate().map(|(i, c)| (c.name, i)).collect();
    let delegate = MapDelegate::new(MainThreadMarker::new().expect("main thread"), center);
    hstack((
        vstack((
            text("City").font(Font::headline()),
            picker(&city, names).accessibility_label("City"),
            segmented(&style, vec![("Map", 0), ("Satellite", 1)]).accessibility_label("Map style"),
            text("Local time").font(Font::headline()),
            native(Dial::new)
                .update(move |dial| dial.ivars().hours.set(local_hours(&CITIES[city.get()])))
                .measure(|_| Size { w: 120.0, h: 120.0 })
                .flexible(false, false)
                .accessibility_label("Local time")
                .accessibility_value(move || {
                    let h = local_hours(&CITIES[city.get()]);
                    format!("{:02}:{:02} in {}", h.floor() as u32, (h.fract() * 60.0).floor() as u32, CITIES[city.get()].name)
                }),
            text(move || center.get()).color(Color::Secondary),
            spacer(),
        ))
        .alignment(Align::Start)
        .spacing(10.0)
        .width(200.0)
        .padding(16.0),
        // The delegate is weak in MapKit, so the view keeps it.
        native({
            let delegate = delegate.clone();
            move |mtm| {
                // SAFETY: NSView's initialiser.
                let map = unsafe { MKMapView::new(mtm) };
                // SAFETY: plain MapKit settings; the delegate answers
                // MKMapViewDelegate and is kept as long as the view.
                unsafe {
                    map.setShowsZoomControls(true);
                    map.setShowsCompass(true);
                    map.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                }
                map
            }
        })
        .keep(delegate)
        .update(move |map| {
            let animated = !std::env::var("CHECK").is_ok_and(|v| v == "all");
            // SAFETY: MapKit's setter, on the main thread.
            unsafe { map.setRegion_animated(region(&CITIES[city.get()]), animated) }
        })
        .update(move |map| {
            // SAFETY: MapKit's configurations (the map copies the one it's given).
            unsafe {
                let config: Retained<MKMapConfiguration> =
                    if style.get() == 1 { Retained::into_super(MKHybridMapConfiguration::new()) } else { Retained::into_super(MKStandardMapConfiguration::new()) };
                map.setPreferredConfiguration(&config);
            }
        })
        .on_remove(move |map| {
            // SAFETY: clearing the delegate before it goes.
            unsafe { map.setDelegate(None) };
            map_removed.set(true);
        })
        .min_width(320.0)
        .accessibility_label("Map"),
    ))
}

fn main() {
    let s = State { city: Signal::new(0), style: Signal::new(0), center: Signal::new(String::new()), map_removed: Signal::new(false) };
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    App::new("Native Views")
        .window(Window::new("Native Views").size(760.0, 480.0).view(content(s)))
        .on_launch(move || {
            if checks {
                self_checks::run(s);
            }
        })
        .run();
}

mod self_checks {
    use super::*;
    use objc2_app_kit::NSWindow;
    use std::rc::Rc;

    fn views(w: &NSWindow) -> Vec<Retained<NSView>> {
        fn walk(v: &NSView, out: &mut Vec<Retained<NSView>>) {
            for s in v.subviews().iter() {
                out.push(s.clone());
                walk(&s, out);
            }
        }
        let mut out = Vec::new();
        if let Some(c) = w.contentView() {
            walk(&c, &mut out);
        }
        out
    }

    fn after(ms: u64, f: impl FnOnce() + 'static) {
        spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
    }

    /// Runs `f` once `ready` holds, or after `max_ms`.
    fn until(ready: impl Fn() -> bool + 'static, max_ms: u64, f: impl FnOnce() + 'static) {
        if ready() || max_ms == 0 {
            return f();
        }
        after(50, move || until(ready, max_ms.saturating_sub(50), f));
    }

    fn check(failed: &Cell<u32>, name: &str, pass: bool, note: String) {
        eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
        failed.set(failed.get() + u32::from(!pass));
    }

    pub fn run(s: State) {
        let w = windows().into_iter().find(|w| w.title().to_string() == "Native Views").unwrap();
        let failed = Rc::new(Cell::new(0));
        let all = views(&w);
        let map = all.iter().find_map(|v| v.clone().downcast::<MKMapView>().ok()).unwrap();
        let dial = all.iter().find_map(|v| v.clone().downcast::<Dial>().ok()).unwrap();
        let near = |m: &MKMapView, c: &City| {
            // SAFETY: the map's region.
            let r = unsafe { m.region() }.center;
            (r.latitude - c.latitude).abs() < 0.05 && (r.longitude - c.longitude).abs() < 0.05
        };
        let f = failed.clone();
        let (fr, df) = (map.frame(), dial.frame());
        check(
            &f,
            "native-map-layout",
            fr.size.width >= 320.0 && (fr.size.width - (w.contentView().unwrap().frame().size.width - 240.0)).abs() < 1.0 && df.size.width == 120.0 && df.size.height == 120.0,
            format!("map {:.0}×{:.0} fills beside the column (200 + 32 padding + 8 spacing); dial {}×{}", fr.size.width, fr.size.height, df.size.width, df.size.height),
        );
        let at_london = near(&map, &CITIES[0]);
        s.city.set(1);
        s.style.set(1);
        let m2 = map.clone();
        until(move || near(&m2, &CITIES[1]) && !s.center.get_untracked().is_empty(), 3000, move || {
            let hours = dial.ivars().hours.get();
            let expected = local_hours(&CITIES[1]);
            check(&f, "native-map-follows", at_london && near(&map, &CITIES[1]), format!("London first {at_london}, then Tokyo {}", near(&map, &CITIES[1])));
            // SAFETY: the map's configuration.
            let kind = unsafe { map.preferredConfiguration() }.class().name().to_string_lossy().into_owned();
            check(&f, "native-map-style", kind.contains("Hybrid"), format!("configuration {kind} (hybrid for Satellite)"));
            let center = s.center.get_untracked();
            check(&f, "native-delegate-kept", center.starts_with("35.6") && center.contains("139.6"), format!("the delegate (kept by the view) reports {center:?}"));
            check(&f, "native-dial-follows", (hours - expected).abs() < 0.05, format!("dial at {hours:.2} h, Tokyo's time {expected:.2} h"));
            // MapKit's own zoom buttons have no name (their images are
            // "add" and "remove"): the map's parts are Apple's, so findings
            // inside the map are left out; everything else must have a name.
            let elements = testing::ax_elements(&w);
            let map_at = elements.iter().position(|e| e.subrole == "AXMapArea");
            let inside_map = |i: usize| map_at.is_some_and(|m| i > m && elements[m + 1..=i].iter().all(|e| e.depth > elements[m].depth));
            let audit: Vec<String> = testing::ax_audit(&w)
                .into_iter()
                .filter(|finding| finding.strip_prefix('#').and_then(|r| r.split(' ').next()).and_then(|n| n.parse().ok()).is_none_or(|i| !inside_map(i)))
                .collect();
            let named = elements.iter().any(|e| e.role == "AXImage" && e.name == "Local time");
            check(&f, "native-accessibility", named && map_at.is_some() && audit.is_empty(), format!("dial named {named}, map an AXMapArea {}; findings outside the map {audit:?}", map_at.is_some()));
            if let Ok(dir) = std::env::var("SNAP") {
                testing::snapshot_windows(&dir);
            }
            drop((map, dial));
            close_window(&w);
            let f2 = f.clone();
            until(move || s.map_removed.get_untracked() && DIALS.with(Cell::get) == 0, 3000, move || {
                let dials = DIALS.with(Cell::get);
                check(&f2, "native-teardown", s.map_removed.get_untracked() && dials == 0, format!("map's on_remove ran {}, dials alive {dials}", s.map_removed.get_untracked()));
                eprintln!("[check] done, {} failed", f2.get());
                std::process::exit(if f2.get() == 0 { 0 } else { 1 });
            });
        });
    }
}
