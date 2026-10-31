//! V1: a screen built only with Ferrokit's styling: text styles, cards
//! (background, corners, border, shadow), styled SF Symbols and pictures.
//! `cargo run -p ferrokit --example styles`; `CHECK=all` runs its
//! self-checks; `SNAP=<dir>` also saves light and dark window images.

use ferrokit::*;

/// A stat card: a coloured symbol, a caption and a big number.
fn stat(symbol: &str, tint: Color, caption: &str, value: Signal<f64>, format: fn(f64) -> String) -> impl View {
    vstack((
        hstack((
            image(symbol).symbol_size(15.0).symbol_weight(Weight::Semibold).hierarchical(tint).frame(20.0, 18.0),
            text(caption).font(Font::subheadline().weight(Weight::Medium)).color(Color::Secondary),
        ))
        .spacing(6.0),
        label(move || format(value.get())).font(Font::title().weight(Weight::Semibold).monospaced_digits()),
    ))
    .spacing(8.0)
    .alignment(Align::Start)
    .padding(16.0)
    .fill_width()
    .background(Color::ControlBackground)
    .corner_radius(12.0)
    .border(Color::Separator, 0.5)
    .shadow(Shadow::soft())
}

/// One row of the activity list.
fn row(symbol: &str, tint: Color, title: &str, detail: &str, amount: &str, positive: bool) -> impl View {
    hstack((
        image(symbol).symbol_size(14.0).color(Color::rgb(1.0, 1.0, 1.0)).frame(18.0, 18.0).padding(7.0).background(tint).corner_radius(8.0),
        vstack((text(title).font(Font::body().weight(Weight::Medium)), text(detail).font(Font::caption()).color(Color::Secondary)))
            .spacing(2.0)
            .alignment(Align::Start),
        spacer(),
        text(amount).font(Font::body().weight(Weight::Semibold).monospaced_digits()).color(if positive { Color::Green } else { Color::Primary }),
    ))
    .spacing(12.0)
    .padding(10.0)
}

fn money(v: f64) -> String {
    format!("${}", format_number(v, 0))
}

fn screen() -> impl View {
    let (balance, income, spent) = (Signal::new(48_210.0), Signal::new(12_480.0), Signal::new(7_935.0));
    vstack((
        vstack((
            text("Good morning").font(Font::subheadline().weight(Weight::Medium)).color(Color::Secondary),
            text("Your money this month").font(Font::large_title().weight(Weight::Bold)),
        ))
        .spacing(2.0)
        .alignment(Align::Start),
        hstack((
            stat("banknote.fill", Color::Green, "Balance", balance, money),
            stat("arrow.down.circle.fill", Color::Blue, "Income", income, money),
            stat("cart.fill", Color::Orange, "Spent", spent, money),
        ))
        .spacing(14.0),
        text("Recent activity").font(Font::title3().weight(Weight::Semibold)),
        vstack((
            row("cup.and.saucer.fill", Color::Brown, "Blue Bottle Coffee", "Today, 8:42", "−$6.50", false),
            divider(),
            row("bag.fill", Color::Pink, "Market Hall", "Yesterday", "−$84.20", false),
            divider(),
            row("building.columns.fill", Color::Indigo, "Salary", "Oct 28", "+$6,240.00", true),
            divider(),
            row("tram.fill", Color::Teal, "Transit pass", "Oct 27", "−$98.00", false),
        ))
        .spacing(0.0)
        .padding(6.0)
        .background(Color::ControlBackground)
        .corner_radius(12.0)
        .border(Color::Separator, 0.5)
        .shadow(Shadow::soft()),
        spacer(),
    ))
    .spacing(18.0)
    .alignment(Align::Start)
    .padding(28.0)
}

