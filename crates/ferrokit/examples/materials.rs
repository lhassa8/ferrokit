//! V2: materials and window chrome. A travel journal: content under a
//! transparent title bar, a sidebar material reaching up behind the window
//! buttons, Liquid Glass over a picture (a material on macOS 15), a HUD
//! panel, and a window with its own background colour.
//! `cargo run -p ferrokit --example materials`; `CHECK=all` runs its
//! self-checks; `SNAP=<dir>` also saves light and dark window images.

use ferrokit::*;

const PLACES: [(&str, &str, Color); 5] = [
    ("Lofoten", "mountain.2.fill", Color::Blue),
    ("Kyoto", "leaf.fill", Color::Red),
    ("Lisbon", "sun.max.fill", Color::Orange),
    ("Patagonia", "wind", Color::Teal),
    ("Reykjavík", "snowflake", Color::Indigo),
];

fn sidebar() -> impl View {
    let mut rows: Vec<AnyView> = vec![AnyView::new(
        text("Places").font(Font::subheadline().weight(Weight::Semibold)).color(Color::Secondary).padding(6.0),
    )];
    for (i, (name, symbol, tint)) in PLACES.into_iter().enumerate() {
        let row = hstack((
            image(symbol).symbol_size(13.0).hierarchical(tint).frame(18.0, 16.0),
            text(name).font(Font::body()),
            spacer(),
        ))
        .spacing(8.0)
        .padding(6.0)
        .fill_width();
        // The selected row: the system's quiet highlight.
        rows.push(if i == 0 { AnyView::new(row.background(Color::Quaternary).corner_radius(6.0)) } else { AnyView::new(row) });
    }
    rows.push(AnyView::new(spacer()));
    vstack(rows).spacing(2.0).alignment(Align::Start).padding(10.0).material(Material::Sidebar)
}

/// A stat on a glass card.
fn stat(caption: &str, value: &str) -> impl View {
    vstack((
        text(caption).font(Font::caption().weight(Weight::Medium)).color(Color::Secondary),
        text(value).font(Font::title2().weight(Weight::Semibold).monospaced_digits()),
    ))
    .spacing(2.0)
    .alignment(Align::Start)
    .padding(12.0)
    .fill_width()
    .background(Color::ControlBackground)
    .corner_radius(10.0)
    .border(Color::Separator, 0.5)
}

fn content(cover: &[u8]) -> impl View {
    vstack((
        // The hero: a picture with a glass caption over its corner.
        zstack((
            image_data(cover).fill().corner_radius(14.0).description("Lofoten at dusk").fill_width().fill_height(),
            vstack((
                text("Lofoten").font(Font::title().weight(Weight::Bold)),
                text("Nine days above the Arctic Circle").font(Font::callout()).color(Color::Secondary),
            ))
            .spacing(2.0)
            .alignment(Align::Start)
            .padding(14.0)
            .glass(Glass::Regular)
            .corner_radius(14.0)
            .padding(16.0),
        ))
        .alignment(Align::Start, Align::End)
        .height(280.0),
        hstack((stat("Days", "9"), stat("Distance", "412 km"), stat("Photos", "1,284"))).spacing(12.0),
        text("Mountains straight out of the sea, and the light that never quite leaves in summer.").font(Font::body()).color(Color::Secondary),
        spacer(),
    ))
    .spacing(16.0)
    .alignment(Align::Start)
    .padding(24.0)
}

