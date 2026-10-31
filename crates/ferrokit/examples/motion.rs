//! V3: motion. A packing list whose rows slide in and out, fade when
//! they're packed, highlight under the pointer and click like buttons; a
//! details panel that fades in and pushes the list down smoothly.
//! `cargo run -p ferrokit --example motion`; `CHECK=all` runs its
//! self-checks.

use ferrokit::*;

#[derive(Clone)]
struct Item {
    id: u32,
    name: String,
    symbol: &'static str,
}

impl Identifiable for Item {
    type Id = u32;
    fn id(&self) -> u32 {
        self.id
    }
}

/// The app's state, `Copy` signals.
#[derive(Clone, Copy)]
struct State {
    items: Signal<Vec<Item>>,
    packed: Signal<Vec<u32>>,
    details: Signal<bool>,
    next: Signal<u32>,
}

const MORE: [(&str, &str); 4] = [("Headlamp", "flashlight.on.fill"), ("Map", "map.fill"), ("Rain jacket", "cloud.rain.fill"), ("Snacks", "takeoutbag.and.cup.and.straw.fill")];

fn row(state: State, item: Item) -> impl View {
    let id = item.id;
    let packed = move || state.packed.get().contains(&id);
    hstack((
        image(item.symbol).symbol_size(14.0).hierarchical(Color::Accent).frame(22.0, 18.0),
        text(&item.name).font(Font::body()),
        spacer(),
        label(move || if packed() { "Packed".into() } else { String::new() }).font(Font::caption()).color(Color::Secondary),
    ))
    .spacing(10.0)
    .padding(10.0)
    .fill_width()
    .hover_highlight(Color::Quaternary)
    .corner_radius(8.0)
    .opacity_when(move || if packed() { 0.45 } else { 1.0 })
    .on_click(move || {
        animated(|| {
            state.packed.update(|p| match p.iter().position(|x| *x == id) {
                Some(i) => {
                    p.remove(i);
                }
                None => p.push(id),
            })
        })
    })
    .transition(Transition::Slide(Edge::Leading))
}

fn screen(state: State) -> impl View {
    vstack((
        hstack((
            text("Packing list").font(Font::title2().weight(Weight::Bold)),
            spacer(),
            button("Details", move || animated(|| state.details.set(!state.details.get()))),
            button("Add", move || {
                animated(|| {
                    let n = state.next.get();
                    state.next.set(n + 1);
                    let (name, symbol) = MORE[n as usize % MORE.len()];
                    state.items.update(|v| v.push(Item { id: 100 + n, name: name.into(), symbol }));
                })
            }),
            button("Remove packed", move || {
                animated(|| {
                    let packed = state.packed.get();
                    state.items.update(|v| v.retain(|i| !packed.contains(&i.id)));
                    state.packed.set(Vec::new());
                })
            }),
        )),
        dynamic(move || {
            if state.details.get() {
                AnyView::new(
                    text("Three days on the coast trail; huts each night, so no tent.")
                        .font(Font::callout())
                        .color(Color::Secondary)
                        .padding(12.0)
                        .fill_width()
                        .background_when(move || if state.packed.get().is_empty() { Color::ControlBackground } else { Color::Green.opacity(0.18) })
                        .corner_radius(10.0)
                        .transition(Transition::Fade),
                )
            } else {
                AnyView::new(spacer().height(0.0))
            }
        }),
        vstack((for_each(move || state.items.get(), move |item| row(state, item)),)).spacing(2.0),
        spacer(),
    ))
    .spacing(14.0)
    .alignment(Align::Start)
    .padding(24.0)
}

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    App::new("Motion")
        .window(Window::new("Packing").size(520.0, 520.0).view(dynamic(move || {
            let items = [("Boots", "shoeprints.fill"), ("Water bottle", "waterbottle.fill"), ("Sunscreen", "sun.max.fill")];
            let state = State {
                items: Signal::new(items.iter().enumerate().map(|(i, (n, s))| Item { id: i as u32, name: (*n).into(), symbol: s }).collect()),
                packed: Signal::new(Vec::new()),
                details: Signal::new(false),
                next: Signal::new(0),
            };
            if checks {
                self_checks::STATE.with(|s| s.set(Some(state)));
            }
            screen(state)
        })))
        .on_launch(move || {
            if checks {
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(500)), |()| self_checks::run());
            }
        })
        .run();
}

mod self_checks {
    use std::cell::Cell;
    use std::rc::Rc;