/// A picture for the checks: left half red, right half blue (`w`×`h`
/// pixels, PNG).
fn two_colour_png(w: isize, h: isize) -> Vec<u8> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    // SAFETY: a plain RGBA bitmap, written byte by byte within its rows.
    unsafe {
        let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            objc2::AnyThread::alloc(),
            std::ptr::null_mut(),
            w,
            h,
            8,
            4,
            true,
            false,
            objc2_app_kit::NSDeviceRGBColorSpace,
            w * 4,
            32,
        )
        .unwrap();
        let data = rep.bitmapData();
        let row = rep.bytesPerRow() as usize;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let p = data.add(y * row + x * 4);
                let (r, b) = if (x as isize) < w / 2 { (255, 0) } else { (0, 255) };
                *p = r;
                *p.add(1) = 0;
                *p.add(2) = b;
                *p.add(3) = 255;
            }
        }
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()).unwrap().to_vec()
    }
}

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    let picture = two_colour_png(200, 100);
    App::new("Styles")
        .window(Window::new("Styles").size(720.0, 620.0).view(screen()))
        .window(
            Window::new("Pictures").size(260.0, 160.0).view(
                hstack((
                    // 200×100 in a 80×80 frame: filled (cropped to the middle),
                    // and fitted (all of it, letterboxed).
                    image_data(&picture).fill().corner_radius(40.0).description("Filled").frame(80.0, 80.0),
                    image_data(&picture).description("Fitted").frame(80.0, 80.0),
                    text("Faded").opacity(0.4),
                ))
                .spacing(16.0)
                .padding(20.0),
            ),
        )
        .on_launch(move || {
            if checks {
                // Once the windows are on screen, for the captures.
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(600)), |()| self_checks::run());
            }
        })
        .run();
}