/// A dusk picture for the hero: an orange-to-violet sky over dark ridges
/// (`w`×`h` pixels, PNG).
fn dusk_png(w: usize, h: usize) -> Vec<u8> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    let ridge = |x: f64, f: f64, a: f64, base: f64| base - a * ((x * f).sin() * 0.6 + (x * f * 2.3 + 1.0).sin() * 0.4);
    // SAFETY: a plain RGBA bitmap, written byte by byte within its rows.
    unsafe {
        let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            objc2::AnyThread::alloc(),
            std::ptr::null_mut(),
            w as isize,
            h as isize,
            8,
            4,
            true,
            false,
            objc2_app_kit::NSDeviceRGBColorSpace,
            w as isize * 4,
            32,
        )
        .unwrap();
        let data = rep.bitmapData();
        let row = rep.bytesPerRow() as usize;
        for y in 0..h {
            let t = y as f64 / h as f64;
            // Violet at the top, through pink, to orange at the horizon.
            let sky = if t < 0.5 {
                let k = t / 0.5;
                (0.30 + 0.62 * k, 0.20 + 0.18 * k, 0.55 - 0.05 * k)
            } else {
                let k = (t - 0.5) / 0.5;
                (0.92 + 0.06 * k, 0.38 + 0.30 * k, 0.50 - 0.30 * k)
            };
            for x in 0..w {
                let fx = x as f64 / w as f64;
                let far = ridge(fx, 9.0, 0.08, 0.62) * h as f64;
                let near = ridge(fx + 0.3, 5.0, 0.12, 0.78) * h as f64;
                let (r, g, b) = if (y as f64) > near {
                    (0.08, 0.07, 0.14)
                } else if (y as f64) > far {
                    (0.22, 0.16, 0.30)
                } else {
                    sky
                };
                let p = data.add(y * row + x * 4);
                *p = (r * 255.0) as u8;
                *p.add(1) = (g * 255.0) as u8;
                *p.add(2) = (b * 255.0) as u8;
                *p.add(3) = 255;
            }
        }
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()).unwrap().to_vec()
    }
}

#[path = "shared/tour.rs"]
mod tour;

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    // `DEMO=1`: a short tour, for recording.
    let demo = std::env::var("DEMO").is_ok();
    let cover = dusk_png(900, 420);
    App::new("Materials")
        .window(
            Window::new("Journal")
                .size(900.0, 620.0)
                .full_size_content()
                .hide_title()
                .toolbar(vec![toolbar_space(), toolbar_button("share", "Share", "square.and.arrow.up", || {})])
                .view(split((sidebar(), content(&cover))).position(210.0)),
        )
        .window(
            Window::new("Now Playing").size(300.0, 120.0).transparent_title_bar().view(
                hstack((
                    image("music.note").symbol_size(22.0).color(Color::rgb(1.0, 1.0, 1.0)).frame(28.0, 28.0),
                    vstack((text("Northern Lights").font(Font::headline()), text("Ambient · 4:12").font(Font::caption()).color(Color::Secondary)))
                        .spacing(2.0)
                        .alignment(Align::Start),
                    spacer(),
                ))
                .spacing(12.0)
                .padding(18.0)
                .material(Material::Hud),
            ),
        )
        .window(
            Window::new("Sand")
                .size(300.0, 140.0)
                .toolbar_style(ToolbarStyle::UnifiedCompact)
                .toolbar(vec![toolbar_space(), toolbar_button("add", "Add", "plus", || {})])
                .transparent_title_bar()
                .background(Color::hex("#e9dcc4"))
                .view(text("A window with its own colour").font(Font::title3()).padding(20.0)),
        )
        .on_launch(move || {
            if checks {
                // Once the windows are on screen, for the captures.
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(600)), |()| self_checks::run());
            } else if demo {
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(300)), |()| demo::run());
            }
        })
        .run();
}

mod demo {
    use ferrokit::*;
    use objc2_foundation::NSPoint;

    use super::tour::{at, place, play};

    pub fn run() {
        let find = |title: &str| windows().into_iter().find(|w| w.title().to_string() == title);
        let (Some(journal), Some(playing)) = (find("Journal"), find("Now Playing")) else { return };
        set_appearance(Appearance::Light);
        place(&journal, 1000.0, 680.0);
        // The player floats over the journal's lower right corner.
        let f = journal.frame();
        playing.setFrameOrigin(NSPoint::new(f.origin.x + f.size.width - playing.frame().size.width - 28.0, f.origin.y + 28.0));
        playing.orderFront(None);
        let steps = vec![
            at(2600, || set_appearance(Appearance::Dark)),
            at(2400, || set_appearance(Appearance::Light)),
            at(1400, || {}),
        ];
        play(steps);
    }
}

mod self_checks {
    use ferrokit::testing::{ax_audit, capture_window};
    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_foundation::NSObjectProtocol;
    use objc2_app_kit::{NSBitmapImageRep, NSTextField, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectView, NSWindow, NSWindowStyleMask};

    type Rgb = (f64, f64, f64);