    use ferrokit::testing::{ax_audit, ax_tree, has_ax, hover, is_pressed, leaving_views, press, set_reduce_motion};
    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSTextField, NSView, NSWindow};

    use super::State;

    thread_local! {
        pub static STATE: Cell<Option<State>> = const { Cell::new(None) };
    }

    type Check = Rc<dyn Fn(&str, bool, String)>;

    /// One second, so a check can look in the middle.
    fn slow() -> Animation {
        Animation::linear(1.0)
    }

    pub fn run() {
        let w = windows()[0].clone();
        let state = STATE.with(Cell::get).unwrap();
        let failed = Rc::new(Cell::new(0));
        let check: Check = {
            let failed = failed.clone();
            Rc::new(move |name: &str, pass: bool, note: String| {
                eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
                failed.set(failed.get() + u32::from(!pass));
            })
        };
        set_reduce_motion(Some(false));
        let problems = ax_audit(&w);
        check("accessibility", problems.is_empty(), format!("{problems:?}"));
        let tree = ax_tree(&w);
        check("rows-are-buttons", has_ax(&tree, "AXButton", "", "Boots"), format!("{:?}", tree.iter().filter(|t| t.0 == "AXButton").collect::<Vec<_>>()));

        // Changes made here land when this callback's batch ends, so each
        // step reads its results a moment later.
        // Not animated: a change lands at once.
        let boots = field(&w, "Boots");
        let before = boots.frame();
        state.details.set(true);
        let (c, w2) = (check.clone(), w.clone());
        later_ms(40, move || {
            let moved = boots.frame();
            let quiet = no_animations(&boots);
            c("plain-change-instant", moved.origin.y > before.origin.y && quiet, format!("Boots y {} → {}; no animation {quiet}", before.origin.y, moved.origin.y));
            state.details.set(false);
            later_ms(40, move || animate_details(w2, state, c, failed));
        });
    }

    /// Animated: the list moves down over the second; the panel fades in.
    fn animate_details(w: Retained<NSWindow>, state: State, check: Check, failed: Rc<Cell<u32>>) {
        let boots = field(&w, "Boots");
        let start = presented(&boots).origin.y;
        animated_with(slow(), || state.details.set(true));
        let started = std::time::Instant::now();

        let c = check.clone();
        later_ms(40, move || {
            let end = model(&boots).origin.y;
            let panel = field(&w, "Three days on the coast trail; huts each night, so no tent.");
            later_ms(300, move || {
                let mid = presented(&boots).origin.y;
                let between = (mid - start) * (mid - end) < 0.0;
                c("frame-animates", (start - end).abs() > 10.0 && between, format!("layer y from {start:.1} to {end:.1} (model); presented {mid:.1} between: {between}"));
                let o = opacity(&panel);
                c("fade-in", panel.alphaValue() == 1.0 && o > 0.05 && o < 0.95, format!("panel alpha {}; presented opacity {o:.2}", panel.alphaValue()));
                // Settled within 2 s of a 1 s animation (it starts when the
                // change is committed, a frame or two in).
                fn settle(boots: Retained<objc2_app_kit::NSTextField>, end: f64, started: std::time::Instant, c: Check, then: Box<dyn FnOnce()>) {
                    let settled = (presented(&boots).origin.y - end).abs() < 0.5 && no_animations(&boots);
                    if settled || started.elapsed().as_millis() > 2000 {
                        c("animation-settles", settled, format!("presented {:.1}, model {end:.1}, after {} ms", presented(&boots).origin.y, started.elapsed().as_millis()));
                        then();
                    } else {
                        later_ms(50, move || settle(boots, end, started, c, then));
                    }
                }
                later_ms(900, move || settle(boots, end, started, c.clone(), Box::new(move || step_rows(w, state, c, failed))));
            });
        });
    }

    /// Rows: slide in, fade when packed, slide out and go.
    fn step_rows(w: Retained<NSWindow>, state: State, check: Check, failed: Rc<Cell<u32>>) {
        animated_with(slow(), || {
            state.next.set(1);
            state.items.update(|v| v.push(super::Item { id: 100, name: "Map".into(), symbol: "map.fill" }));
        });
        let c = check.clone();
        later_ms(40, move || {
            let map = field(&w, "Map");
            let end_x = model(&map).origin.x;
            later_ms(300, move || {
                let x = presented(&map).origin.x;
                let o = opacity(&map);
                c("slide-in", x < end_x - 1.0 && x > end_x - 24.0 && o > 0.05 && o < 0.95, format!("presented x {x:.1} (ends {end_x:.1}, starts 24 pt left); opacity {o:.2}"));
                later_ms(900, move || click_and_hover(w, state, c, failed));
            });
        });
    }

    fn click_and_hover(w: Retained<NSWindow>, state: State, c: Check, failed: Rc<Cell<u32>>) {
        // Click "Boots": it dims while pressed, then packs (fading to 0.45).
        let boots = field(&w, "Boots");
        let Some(row) = interaction_of(&w, "Boots") else {
            c("click-row", false, "no interaction view".into());
            return finish(failed);
        };
        press(&row, true);
        let dimmed = (boots.alphaValue() - 0.55).abs() < 0.01 && is_pressed(&row) == Some(true);
        // The release clicks, inside `animated` (default timing).
        press(&row, false);
        let (w2, c2) = (w.clone(), c.clone());
        later_ms(40, move || {
            let packed = state.packed.get() == vec![0];
            let animating = !no_animations(&boots);
            c2("click-row", dimmed && packed, format!("dimmed while pressed {dimmed}; packed after release {packed}"));
            c2("opacity-when-animates", (boots.alphaValue() - 0.45).abs() < 0.01 && animating, format!("model alpha {:.2}; animating {animating}", boots.alphaValue()));
            let name = ax_tree(&w2).into_iter().find(|t| t.0 == "AXButton" && t.2.starts_with("Boots")).map(|t| t.2);
            c2("button-name-follows-text", name.as_deref() == Some("Boots, Packed"), format!("{name:?}"));
            // The details panel's background follows the packed list.
            let panel = field(&w2, "Three days on the coast trail; huts each night, so no tent.");
            let tint = surface_under_view(&panel).and_then(|s| bg_rgba(&s));
            c2("background-when", tint.is_some_and(|(r, g, b, _)| g > r && g > b), format!("panel background {tint:.2?} (green once something's packed)"));
            // Hover: the row's highlight comes on.
            let water = interaction_of(&w2, "Water bottle").unwrap();
            let surface = surface_under(&water);
            let before = surface.as_ref().and_then(|s| bg_rgba(s)).map(|c| c.3);
            hover(&water, true);
            later_ms(400, move || {
                let after = surface.as_ref().and_then(|s| bg_rgba(s)).map(|c| c.3);
                c2("hover-highlight", before == Some(0.0) && after.is_some_and(|a| a > 0.0), format!("highlight alpha {before:?} → {after:?}"));
                hover(&water, false);
                later_ms(400, move || remove_packed(w2, state, c2, failed));
            });
        });
    }

    fn remove_packed(w: Retained<NSWindow>, state: State, check: Check, failed: Rc<Cell<u32>>) {
        let boots = field(&w, "Boots");
        animated_with(slow(), || {
            let packed = state.packed.get();
            state.items.update(|v| v.retain(|i| !packed.contains(&i.id)));
            state.packed.set(Vec::new());
        });
        let c = check.clone();
        // SAFETY (here and below): reading the view hierarchy on the main thread.
        let shown = |v: &NSView| unsafe { v.superview() }.is_some();
        later_ms(350, move || {
            let still = shown(&boots);
            let o = opacity(&boots);
            c("slide-out", still && leaving_views() > 0 && o < 0.45 && o > 0.0, format!("still shown {still}, {} views leaving; presented opacity {o:.2}", leaving_views()));
            later_ms(900, move || {
                let gone = !shown(&boots) && leaving_views() == 0 && field_opt(&w, "Boots").is_none();
                c("removed-after-transition", gone, format!("removed {}; leaving {}", !shown(&boots), leaving_views()));
                reduce(w, state, c, failed);
            });
        });
    }

    /// With Reduce Motion: layout lands at once, slides only fade.
    fn reduce(w: Retained<NSWindow>, state: State, check: Check, failed: Rc<Cell<u32>>) {
        set_reduce_motion(Some(true));
        let water = field(&w, "Water bottle");
        animated_with(slow(), || {
            state.details.set(false);
            state.items.update(|v| v.push(super::Item { id: 200, name: "Snacks".into(), symbol: "takeoutbag.and.cup.and.straw.fill" }));
        });
        let c = check.clone();
        later_ms(40, move || {
            let snacks = field(&w, "Snacks");
            let end_x = model(&snacks).origin.x;
            let moved_at_once = no_position_animation(&water);
            later_ms(300, move || {
                let x = presented(&snacks).origin.x;
                let o = opacity(&snacks);
                c("reduce-motion", moved_at_once && (x - end_x).abs() < 0.5 && o > 0.05 && o < 0.95, format!("layout moved without animating {moved_at_once}; Snacks at x {x:.1} (end {end_x:.1}) fading in, opacity {o:.2}"));
                set_reduce_motion(None);
                later_ms(900, move || finish(failed));
            });
        });
    }

    fn finish(failed: Rc<Cell<u32>>) {
        eprintln!("[check] done, {} failed", failed.get());
        std::process::exit(if failed.get() == 0 { 0 } else { 1 });
    }

    fn later_ms(ms: u64, f: impl FnOnce() + 'static) {
        spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
    }

    fn presented(v: &NSView) -> objc2_core_foundation::CGRect {
        let layer = v.layer().unwrap();
        // SAFETY: reading the layer's in-flight copy on the main thread.
        unsafe { layer.presentationLayer() }.unwrap_or(layer).frame()
    }

    fn opacity(v: &NSView) -> f32 {
        let layer = v.layer().unwrap();
        // SAFETY: as above.
        unsafe { layer.presentationLayer() }.unwrap_or(layer).opacity()
    }

    /// The layer's own (final) frame.
    fn model(v: &NSView) -> objc2_core_foundation::CGRect {
        v.layer().unwrap().frame()
    }

    fn no_animations(v: &NSView) -> bool {
        v.layer().is_none_or(|l| l.animationKeys().is_none_or(|k| k.count() == 0))
    }

    fn no_position_animation(v: &NSView) -> bool {
        v.layer().is_none_or(|l| l.animationKeys().is_none_or(|k| !k.iter().any(|k| k.to_string().contains("position") || k.to_string().contains("frame"))))
    }

    fn all(w: &NSWindow) -> Vec<Retained<NSView>> {
        fn walk(v: &NSView, out: &mut Vec<Retained<NSView>>) {
            for s in v.subviews().iter() {
                walk(&s, out);
                out.push(s);
            }
        }
        let mut out = Vec::new();
        if let Some(c) = w.contentView() {
            walk(&c, &mut out);
        }
        out
    }

    fn field_opt(w: &NSWindow, text: &str) -> Option<Retained<NSTextField>> {
        all(w).into_iter().filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.stringValue().to_string() == text)
    }

    fn field(w: &NSWindow, text: &str) -> Retained<NSTextField> {
        field_opt(w, text).unwrap_or_else(|| panic!("no field {text:?}"))
    }

    /// The interaction view over the row showing `text`.
    fn interaction_of(w: &NSWindow, text: &str) -> Option<Retained<NSView>> {
        let f = field(w, text);
        let r = f.frame();
        all(w).into_iter().find(|v| v.class().name().to_string_lossy() == "FerrokitInteraction" && {
            let i = v.frame();
            i.origin.y <= r.origin.y && i.origin.y + i.size.height >= r.origin.y + r.size.height
        })
    }

    /// The highlight surface behind an interaction view's row.
    fn surface_under(interaction: &NSView) -> Option<Retained<NSView>> {
        // SAFETY: reading the view hierarchy on the main thread.
        let subs = unsafe { interaction.superview() }?.subviews();
        subs.iter().find(|s| s.class().name().to_string_lossy() == "FerrokitSurface" && s.frame() == interaction.frame())
    }

    /// The surface behind a label (the nearest one around it).
    fn surface_under_view(v: &NSView) -> Option<Retained<NSView>> {
        // SAFETY: reading the view hierarchy on the main thread.
        let subs = unsafe { v.superview() }?.subviews();
        let f = v.frame();
        subs.iter().find(|s| {
            let r = s.frame();
            s.class().name().to_string_lossy() == "FerrokitSurface" && r.origin.x <= f.origin.x && r.origin.y <= f.origin.y && r.origin.y + r.size.height >= f.origin.y + f.size.height
        })
    }

    /// A layer's background colour, in sRGB.
    fn bg_rgba(v: &NSView) -> Option<(f64, f64, f64, f64)> {
        let color = v.layer()?.backgroundColor()?;
        let ns = objc2_app_kit::NSColor::colorWithCGColor(&color)?.colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace())?;
        Some((ns.redComponent(), ns.greenComponent(), ns.blueComponent(), ns.alphaComponent()))
    }

}