mod self_checks {
    use ferrokit::testing::{ax_audit, capture_window};
    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSBitmapImageRep, NSColor, NSImageView, NSTextField, NSView, NSWindow};
    use objc2_foundation::NSObjectProtocol;

    type Rgb = (f64, f64, f64);

    pub fn run() {
        let all_windows = windows();
        let w = all_windows.iter().find(|w| w.title().to_string() == "Styles").unwrap().clone();
        let pics = all_windows.iter().find(|w| w.title().to_string() == "Pictures").unwrap().clone();
        let failed = std::cell::Cell::new(0);
        let check = |name: &str, pass: bool, note: String| {
            eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
            failed.set(failed.get() + u32::from(!pass));
        };
        let problems = [ax_audit(&w), ax_audit(&pics)].concat();
        check("accessibility", problems.is_empty(), format!("{problems:?}"));

        // Text styles: fonts and colours read back from the labels.
        let field = |text: &str| all(&w).into_iter().filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.stringValue().to_string() == text).unwrap();
        let bold = |f: &NSTextField| f.font().is_some_and(|f| f.fontDescriptor().symbolicTraits().contains(objc2_app_kit::NSFontDescriptorSymbolicTraits::TraitBold));
        let title = field("Your money this month");
        let size = |f: &NSTextField| f.font().map_or(0.0, |f| f.pointSize());
        check("text-large-title", size(&title) == 26.0 && bold(&title), format!("large title, bold: {} pt, bold {}", size(&title), bold(&title)));
        let caption = field("Today, 8:42");
        let secondary = caption.textColor().is_some_and(|c| c.isEqual(Some(&NSColor::secondaryLabelColor())));
        check("text-caption-secondary", size(&caption) == 10.0 && secondary, format!("caption {} pt; secondary colour {secondary}", size(&caption)));
        let amount = field("$48,210");
        let digits = amount.font().map(|f| {
            let width = |s: &str| unsafe {
                let attrs = objc2_foundation::NSDictionary::from_retained_objects(&[objc2_app_kit::NSFontAttributeName], &[Retained::into_super(Retained::into_super(f.clone()))]);
                objc2_app_kit::NSStringDrawing::sizeWithAttributes(&*objc2_foundation::NSString::from_str(s), Some(&attrs)).width
            };
            (width("1111"), width("8888"))
        });
        check("text-monospaced-digits", digits.is_some_and(|(a, b)| (a - b).abs() < 0.01), format!("width of 1111 and 8888: {digits:?}"));

        // Cards: the three stat cards come out the same width.
        let surfaces: Vec<Retained<NSView>> = all(&w).into_iter().filter(|v| v.class().name().to_string_lossy() == "FerrokitSurface").collect();
        let cards: Vec<_> = surfaces.iter().filter(|s| s.frame().size.height > 60.0 && s.frame().size.height < 120.0).map(|s| s.frame()).collect();
        let widths: Vec<f64> = cards.iter().map(|f| f.size.width).collect();
        check("cards-equal-width", widths.len() == 3 && widths.iter().all(|x| (x - widths[0]).abs() < 1.0), format!("stat card widths {widths:?}"));
        // Modifier order: `.frame(18, 18).padding(7)`: an 18-point icon in a
        // 32-point tile.
        let tiles: Vec<_> = surfaces.iter().filter(|s| s.frame().size.height < 40.0).map(|s| s.frame().size).collect();
        let icons: Vec<_> = all(&w).into_iter().filter_map(|v| v.downcast::<NSImageView>().ok()).map(|v| v.frame().size).filter(|s| s.width == 18.0).collect();
        check(
            "modifier-order",
            tiles.len() == 4 && tiles.iter().all(|t| t.width == 32.0 && t.height == 32.0) && icons.len() == 4,
            format!("tiles {:?}; 18-point icons {}", tiles.iter().map(|t| (t.width, t.height)).collect::<Vec<_>>(), icons.len()),
        );
        // Symbols: configured and tinted.
        let symbols: Vec<_> = all(&w).into_iter().filter_map(|v| v.downcast::<NSImageView>().ok()).collect();
        let configured = symbols.iter().filter(|v| v.symbolConfiguration().is_some()).count();
        let tinted = symbols.iter().filter(|v| v.contentTintColor().is_some()).count();
        check("symbols-styled", configured == 7 && tinted == 4, format!("{configured} of {} with a symbol configuration (7); {tinted} tinted (4)", symbols.len()));

        captures(w, pics, surfaces, failed.get());
    }

    /// The capture checks: each a run-loop pass after the appearance
    /// changes, so the window has drawn in it.
    fn captures(w: Retained<NSWindow>, pics: Retained<NSWindow>, surfaces: Vec<Retained<NSView>>, failed: u32) {
        let failed = std::rc::Rc::new(std::cell::Cell::new(failed));
        let check = {
            let failed = failed.clone();
            move |name: &str, pass: bool, note: String| {
                eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
                failed.set(failed.get() + u32::from(!pass));
            }
        };
        let looks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let card = surfaces.into_iter().find(|s| s.frame().size.height > 60.0 && s.frame().size.height < 120.0).unwrap();
        let shot = {
            let (w, looks, card) = (w.clone(), looks.clone(), card.clone());
            move |name: &'static str| {
                let Some(rep) = capture_window(&w) else {
                    eprintln!("[check] card-capture skipped: no screen-recording permission");
                    return;
                };
                if let Ok(snap) = std::env::var("SNAP") {
                    save(&rep, &format!("{snap}/styles-{name}.png"));
                }
                let r = card.convertRect_toView(card.bounds(), None);
                let (x0, top, x1, bottom) = (r.origin.x, r.origin.y + r.size.height, r.origin.x + r.size.width, r.origin.y);
                let inside = pixel(&rep, &w, (x0 + x1) / 2.0, bottom + 6.0);
                let corner = pixel(&rep, &w, x0 + 1.0, top - 1.0);
                // Window background: 14 points below the card, past its
                // shadow (the gap before the next heading).
                let window = pixel(&rep, &w, (x0 + x1) / 2.0, bottom - 14.0);
                let shadow = pixel(&rep, &w, (x0 + x1) / 2.0, bottom - 2.0);
                let want = objc2_app_kit::NSAppearanceCustomization::effectiveAppearance(&*card).name().to_string();
                looks.borrow_mut().push((name, inside, corner, window, shadow, want));
            }
        };
        set_appearance(Appearance::Light);
        let (shot2, w2) = (shot.clone(), w.clone());
        later_ms(400, move || {
            shot2("light");
            set_appearance(Appearance::Dark);
            later_ms(400, move || {
                shot("dark");
                let _ = &w2;
                set_appearance(Appearance::System);
                let looks = looks.borrow();
                for (name, inside, corner, window, shadow, appearance) in looks.iter() {
                    // The card is the system's control background (white in
                    // light, dark grey in dark); its corner is cut away (the
                    // window shows); a shadow falls below it.
                    let fill = if *name == "light" { close(*inside, (1.0, 1.0, 1.0), 0.02) } else { lum(*inside) < 0.3 };
                    check(
                        &format!("card-{name}"),
                        fill && close(*corner, *window, 0.04) && lum(*shadow) < lum(*window) - 0.004,
                        format!("({appearance}) inside {inside:.3?}: control background {fill}; corner {corner:.3?} like the window {window:.3?}; below it {shadow:.3?} darker"),
                    );
                }
                if looks.len() == 2 {
                    check("card-follows-dark-mode", lum(looks[1].1) < lum(looks[0].1) - 0.3, format!("card light {:.3?}, dark {:.3?}", looks[0].1, looks[1].1));
                }
                drop(looks);
                later_ms(400, move || {
                    pictures(&pics, &check);
                    let faded = all(&pics).into_iter().filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.stringValue().to_string() == "Faded").map(|f| f.alphaValue());
                    check("opacity", faded == Some(0.4), format!("{faded:?}"));
                    eprintln!("[check] done, {} failed", failed.get());
                    std::process::exit(if failed.get() == 0 { 0 } else { 1 });
                });
            });
        });
    }

    /// Pictures: filled (cropped to the middle, rounded) and fitted.
    fn pictures(pics: &NSWindow, check: &dyn Fn(&str, bool, String)) {
        let Some(rep) = capture_window(pics) else { return };
        let views: Vec<Retained<NSView>> = all(pics).into_iter().filter(|v| v.class().name().to_string_lossy() == "FerrokitSurface").collect();
        let at = |v: &NSView, fx: f64, fy: f64| {
            let r = v.convertRect_toView(v.bounds(), None);
            pixel(&rep, pics, r.origin.x + r.size.width * fx, r.origin.y + r.size.height * (1.0 - fy))
        };
        let (filled, fitted) = (&views[0], &views[1]);
        // (The bitmap is device RGB: the capture shows it colour-managed.)
        let red = |c: Rgb| c.0 > 0.8 && c.2 < 0.3;
        let blue = |c: Rgb| c.2 > 0.8 && c.0 < 0.2;
        // Filled: 200×100 scaled to 160×80, cropped to the middle 80: red
        // left of centre, blue right; the corner cut away.
        let fill_ok = red(at(filled, 0.3, 0.5)) && blue(at(filled, 0.7, 0.5)) && !red(at(filled, 0.03, 0.03));
        // Fitted: 80 wide by 40 high in the middle, nothing above it.
        let fit_ok = red(at(fitted, 0.25, 0.5)) && blue(at(fitted, 0.75, 0.5)) && !red(at(fitted, 0.5, 0.1)) && !blue(at(fitted, 0.5, 0.1));
        check(
            "pictures",
            fill_ok && fit_ok,
            format!("filled: {:.2?} | {:.2?}, corner {:.2?}; fitted: {:.2?} | {:.2?}, above {:.2?}", at(filled, 0.3, 0.5), at(filled, 0.7, 0.5), at(filled, 0.03, 0.03), at(fitted, 0.25, 0.5), at(fitted, 0.75, 0.5), at(fitted, 0.5, 0.1)),
        );
    }

    fn later_ms(ms: u64, f: impl FnOnce() + 'static) {
        spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
    }

    /// The colour at window point (x, y), y from the bottom as AppKit's.
    fn pixel(rep: &NSBitmapImageRep, w: &NSWindow, x: f64, y: f64) -> Rgb {
        let scale = w.backingScaleFactor();
        let py = ((w.frame().size.height - y) * scale) as isize;
        let px = (x * scale) as isize;
        rep.colorAtX_y(px, py)
            .and_then(|c| c.colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace()))
            .map_or((0.0, 0.0, 0.0), |c| (c.redComponent(), c.greenComponent(), c.blueComponent()))
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