    pub fn run() {
        let all_windows = windows();
        let find = |t: &str| all_windows.iter().find(|w| w.title().to_string() == t).unwrap().clone();
        let (w, hud, sand) = (find("Journal"), find("Now Playing"), find("Sand"));
        let failed = std::rc::Rc::new(std::cell::Cell::new(0));
        let check = {
            let failed = failed.clone();
            move |name: &str, pass: bool, note: String| {
                eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
                failed.set(failed.get() + u32::from(!pass));
            }
        };
        let problems = [ax_audit(&w), ax_audit(&hud), ax_audit(&sand)].concat();
        check("accessibility", problems.is_empty(), format!("{problems:?}"));

        // Window chrome read back.
        let full = w.styleMask().contains(NSWindowStyleMask::FullSizeContentView);
        let hidden = w.titleVisibility() == objc2_app_kit::NSWindowTitleVisibility::Hidden;
        check(
            "chrome-full-size",
            full && w.titlebarAppearsTransparent() && hidden && w.toolbarStyle() == objc2_app_kit::NSWindowToolbarStyle::Unified,
            format!("full-size content {full}, transparent bar {}, title hidden {hidden}, toolbar {:?}", w.titlebarAppearsTransparent(), w.toolbarStyle()),
        );
        let sand_bg = sand.backgroundColor().colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace()).map(|c| (c.redComponent(), c.greenComponent(), c.blueComponent()));
        check(
            "chrome-compact-background",
            sand.toolbarStyle() == objc2_app_kit::NSWindowToolbarStyle::UnifiedCompact
                && sand.titlebarAppearsTransparent()
                && !sand.styleMask().contains(NSWindowStyleMask::FullSizeContentView)
                && sand_bg.is_some_and(|c| close(c, (0xe9 as f64 / 255.0, 0xdc as f64 / 255.0, 0xc4 as f64 / 255.0), 0.01)),
            format!("toolbar {:?}, background {sand_bg:.3?}", sand.toolbarStyle()),
        );

        // Under the title bar: the sidebar's material reaches the window's
        // top; its first label and the content start below the bar.
        let content_h = w.contentView().unwrap().bounds().size.height;
        let bar = content_h - w.contentLayoutRect().size.height;
        let top_of = |v: &NSView| content_h - v.convertRect_toView(v.bounds(), None).origin.y - v.bounds().size.height;
        let materials: Vec<Retained<NSVisualEffectView>> = all(&w).into_iter().filter_map(|v| v.downcast::<NSVisualEffectView>().ok()).collect();
        let side = materials.iter().find(|m| m.material() == NSVisualEffectMaterial::Sidebar).cloned();
        let places = field(&w, "Places");
        // (The hero's title, not the sidebar row of the same name.)
        let hero_title = all(&w).into_iter().filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.stringValue().to_string() == "Lofoten" && f.font().is_some_and(|f| f.pointSize() > 20.0)).unwrap();
        check(
            "sidebar-under-title-bar",
            bar > 20.0
                && side.as_ref().is_some_and(|s| top_of(s).abs() < 0.5 && s.blendingMode() == NSVisualEffectBlendingMode::BehindWindow && s.frame().size.height >= content_h - 0.5)
                && top_of(&places) >= bar,
            format!(
                "bar {bar} pt; sidebar material top {:?}, height {:?} of {content_h}; 'Places' top {}",
                side.as_ref().map(|s| top_of(s)),
                side.as_ref().map(|s| s.frame().size.height),
                top_of(&places)
            ),
        );
        let surfaces: Vec<Retained<NSView>> = all(&w).into_iter().filter(|v| v.class().name().to_string_lossy() == "FerrokitSurface").collect();
        let picture = surfaces.iter().find(|s| s.frame().size.height > 200.0).cloned().unwrap();
        check("content-below-title-bar", top_of(&picture) >= bar + 24.0 - 0.5, format!("picture top {} (bar {bar} + 24 padding)", top_of(&picture)));

        // The glass caption: Liquid Glass on macOS 26 (a popover material
        // before), over the picture, at its bottom-left corner.
        let glass = all(&w).into_iter().find(|v| {
            objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some_and(|g| v.isKindOfClass(g)) || v.downcast_ref::<NSVisualEffectView>().is_some_and(|m| m.material() == NSVisualEffectMaterial::Popover)
        });
        let kind = glass.as_ref().map(|g| g.class().name().to_string_lossy().to_string());
        let want = if has_liquid_glass() { "NSGlassEffectView" } else { "FerrokitMaterial" };
        let is_want = glass.as_ref().is_some_and(|g| objc2::runtime::AnyClass::get(&std::ffi::CString::new(want).unwrap()).is_some_and(|c| g.isKindOfClass(c)));
        let above = glass.as_ref().is_some_and(|g| {
            // SAFETY: reading the view hierarchy on the main thread.
            let subs = unsafe { picture.superview() }.unwrap().subviews();
            let index = |v: &NSView| subs.iter().position(|s| std::ptr::eq(&*s, v));
            index(g).zip(index(&picture)).is_some_and(|(g, p)| g > p)
        });
        let (pf, gf) = (picture.frame(), glass.as_ref().map(|g| g.frame()));
        let corner = gf.is_some_and(|g| (g.origin.x - pf.origin.x - 16.0).abs() < 0.5 && (pf.origin.y + pf.size.height - (g.origin.y + g.size.height) - 16.0).abs() < 0.5);
        check(
            "glass-over-picture",
            is_want && above && corner,
            format!("{kind:?} (want {want}); above the picture {above}; 16 pt in from its bottom-left {corner} (picture {pf:?}, glass {gf:?})"),
        );
        let title_in_glass = gf.is_some_and(|g| {
            let t = hero_title.frame();
            t.origin.x >= g.origin.x && t.origin.y >= g.origin.y && t.origin.x + t.size.width <= g.origin.x + g.size.width
        });
        check("zstack-layers", title_in_glass && pf.size.width > 500.0 && (pf.size.height - 280.0).abs() < 0.5, format!("title inside the glass {title_in_glass}; picture {:?}", pf.size));
        let hud_material = all(&hud).into_iter().filter_map(|v| v.downcast::<NSVisualEffectView>().ok()).map(|m| m.material()).next();
        check("hud-material", hud_material == Some(NSVisualEffectMaterial::HUDWindow), format!("{hud_material:?}"));

        captures(w, sand, side.map(Retained::into_super), glass, picture, failed);
    }

    /// The capture checks, in light and then dark.
    fn captures(w: Retained<NSWindow>, sand: Retained<NSWindow>, side: Option<Retained<NSView>>, glass: Option<Retained<NSView>>, picture: Retained<NSView>, failed: std::rc::Rc<std::cell::Cell<u32>>) {
        let check = {
            let failed = failed.clone();
            move |name: &str, pass: bool, note: String| {
                eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
                failed.set(failed.get() + u32::from(!pass));
            }
        };
        let looks = std::rc::Rc::new(std::cell::RefCell::new(Vec::<(&str, Rgb, Rgb, Rgb, Rgb)>::new()));
        let shot = {
            let (w, sand, looks) = (w.clone(), sand.clone(), looks.clone());
            move |name: &'static str| {
                let Some(rep) = capture_window(&w) else {
                    eprintln!("[check] material-capture skipped: no screen-recording permission");
                    return;
                };
                if let Ok(snap) = std::env::var("SNAP") {
                    save(&rep, &format!("{snap}/materials-{name}.png"));
                    if let Some(r) = capture_window(&sand) {
                        save(&r, &format!("{snap}/materials-sand-{name}.png"));
                    }
                }
                let mid = |v: &NSView, fx: f64, fy: f64| {
                    let r = v.convertRect_toView(v.bounds(), None);
                    pixel(&rep, &w, r.origin.x + r.size.width * fx, r.origin.y + r.size.height * fy)
                };
                let Some((side, glass)) = side.as_ref().zip(glass.as_ref()) else { return };
                // A sidebar point clear of its rows; the content beside it.
                let sidebar = mid(side, 0.5, 0.2);
                let content = pixel(&rep, &w, side.convertRect_toView(side.bounds(), None).origin.x + side.frame().size.width + 40.0, 30.0);
                // The glass: near its right edge (clear of text) and the
                // picture just right of it at the same height.
                let g = glass.convertRect_toView(glass.bounds(), None);
                let in_glass = pixel(&rep, &w, g.origin.x + g.size.width - 6.0, g.origin.y + g.size.height / 2.0);
                let beside = pixel(&rep, &w, g.origin.x + g.size.width + 14.0, g.origin.y + g.size.height / 2.0);
                let _ = &picture;
                looks.borrow_mut().push((name, sidebar, content, in_glass, beside));
            }
        };
        set_appearance(Appearance::Light);
        let shot2 = shot.clone();
        let sand2 = sand.clone();
        later_ms(500, move || {
            shot2("light");
            set_appearance(Appearance::Dark);
            later_ms(500, move || {
                shot("dark");
                set_appearance(Appearance::System);
                let looks = looks.borrow();
                for (name, sidebar, content, in_glass, beside) in looks.iter() {
                    // The glass shows over the picture: lighter (or tinted)
                    // than the picture beside it.
                    let detail = format!("in the glass {in_glass:.3?}, the picture beside it {beside:.3?}");
                    if ferrokit::testing::virtual_machine() {
                        // No GPU in a virtual machine: materials don't blur.
                        eprintln!("[check] glass-visible-{name} SKIP a virtual machine ({detail})");
                    } else {
                        check(&format!("glass-visible-{name}"), (lum(*in_glass) - lum(*beside)).abs() > 0.04, detail);
                    }
                    let _ = (sidebar, content);
                }
                if looks.len() == 2 {
                    let (l, d) = (&looks[0], &looks[1]);
                    check("sidebar-follows-dark-mode", lum(l.1) > lum(d.1) + 0.3, format!("sidebar light {:.3?}, dark {:.3?}", l.1, d.1));
                    check("glass-follows-dark-mode", lum(l.3) > lum(d.3) + 0.1, format!("glass light {:.3?}, dark {:.3?}", l.3, d.3));
                }
                drop(looks);
                if let Some(rep) = capture_window(&sand2) {
                    let c = pixel(&rep, &sand2, sand2.frame().size.width - 30.0, 30.0);
                    check("window-background", close(c, (0xe9 as f64 / 255.0, 0xdc as f64 / 255.0, 0xc4 as f64 / 255.0), 0.03), format!("{c:.3?}"));
                }
                eprintln!("[check] done, {} failed", failed.get());
                std::process::exit(if failed.get() == 0 { 0 } else { 1 });
            });
        });
    }

    fn field(w: &NSWindow, text: &str) -> Retained<NSTextField> {
        all(w).into_iter().filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.stringValue().to_string() == text).unwrap()
    }

    fn later_ms(ms: u64, f: impl FnOnce() + 'static) {
        spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
    }

    /// The colour at window point (x, y), y from the bottom as AppKit's.
    fn pixel(rep: &NSBitmapImageRep, w: &NSWindow, x: f64, y: f64) -> Rgb {
        let scale = w.backingScaleFactor();
        let py = ((w.frame().size.height - y) * scale) as isize;
        let px = (x * scale) as isize;
        // The capture is in the display's colour space (Display P3), but
        // `colorAtX_y` tags its pixels as generic RGB: read the components
        // in the capture's own space.
        let Some(c) = rep.colorAtX_y(px, py) else { return (0.0, 0.0, 0.0) };
        let components = [c.redComponent(), c.greenComponent(), c.blueComponent(), 1.0];
        // SAFETY: four components for an RGB colour space with alpha.
        let c = unsafe { objc2_app_kit::NSColor::colorWithColorSpace_components_count(&rep.colorSpace(), std::ptr::NonNull::new(components.as_ptr() as *mut f64).unwrap(), 4) };
        c.colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace()).map_or((0.0, 0.0, 0.0), |c| (c.redComponent(), c.greenComponent(), c.blueComponent()))
    }

    fn close(a: Rgb, b: Rgb, tolerance: f64) -> bool {
        (a.0 - b.0).abs() < tolerance && (a.1 - b.1).abs() < tolerance && (a.2 - b.2).abs() < tolerance
    }

    fn lum(c: Rgb) -> f64 {
        0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2
    }

    fn save(rep: &NSBitmapImageRep, path: &str) {
        // SAFETY: encoding a bitmap as PNG with no options.
        if let Some(png) = unsafe { rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()) } {
            png.writeToFile_atomically(&objc2_foundation::NSString::from_str(path), true);
        }
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
}
