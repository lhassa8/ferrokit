//! Helpers for automated checks of apps built with ferrokit: the
//! accessibility tree as VoiceOver sees it, key equivalents through the menu,
//! simulated edits, memory footprint and window snapshots. Used by the
//! checklist and the example apps' self-checks.

use ferrokit_core::kept::{UiCell, UiLocal};
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSTextField};
use objc2_foundation::NSString;

use crate::mtm;

#[path = "input.rs"]
pub mod input;

#[path = "soak.rs"]
pub mod soak;

thread_local! {
    /// Checks failed so far (see [`check`]).
    static FAILED_CHECKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Reports one self-check as `cargo ferrokit check` reads them: prints
/// `[check] <name> PASS <note>` or `FAIL`, and counts the failures for
/// [`finish`]. Put what was seen in `note`, so a failure says why.
pub fn check(name: &str, pass: bool, note: impl std::fmt::Display) {
    eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
    if !pass {
        FAILED_CHECKS.set(FAILED_CHECKS.get() + 1);
    }
}

/// Reports a self-check that couldn't run here (the screen is locked, a
/// permission isn't granted): `[check] <name> SKIP <why>`. Not a failure;
/// `cargo ferrokit check` counts it apart.
pub fn skip(name: &str, why: impl std::fmt::Display) {
    eprintln!("[check] {name} SKIP {why}");
}

/// Ends a self-check run: prints `[check] done, <n> failed` and exits, with
/// a failing status if any [`check`] failed.
pub fn finish() -> ! {
    let failed = FAILED_CHECKS.get();
    eprintln!("[check] done, {failed} failed");
    std::process::exit(if failed == 0 { 0 } else { 1 });
}

/// Runs `steps` one after another, each in a turn of its own `pause_ms`
/// after the one before, then [`finish`]es. A self-check acts in one step
/// and looks in the next: effects, table updates and sheets settle after
/// the step that caused them returns. A step that starts slower work (a
/// save on another thread, a network call) calls [`wait_for`], and the next
/// step waits until that work is done instead of a fixed time.
///
/// ```no_run
/// use ferrokit::testing::{check, steps};
/// use ferrokit::*;
///
/// let count = Signal::new(0);
/// steps(100, vec![
///     Box::new(move || count.set(1)),
///     Box::new(move || check("counted", count.get_untracked() == 1, count.get_untracked())),
/// ]);
/// ```
pub fn steps(pause_ms: u64, steps: Vec<Box<dyn FnOnce()>>) {
    fn next(pause_ms: u64, mut rest: std::collections::VecDeque<Box<dyn FnOnce()>>) {
        crate::spawn(
            move || std::thread::sleep(std::time::Duration::from_millis(pause_ms)),
            move |()| {
                if still_waiting() {
                    // Look again shortly; the pause is already behind us.
                    return next(WAIT_POLL_MS, rest);
                }
                match rest.pop_front() {
                    Some(step) => {
                        step();
                        next(pause_ms, rest);
                    }
                    None => finish(),
                }
            },
        );
    }
    next(pause_ms, steps.into());
}

const WAIT_POLL_MS: u64 = 20;

/// A condition the next of [`steps`] waits for, and until when.
type Wait = (Box<dyn Fn() -> bool>, std::time::Instant, u64);

thread_local! {
    static WAIT: UiCell<Option<Wait>> = UiCell::new(None);
}

/// Whether [`steps`] should hold the next step: a [`wait_for`] condition is
/// still false and its time isn't up. Clears the wait once either is true.
fn still_waiting() -> bool {
    let Some((condition, deadline, ms)) = WAIT.with(|w| w.borrow_mut().take()) else { return false };
    if condition() {
        return false;
    }
    if std::time::Instant::now() >= deadline {
        eprintln!("[check] (waited {ms} ms for a condition that stayed false; going on)");
        return false;
    }
    WAIT.with(|w| *w.borrow_mut() = Some((condition, deadline, ms)));
    true
}

/// Called in one of [`steps`]: holds the next step until `condition` is
/// true, looking every 20 ms, for up to `timeout_ms` (then the next step
/// runs anyway, and its check reports what it sees). Use it instead of a
/// long pause for work that finishes in its own time: a save on another
/// thread, a task's network call, a file another process writes.
///
/// ```no_run
/// use ferrokit::testing::{check, steps, wait_for};
/// use ferrokit::*;
///
/// let saved = Signal::new(false);
/// steps(100, vec![
///     Box::new(move || {
///         spawn(|| std::thread::sleep(std::time::Duration::from_millis(300)), move |()| saved.set(true));
///         wait_for(5_000, move || saved.get_untracked());
///     }),
///     Box::new(move || check("saved", saved.get_untracked(), "")),
/// ]);
/// ```
pub fn wait_for(timeout_ms: u64, condition: impl Fn() -> bool + 'static) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    WAIT.with(|w| *w.borrow_mut() = Some((Box::new(condition), deadline, timeout_ms)));
}

/// Outside [`steps`]: calls `then(true)` as soon as `condition` is true
/// (looking every 20 ms, between events), or `then(false)` after
/// `timeout_ms`.
pub fn wait_until(timeout_ms: u64, condition: impl Fn() -> bool + 'static, then: impl FnOnce(bool) + 'static) {
    fn poll(deadline: std::time::Instant, condition: Box<dyn Fn() -> bool>, then: Box<dyn FnOnce(bool)>) {
        if condition() {
            return then(true);
        }
        if std::time::Instant::now() >= deadline {
            return then(false);
        }
        crate::spawn(move || std::thread::sleep(std::time::Duration::from_millis(WAIT_POLL_MS)), move |()| poll(deadline, condition, then));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    poll(deadline, Box::new(condition), Box::new(then));
}

/// Presses the button titled `title` in the sheet on `window`: answers an
/// [`alert`](crate::alert) ("Delete", "Cancel") or a [`sheet`](crate::sheet)'s
/// own buttons, as a click would. False if `window` shows no sheet or the
/// sheet has no such button. The alert's callback runs on a later pass of
/// the run loop, so look in the next step.
pub fn press_sheet_button(window: &crate::WindowRef, title: &str) -> bool {
    let window = window.ns();
    fn find(view: &objc2_app_kit::NSView, title: &str) -> Option<Retained<objc2_app_kit::NSButton>> {
        if let Some(b) = view.downcast_ref::<objc2_app_kit::NSButton>()
            && b.title().to_string() == title
            && b.isEnabled()
        {
            return Some(objc2::Message::retain(b));
        }
        view.subviews().iter().find_map(|v| find(&v, title))
    }
    let Some(sheet) = window.attachedSheet() else { return false };
    let Some(button) = sheet.contentView().and_then(|v| find(&v, title)) else { return false };
    // SAFETY: on the main thread; performClick sends the button's action.
    unsafe { button.performClick(None) };
    true
}

/// The first view in `window` with the accessibility label `label` (what
/// `.accessibility_label(..)` set), for a check to read or drive: a field,
/// a table, an editor's text view, a web view.
pub fn view_labeled(window: &crate::WindowRef, label: &str) -> Option<Retained<objc2_app_kit::NSView>> {
    let window = window.ns();
    fn find(view: &objc2_app_kit::NSView, label: &str) -> Option<Retained<objc2_app_kit::NSView>> {
        // SAFETY: NSView answers accessibilityLabel (NSAccessibility).
        let own: Option<Retained<NSString>> = unsafe { objc2::msg_send![view, accessibilityLabel] };
        if own.is_some_and(|l| l.to_string() == label) {
            return Some(objc2::Message::retain(view));
        }
        view.subviews().iter().find_map(|v| find(&v, label))
    }
    find(&*window.contentView()?, label)
}

/// The process's physical footprint now, in MB: what Activity Monitor's
/// Memory column shows. Read in-process (`proc_pid_rusage`), so it works in
/// a sandboxed app too (it ran `vmmap`, which the App Sandbox refuses, and
/// gave 0 there).
pub fn footprint_mb() -> f64 {
    rusage().map_or(0.0, |r| r.ri_phys_footprint as f64 / (1 << 20) as f64)
}

/// The process's peak physical footprint so far, in MB (see [`footprint_mb`]).
pub fn peak_footprint_mb() -> f64 {
    rusage().map_or(0.0, |r| r.ri_lifetime_max_phys_footprint as f64 / (1 << 20) as f64)
}

fn rusage() -> Option<libc::rusage_info_v4> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: the kernel fills a v4 record of our own process into `info`.
    let ok = unsafe { libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V4, info.as_mut_ptr().cast()) } == 0;
    // SAFETY: filled by the call above when it succeeded (and zeroed before).
    ok.then(|| unsafe { info.assume_init() })
}

/// The window's accessibility tree, flattened: (role, subrole, label/title)
/// for every element under the content view, as VoiceOver sees it. On
/// macOS 27, AppKit's text fields and checkboxes host SwiftUI internally, so
/// the AX element is a child of the NSView, not the view itself.
pub fn ax_tree(window: &crate::WindowRef) -> Vec<(String, String, String)> {
    let window = window.ns();
    // Not every element in the tree implements every accessor; sending an
    // unknown selector raises an exception that would unwind out of the
    // caller. Ask first.
    fn responds(obj: &objc2::runtime::AnyObject, sel: objc2::runtime::Sel) -> bool {
        obj.class().responds_to(sel)
    }
    fn s(obj: &objc2::runtime::AnyObject, sel: &str) -> String {
        let selector = match sel {
            "role" => objc2::sel!(accessibilityRole),
            "subrole" => objc2::sel!(accessibilitySubrole),
            "label" => objc2::sel!(accessibilityLabel),
            _ => objc2::sel!(accessibilityTitle),
        };
        if !responds(obj, selector) {
            return String::new();
        }
        let value: Option<Retained<objc2::runtime::AnyObject>> = unsafe {
            match sel {
                "role" => objc2::msg_send![obj, accessibilityRole],
                "subrole" => objc2::msg_send![obj, accessibilitySubrole],
                "label" => objc2::msg_send![obj, accessibilityLabel],
                _ => objc2::msg_send![obj, accessibilityTitle],
            }
        };
        value.and_then(|v| v.downcast::<NSString>().ok()).map(|s| s.to_string()).unwrap_or_default()
    }
    fn walk(obj: &objc2::runtime::AnyObject, out: &mut Vec<(String, String, String)>, depth: usize) {
        let label = { let l = s(obj, "label"); if l.is_empty() { s(obj, "title") } else { l } };
        out.push((s(obj, "role"), s(obj, "subrole"), label));
        if depth > 12 {
            return;
        }
        if !responds(obj, objc2::sel!(accessibilityChildren)) {
            return;
        }
        let kids: Option<Retained<objc2_foundation::NSArray<objc2::runtime::AnyObject>>> =
            unsafe { objc2::msg_send![obj, accessibilityChildren] };
        if let Some(kids) = kids {
            for k in kids.iter() {
                walk(&k, out, depth + 1);
            }
        }
    }
    let mut out = Vec::new();
    walk(&window.contentView().unwrap(), &mut out, 0);
    out
}
/// Whether the tree has an element with this role (and subrole and label,
/// unless empty).
pub fn has_ax(tree: &[(String, String, String)], role: &str, subrole: &str, label: &str) -> bool {
    tree.iter().any(|(r, s, l)| r == role && (subrole.is_empty() || s == subrole) && (label.is_empty() || l == label))
}
/// Sets a text field's text and posts the change notification, as typing does.
pub fn post_edit(field: &NSTextField, text: &str) {
    field.setStringValue(&NSString::from_str(text));
    unsafe {
        objc2_foundation::NSNotificationCenter::defaultCenter()
            .postNotificationName_object(objc2_app_kit::NSControlTextDidChangeNotification, Some(field))
    };
}
/// Sends a control's action to its target, as a click or Return does.
pub fn send_action(control: &objc2_app_kit::NSControl) -> bool {
    unsafe { control.sendAction_to(control.action(), control.target().as_deref()) }
}

/// Lays out right to left (`Some(true)`), left to right, or as the app's
/// language says (`None`): for checking a layout mirrors. Windows lay out
/// again as they change; call it before opening the window to check.
pub fn set_right_to_left(on: Option<bool>) {
    crate::force_right_to_left(on);
}

/// The failures caught where AppKit called in (exceptions and panics; the
/// last 100): see [`App::on_error`](crate::App::on_error).
pub fn failures() -> Vec<crate::Failure> {
    crate::failure::caught()
}

/// Quick Look thumbnails made so far ([`thumbnail`](crate::thumbnail)).
pub fn thumbnails_made() -> usize {
    crate::quicklook::THUMBNAILS.with(std::cell::Cell::get)
}

/// The files the Quick Look panel was given, and the one it shows (`None`
/// while it's closed).
pub fn quick_look_state() -> (Vec<std::path::PathBuf>, Option<usize>) {
    crate::quicklook::quick_look_state()
}

/// Renders every window to `<dir>/<title>.png` (AppKit's own drawing, no
/// screen-recording permission needed). The window's frame view is drawn, so
/// the image includes the title bar and the window background (which the
/// content view alone doesn't draw).
pub fn snapshot_windows(dir: &str) {
    for w in crate::windows_ns() {
        let content = w.contentView().unwrap();
        // SAFETY: reading the view hierarchy on the main thread.
        let view = unsafe { content.superview() }.unwrap_or(content);
        let bounds = view.bounds();
        let Some(rep) = view.bitmapImageRepForCachingDisplayInRect(bounds) else { continue };
        view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
        let png = unsafe {
            rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new())
        };
        if let Some(png) = png {
            let path = format!("{dir}/{}.png", w.title());
            png.writeToFile_atomically(&NSString::from_str(&path), true);
        }
    }
}

/// How many times About {app} was shown with [`App::about`](crate::App::about)'s credits.
pub fn about_shown() -> u32 {
    crate::ABOUT_SHOWN.with(|a| a.get())
}

/// Sends a key-down with `chars` and `modifiers` (`&[Modifier::Command]`) to
/// the main menu's key equivalents (no window needed); true if an enabled
/// item took it.
pub fn menu_key(chars: &str, modifiers: &[crate::Modifier]) -> bool {
    let modifiers = input::flags(modifiers);
    use objc2_app_kit::{NSEvent, NSEventType};
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown,
        objc2_foundation::NSPoint::new(0.0, 0.0),
        modifiers,
        0.0,
        0,
        None,
        &NSString::from_str(chars),
        &NSString::from_str(chars),
        false,
        0,
    )
    .unwrap();
    NSApplication::sharedApplication(mtm()).mainMenu().unwrap().performKeyEquivalent(&event)
}

/// Performs a keyboard shortcut the way the menu bar does, without needing
/// the app to be active: finds the main-menu item for ⌘`chars` (⇧⌘ when
/// `shift`; AppKit's rule: an uppercase key equivalent implies Shift).
/// An item of the app's own ([`menu_item`](crate::menu_item),
/// [`menu_command`](crate::menu_command)) runs if it's enabled; a standard
/// item (Undo, Select All) sends its action along `window`'s responder
/// chain, starting at its first responder. Returns whether an item matched
/// and something handled it.
///
/// (A synthesized key event through `NSMenu.performKeyEquivalent` matches
/// the item but goes nowhere when the app isn't active: there's no key
/// window. A person pressing keys always has the app active.)
pub fn shortcut(window: &crate::WindowRef, chars: &str, shift: bool) -> bool {
    let window = window.ns();
    use objc2_app_kit::{NSEventModifierFlags, NSMenu};
    fn find(menu: &NSMenu, key: &str, shift: bool) -> Option<(Retained<NSMenu>, isize)> {
        for i in 0..menu.numberOfItems() {
            let item = menu.itemAtIndex(i)?;
            if let Some(sub) = item.submenu() {
                if let Some(found) = find(&sub, key, shift) {
                    return Some(found);
                }
                continue;
            }
            let k = item.keyEquivalent().to_string();
            let mask = item.keyEquivalentModifierMask();
            let wants_shift = mask.contains(NSEventModifierFlags::Shift) || (k.chars().all(char::is_uppercase) && !k.is_empty());
            let others = mask.intersects(NSEventModifierFlags::Option | NSEventModifierFlags::Control);
            if k.to_lowercase() == key && mask.contains(NSEventModifierFlags::Command) && wants_shift == shift && !others {
                return Some((objc2::Message::retain(menu), i));
            }
        }
        None
    }
    let app = NSApplication::sharedApplication(mtm());
    let Some((menu, i)) = app.mainMenu().and_then(|m| find(&m, &chars.to_lowercase(), shift)) else { return false };
    let Some(item) = menu.itemAtIndex(i) else { return false };
    if item.target().is_some() {
        // The app's own item: enabled as the menu shows it, then run.
        menu.update();
        if !item.isEnabled() {
            return false;
        }
        menu.performActionForItemAtIndex(i);
        return true;
    }
    let Some(action) = item.action() else { return false };
    // SAFETY: standard responder-chain dispatch, the item as the sender (as
    // the menu sends it: Find's actions read their item's tag).
    window.firstResponder().is_some_and(|r| unsafe { r.tryToPerform_with(action, Some(&item)) })
}

/// From now on, [`reveal_in_finder`](crate::reveal_in_finder) records what
/// it's asked to show instead of opening Finder windows (`false` stops).
pub fn capture_reveals(on: bool) {
    crate::services::capture_reveals(on);
}

/// What [`reveal_in_finder`](crate::reveal_in_finder) was asked to show
/// since the last call, while [`capture_reveals`] is on: one list of paths
/// per call.
pub fn revealed() -> Vec<Vec<std::path::PathBuf>> {
    crate::services::take_reveals()
}

/// Sets `path`'s last-opened date (seconds since 1970, `None` for never),
/// for checks of lists that use [`file_dates`](crate::file_dates).
pub fn set_last_opened(path: &std::path::Path, secs: Option<i64>) -> std::io::Result<()> {
    crate::services::set_last_opened(path, secs)
}

/// Runs `f` the way Ferrokit runs code AppKit calls in (an action, a
/// timer): a panic in it is caught and reported (see
/// [`failures`]), not a crash.
pub fn catching(f: impl FnOnce()) {
    crate::failure::entry("a check", (), f);
}

/// The crash reports in `dir` not yet marked sent, oldest first (what
/// [`App::crash_reports`](crate::App::crash_reports) hands the app at
/// launch). Checks point `FERROKIT_CRASH_DIR` at a folder of their own.
pub fn pending_crash_reports(dir: &std::path::Path) -> Vec<crate::CrashReport> {
    crate::crash::pending(dir)
}

/// Whether the system spell checker flags `word` in English. (With automatic
/// language detection one isolated word can be guessed as another language
/// and pass, so the language is given.)
pub fn is_misspelled(word: &str) -> bool {
    let checker = objc2_app_kit::NSSpellChecker::sharedSpellChecker();
    let mut count: isize = 0;
    // SAFETY: documented NSSpellChecker method; wordCount out-pointer is valid.
    let range: objc2_foundation::NSRange = unsafe {
        objc2::msg_send![&*checker, checkSpellingOfString: &*NSString::from_str(word), startingAt: 0isize,
            language: &*NSString::from_str("en"), wrap: false, inSpellDocumentWithTag: 0isize, wordCount: &mut count]
    };
    range.length > 0
}

/// The titles of a text view's context menu for a right-click on `word`.
pub fn context_menu_for_word(view: &objc2_app_kit::NSTextView, word: &str) -> Vec<String> {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{NSPoint, NSRange, NSRect};
    let Some(window) = view.window() else { return Vec::new() };
    let text = view.string().to_string();
    let Some(byte_index) = text.find(word) else { return Vec::new() };
    let location = text[..byte_index].encode_utf16().count();
    // SAFETY: NSTextInputClient geometry query.
    let screen: NSRect = unsafe {
        objc2::msg_send![view, firstRectForCharacterRange: NSRange::new(location + word.len() / 2, 1), actualRange: std::ptr::null_mut::<NSRange>()]
    };
    let w = window.convertRectFromScreen(screen);
    let point = NSPoint::new(w.origin.x + w.size.width / 2.0, w.origin.y + w.size.height / 2.0);
    let Some(event) = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::RightMouseDown, point, NSEventModifierFlags::empty(), 0.0, window.windowNumber(), None, 0, 1, 1.0,
    ) else {
        return Vec::new();
    };
    let Some(menu) = view.menuForEvent(&event) else { return Vec::new() };
    (0..menu.numberOfItems()).filter_map(|i| menu.itemAtIndex(i)).map(|item| item.title().to_string()).collect()
}

/// Captures `window` as the window server shows it (lossless, no shadow, via
/// `screencapture -l`): what's on screen, which in-process snapshots can
/// miss (Core Animation layers, animations, vibrancy). Needs the
/// screen-recording permission; `None` without it.
pub fn capture_window(window: &crate::WindowRef) -> Option<Retained<objc2_app_kit::NSBitmapImageRep>> {
    let window = window.ns();
    let path = std::env::temp_dir().join(format!("ferrokit-capture-{}-{}.png", std::process::id(), window.windowNumber()));
    let ok = std::process::Command::new("screencapture")
        .args(["-x", "-o", "-l", &window.windowNumber().to_string()])
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success());
    let bytes = if ok { std::fs::read(&path).ok() } else { None };
    let rep = bytes.and_then(|b| objc2_app_kit::NSBitmapImageRep::imageRepWithData(&objc2_foundation::NSData::with_bytes(&b)));
    let _ = std::fs::remove_file(&path);
    // Without the permission screencapture writes the desktop, nothing, or
    // a window-sized image without the window's content.
    rep.filter(|r| (r.pixelsWide() as f64 - window.frame().size.width * window.backingScaleFactor()).abs() < 2.0)
        .filter(|r| has_content(r))
}

/// Whether a capture shows anything: a window with views has some
/// variation in its colours; a blank stand-in is one flat colour.
fn has_content(rep: &objc2_app_kit::NSBitmapImageRep) -> bool {
    let srgb = objc2_app_kit::NSColorSpace::sRGBColorSpace();
    let (w, h) = (rep.pixelsWide(), rep.pixelsHigh());
    let mut seen: Vec<(f64, f64, f64)> = Vec::new();
    for y in (0..h).step_by((h / 40).max(1) as usize) {
        for x in (0..w).step_by((w / 40).max(1) as usize) {
            if let Some(c) = rep.colorAtX_y(x, y).and_then(|c| c.colorUsingColorSpace(&srgb)) {
                let p = (c.redComponent(), c.greenComponent(), c.blueComponent());
                if !seen.iter().any(|q| (q.0 - p.0).abs() + (q.1 - p.1).abs() + (q.2 - p.2).abs() < 0.05) {
                    seen.push(p);
                }
            }
        }
    }
    seen.len() >= 3
}

/// The sRGB colours (r, g, b in 0…1) of a capture from [`capture_window`]
/// inside `view`'s bounds: rows from the top, every `step` pixels.
pub fn captured_pixels(rep: &objc2_app_kit::NSBitmapImageRep, view: &objc2_app_kit::NSView, step: usize) -> Vec<Vec<(f64, f64, f64)>> {
    let Some(window) = view.window() else { return Vec::new() };
    let scale = window.backingScaleFactor();
    let r = view.convertRect_toView(view.bounds(), None);
    let height = window.frame().size.height;
    let (x0, x1) = ((r.origin.x * scale) as isize, ((r.origin.x + r.size.width) * scale) as isize);
    let (y0, y1) = (((height - r.origin.y - r.size.height) * scale) as isize, ((height - r.origin.y) * scale) as isize);
    let srgb = objc2_app_kit::NSColorSpace::sRGBColorSpace();
    let mut out = Vec::new();
    for y in (y0.max(0)..y1.min(rep.pixelsHigh())).step_by(step.max(1)) {
        let row = (x0.max(0)..x1.min(rep.pixelsWide()))
            .step_by(step.max(1))
            .map(|x| rep.colorAtX_y(x, y).and_then(|c| c.colorUsingColorSpace(&srgb)).map_or((0.0, 0.0, 0.0), |c| (c.redComponent(), c.greenComponent(), c.blueComponent())))
            .collect();
        out.push(row);
    }
    out
}

/// One accessibility element as VoiceOver gets it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AxElement {
    /// Nesting depth under the window's content view.
    pub depth: usize,
    /// `AXButton`, `AXTextField`…
    pub role: String,
    /// `AXSwitch`, `AXSearchField`…
    pub subrole: String,
    /// What VoiceOver reads as the name: the label, else the title, else a
    /// linked title element's value, else the placeholder; for static text
    /// (labels, table and sidebar rows) and a list's items, its text.
    pub name: String,
    /// The element's own label (set with `accessibility_label`).
    pub label: String,
    /// What VoiceOver reads as the value: a field's text, a checkbox's or
    /// switch's state (`0` or `1`; a switch's role is `AXButton`, subrole
    /// `AXSwitch`), a slider's or progress bar's number;
    /// empty when the element has none.
    pub value: String,
}

/// Every accessibility element under `window`'s content view, depth first,
/// including those inside SwiftUI views (`NSHostingView`), which SwiftUI
/// makes only once the system's accessibility API is asked: the walk asks
/// it first, as VoiceOver does.
pub fn ax_elements(window: &crate::WindowRef) -> Vec<AxElement> {
    let window = window.ns();
    use objc2::runtime::AnyObject;
    // Asked of the object, not its class: some elements (a table's or
    // outline's rows) answer through AppKit's own forwarding.
    fn responds(obj: &AnyObject, sel: objc2::runtime::Sel) -> bool {
        // SAFETY: every NSObject answers respondsToSelector:.
        unsafe { objc2::msg_send![obj, respondsToSelector: sel] }
    }
    // An attribute through the NSAccessibility protocol, else through the
    // older attribute API, which some of AppKit's own elements (a table's or
    // outline's rows) answer only to; VoiceOver asks both ways.
    fn attribute(obj: &AnyObject, sel: objc2::runtime::Sel, name: &str) -> Option<Retained<AnyObject>> {
        if responds(obj, sel) {
            // SAFETY: the selector exists and returns an object (or nil).
            unsafe { objc2::msg_send![obj, performSelector: sel] }
        } else if responds(obj, objc2::sel!(accessibilityAttributeValue:)) {
            // SAFETY: returns the attribute's value or nil.
            unsafe { objc2::msg_send![obj, accessibilityAttributeValue: &*NSString::from_str(name)] }
        } else {
            None
        }
    }
    fn string(obj: &AnyObject, sel: objc2::runtime::Sel, name: &str) -> String {
        attribute(obj, sel, name).and_then(|v| v.downcast::<NSString>().ok()).map(|s| s.to_string()).unwrap_or_default()
    }
    // A value as text: a string as it is, a number by its description.
    fn value(obj: &AnyObject) -> String {
        let Some(v) = attribute(obj, objc2::sel!(accessibilityValue), "AXValue") else { return String::new() };
        match v.downcast::<NSString>() {
            Ok(s) => s.to_string(),
            Err(v) => v.downcast::<objc2_foundation::NSNumber>().map(|n| n.stringValue().to_string()).unwrap_or_default(),
        }
    }
    fn walk(obj: &AnyObject, depth: usize, parent: &str, parent_name: &str, out: &mut Vec<AxElement>) {
        let label = string(obj, objc2::sel!(accessibilityLabel), "AXDescription");
        let title = string(obj, objc2::sel!(accessibilityTitle), "AXTitle");
        let linked = if responds(obj, objc2::sel!(accessibilityTitleUIElement)) {
            // SAFETY: returns an element or nil.
            let e: Option<Retained<AnyObject>> = unsafe { objc2::msg_send![obj, accessibilityTitleUIElement] };
            e.map(|e| string(&e, objc2::sel!(accessibilityValue), "AXValue")).unwrap_or_default()
        } else {
            String::new()
        };
        let placeholder = string(obj, objc2::sel!(accessibilityPlaceholderValue), "AXPlaceholderValue");
        let role = string(obj, objc2::sel!(accessibilityRole), "AXRole");
        // Static text (a label, a table or sidebar row's text) and the text
        // of a list's items (a column browser's) are read by their value.
        let read_by_value = role == "AXStaticText" || (role == "AXTextField" && ["AXList", "AXRow", "AXCell"].contains(&parent));
        let text = if read_by_value { string(obj, objc2::sel!(accessibilityValue), "AXValue") } else { String::new() };
        // An empty field in a table's cell is read by the cell's name (its
        // column's title).
        let text = if read_by_value && text.trim().is_empty() && parent == "AXCell" { parent_name.to_string() } else { text };
        let name = [&label, &title, &linked, &placeholder, &text].into_iter().find(|s| !s.trim().is_empty()).cloned().unwrap_or_default();
        out.push(AxElement {
            depth,
            role,
            subrole: string(obj, objc2::sel!(accessibilitySubrole), "AXSubrole"),
            name,
            label,
            value: value(obj),
        });
        if depth > 14 {
            return;
        }
        // A table's or list's rows: only the visible ones, as VoiceOver
        // walks them (asking a 200,000-row table for all its rows makes an
        // element for each).
        let kids = match out.last().map(|e| e.role.as_str()) {
            Some("AXTable" | "AXOutline") => attribute(obj, objc2::sel!(accessibilityVisibleRows), "AXVisibleRows"),
            Some("AXList") => attribute(obj, objc2::sel!(accessibilityVisibleChildren), "AXVisibleChildren"),
            _ => attribute(obj, objc2::sel!(accessibilityChildren), "AXChildren"),
        }
        .and_then(|k| k.downcast::<objc2_foundation::NSArray>().ok());
        let (role, name) = out.last().map(|e| (e.role.clone(), e.name.clone())).unwrap_or_default();
        for k in kids.iter().flat_map(|k| k.iter()) {
            walk(&k, depth + 1, &role, &name, out);
        }
    }
    let mut out = Vec::new();
    if let Some(content) = window.contentView() {
        wake_swiftui(&content);
        walk(&content, 0, "", "", &mut out);
    }
    out
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> *const std::ffi::c_void;
    fn AXUIElementCopyAttributeValue(element: *const std::ffi::c_void, attribute: *const std::ffi::c_void, value: *mut *const std::ffi::c_void) -> i32;
    fn CFRelease(object: *const std::ffi::c_void);
}

/// SwiftUI makes a hosting view's accessibility elements only once an
/// assistive app (VoiceOver) asks through the system accessibility API;
/// until then a walk inside the app finds one empty group. When `view` holds
/// a SwiftUI view, ask once that way, as VoiceOver would. Asking the app's
/// own process needs no Accessibility permission.
fn wake_swiftui(view: &objc2_app_kit::NSView) {
    fn has_hosting(v: &objc2_app_kit::NSView) -> bool {
        v.class().name().to_string_lossy().contains("HostingView") || v.subviews().iter().any(|s| has_hosting(&s))
    }
    if !has_hosting(view) {
        return;
    }
    let windows = NSString::from_str("AXWindows");
    // SAFETY: AX calls on this process's own application element; the
    // NSString is toll-free bridged to the CFString the call takes; each
    // object created or copied is released once.
    unsafe {
        let app = AXUIElementCreateApplication(std::process::id() as i32);
        if app.is_null() {
            return;
        }
        let mut value = std::ptr::null();
        if AXUIElementCopyAttributeValue(app, (&*windows as *const NSString).cast(), &mut value) == 0 && !value.is_null() {
            CFRelease(value);
        }
        CFRelease(app);
    }
}

/// What VoiceOver users would miss in `window`: controls with no name to
/// read (fields, editors, sliders, pop-ups, checkboxes, switches, buttons,
/// steppers), and images and controls read as a symbol name
/// ("speaker.wave.2"). Empty
/// when every control can be told apart by ear.
pub fn ax_audit(window: &crate::WindowRef) -> Vec<String> {
    const NEEDS_NAME: &[&str] =
        &["AXTextField", "AXTextArea", "AXSlider", "AXPopUpButton", "AXCheckBox", "AXRadioButton", "AXButton", "AXIncrementor", "AXComboBox", "AXMenuButton", "AXDateTimeArea", "AXColorWell"];
    let symbol_like = |s: &str| s.contains('.') && !s.contains(' ') && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.');
    let elements = ax_elements(window);
    // Whether element `i` lies inside a named date picker: its stepper and
    // calendar buttons are AppKit's own parts of that control.
    let in_named_date = |i: usize| {
        let mut depth = elements[i].depth;
        elements[..i].iter().rev().any(|a| {
            if a.depth < depth {
                depth = a.depth;
                return a.role == "AXDateTimeArea" && !a.name.trim().is_empty();
            }
            false
        })
    };
    elements
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            // A stepper's or scroll bar's arrows and pages, and the parts of a
            // named date picker, are parts VoiceOver names itself.
            let part = ["AXIncrementArrow", "AXDecrementArrow", "AXIncrementPage", "AXDecrementPage"].contains(&e.subrole.as_str()) || in_named_date(i);
            let unnamed = NEEDS_NAME.contains(&e.role.as_str()) && !part && e.name.trim().is_empty();
            // A control is only flagged for a real symbol's name: "sample.sqlite" is a file's.
            let is_symbol = |n: &str| e.role == "AXImage" || objc2_app_kit::NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(n), None).is_some();
            let symbol = (e.role == "AXImage" && e.name.is_empty()) || (symbol_like(&e.name) && is_symbol(&e.name));
            (unnamed || symbol).then(|| {
                let what = if e.subrole.is_empty() { e.role.clone() } else { format!("{} {}", e.role, e.subrole) };
                if symbol { format!("#{i} {what} named {:?}", e.name) } else { format!("#{i} {what} without a name") }
            })
        })
        .collect()
}

#[repr(C)]
#[derive(Default)]
struct MallocStatistics {
    blocks_in_use: u32,
    size_in_use: usize,
    max_size_in_use: usize,
    size_allocated: usize,
}

unsafe extern "C" {
    fn malloc_zone_statistics(zone: *mut std::ffi::c_void, stats: *mut MallocStatistics);
}

/// Memory the process has allocated and not freed, in MB (all malloc
/// zones). Unlike [`footprint_mb`], it drops as soon as something is freed:
/// the allocator returns freed pages to the system in batches, at times of
/// its own (seconds later when the machine is busy), so footprint can't tell
/// a leak from memory not yet handed back. Use this for leak checks.
pub fn heap_in_use_mb() -> f64 {
    let mut stats = MallocStatistics::default();
    // SAFETY: a null zone means all zones; `stats` is a valid out-pointer
    // with the C layout of malloc_statistics_t.
    unsafe { malloc_zone_statistics(std::ptr::null_mut(), &mut stats) };
    stats.size_in_use as f64 / (1024.0 * 1024.0)
}

unsafe extern "C" {
    fn sysctlbyname(name: *const std::ffi::c_char, old: *mut std::ffi::c_void, len: *mut usize, new: *mut std::ffi::c_void, new_len: usize) -> i32;
}

/// Whether this Mac is a virtual machine (GitHub's macOS runners are).
/// There the window server has no GPU, so materials don't blur, drawing is
/// slower and window-server input doesn't arrive: checks of those say SKIP
/// rather than fail.
pub fn virtual_machine() -> bool {
    let mut on: i32 = 0;
    let mut len = std::mem::size_of::<i32>();
    // SAFETY: a NUL-terminated name and an out-pointer of the length given.
    let ok = unsafe { sysctlbyname(c"kern.hv_vmm_present".as_ptr(), (&raw mut on).cast(), &mut len, std::ptr::null_mut(), 0) };
    ok == 0 && on == 1
}

/// What a [`canvas`](crate::canvas) (or chart) drew last, one line per
/// operation (`fill rect 10,20 30x40 Blue`, `text "Jan" 11pt`); `None` if
/// `view` isn't one.
pub fn canvas_drawing(view: &objc2_app_kit::NSView) -> Option<Vec<String>> {
    crate::canvas::describe(view)
}

/// A canvas as a PNG, on the content background: what its Copy Image puts
/// on the clipboard. `None` if `view` isn't a canvas.
pub fn canvas_png(view: &objc2_app_kit::NSView) -> Option<Vec<u8>> {
    crate::canvas::png(view)
}

/// Clicks a [`canvas`](crate::canvas) at `x, y` (points from its top
/// left), running its [`on_click`](crate::Canvas::on_click) handler; false
/// if it has none.
pub fn canvas_click(view: &objc2_app_kit::NSView, x: f64, y: f64) -> bool {
    crate::canvas::click_at(view, x, y)
}

/// Scrolls a [`canvas`](crate::canvas) (or chart) by `dx, dy` points with
/// the pointer at `x, y`, as a trackpad would; false if it didn't use it.
pub fn canvas_scroll(view: &objc2_app_kit::NSView, dx: f64, dy: f64, x: f64, y: f64) -> bool {
    crate::canvas::scroll_at(view, dx, dy, x, y)
}

/// Pinches a [`canvas`](crate::canvas) (or chart) by `amount` (0.5 is 50%
/// larger) at `x, y`; false if it didn't use it.
pub fn canvas_magnify(view: &objc2_app_kit::NSView, amount: f64, x: f64, y: f64) -> bool {
    crate::canvas::magnify_at(view, amount, x, y)
}

/// What dragging a canvas would carry now (its `draggable`, or the image
/// of an `exportable` one), without a drag session.
pub fn canvas_drag(view: &objc2_app_kit::NSView) -> Option<crate::DragItem> {
    crate::canvas::drag_item(view)
}

/// Drags inside a canvas from `from` to `to` (its coordinates), as a user's
/// drag reaches its [`on_drag`](crate::Canvas::on_drag); false without one.
pub fn canvas_move(view: &objc2_app_kit::NSView, from: (f64, f64), to: (f64, f64)) -> bool {
    crate::canvas::move_on(view, from, to)
}

/// Writes a promised file into `dir` as a drop there would; the path.
pub fn fulfil(item: &crate::DragItem, dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    crate::transfer::fulfil(item, dir)
}

/// Drops whatever is on `pasteboard` onto `view` as AppKit does at the end
/// of a drag (entered, then perform). Whether it was taken.
pub fn drop_on_view(view: &objc2_app_kit::NSView, pasteboard: &objc2_app_kit::NSPasteboard) -> bool {
    ferrokit_core::testing::drop_on_view(view, pasteboard)
}

/// Overrides the Reduce Motion setting animated changes follow (`None`:
/// the system's), so checks can run both paths without changing it.
pub fn set_reduce_motion(on: Option<bool>) {
    crate::motion::set_reduce_motion(on);
}

/// Sends a view made with `on_hover` or `hover_highlight` (its interaction
/// view, a `FerrokitInteraction`) the pointer entering or leaving. False if
/// `view` isn't one.
pub fn hover(view: &objc2_app_kit::NSView, inside: bool) -> bool {
    crate::interact::hover(view, inside)
}

/// Presses (`down`) or releases a view made with `on_click` (its
/// `FerrokitInteraction`), as the mouse does: the press dims it, the
/// release clicks. False if `view` isn't one.
pub fn press(view: &objc2_app_kit::NSView, down: bool) -> bool {
    crate::interact::press(view, down)
}

/// Makes [`notify`](crate::notify) record notifications instead of posting
/// them (no permission prompt, no banners); read them with
/// [`sent_notifications`].
pub fn hold_notifications() {
    crate::services::HOLD.with(|h| h.set(true));
}

/// The notifications recorded since [`hold_notifications`], oldest first
/// (one posted again with the same id replaces it; cancelled ones go).
pub fn sent_notifications() -> Vec<crate::SentNotification> {
    crate::services::SENT.with_borrow(|s| s.clone())
}

/// Runs the click action of the notification `id`, as clicking it in
/// Notification Center would; false if it has none.
pub fn click_notification(id: &str) -> bool {
    crate::services::click(id)
}

/// What a [`share_button`](crate::share_button) would share if clicked now.
pub fn shared_items(button: &objc2_app_kit::NSButton) -> Option<Vec<crate::ShareItem>> {
    crate::services::shared_by(button)
}

/// The app's menu-bar extras, oldest first, each with its panel's popover
/// (for extras with a panel).
pub fn menu_bar_extras() -> Vec<(Retained<objc2_app_kit::NSStatusItem>, Option<Retained<objc2_app_kit::NSPopover>>)> {
    crate::status::EXTRAS.with_borrow(|e| e.iter().map(|i| (i.item.clone(), i.popover.clone())).collect())
}


/// Signals in existence, for leak checks (a closed window's signals are freed).
pub fn live_signals() -> usize {
    crate::signal::live_signals()
}

/// View nodes in existence, for leak checks.
pub fn live_nodes() -> usize {
    crate::view::live_nodes()
}

/// Probes for Ferrokit's own checks: counters and switches that look inside
/// the framework (layout passes, effect runs, animations in flight). They
/// are **outside the stability promise**: they may change or go in any
/// release. An app's own checks need only what's in [`testing`](crate::testing)
/// itself.
pub mod probes {
    #[allow(unused_imports)]
    use super::*;

    /// Symbol effects running ([`Image::effect`](crate::Image::effect) while
    /// active) and one-shot effects played so far ([`Image::effect_on`](crate::Image::effect_on)).
    pub fn symbol_effects() -> (isize, usize) {
        crate::controls::SYMBOL_EFFECTS.with(std::cell::Cell::get)
    }

    /// Turns preview mode on or off for windows opened from now on, as
    /// `cargo ferrokit preview` does with `FERROKIT_PREVIEW=1`: a window
    /// without [`remember_frame`](crate::Window::remember_frame) keeps its frame
    /// under its title.
    pub fn set_preview_mode(on: bool) {
        crate::set_preview_mode(on);
    }

    /// Sends a ⌘-key equivalent through the main menu, as a key press would.
    pub fn key_equiv(w: &crate::WindowRef, chars: &str, shift: bool) -> bool {
        let w = w.ns();
        use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
        let mut flags = NSEventModifierFlags::Command;
        // As a real key press: Shift shows in the characters (⇧⌘Z sends "Z").
        let typed = if shift { chars.to_uppercase() } else { chars.to_string() };
        if shift {
            flags |= NSEventModifierFlags::Shift;
        }
        let event = {
            NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyDown,
                objc2_foundation::NSPoint::new(0.0, 0.0),
                flags,
                0.0,
                w.windowNumber(),
                None,
                &NSString::from_str(&typed),
                &NSString::from_str(&typed),
                false,
                0,
            )
        }
        .unwrap();
        NSApplication::sharedApplication(mtm()).mainMenu().unwrap().performKeyEquivalent(&event)
    }

    /// The titles of the app menu's items (the first menu: About, Check for
    /// Updates…, Settings…, Hide, Quit), separators as `"-"`.
    pub fn app_menu_titles() -> Vec<String> {
        let bar = NSApplication::sharedApplication(mtm()).mainMenu();
        let menu = bar.and_then(|b| b.itemAtIndex(0)).and_then(|i| i.submenu());
        menu.map(|m| m.itemArray().iter().map(|i| if i.isSeparatorItem() { "-".into() } else { i.title().to_string() }).collect()).unwrap_or_default()
    }

    /// Layout counters since the app started: nodes placed, subtrees skipped as
    /// clean, and ideal sizes computed (cache misses).
    pub fn layout_stats() -> (u64, u64, u64) {
            crate::layout::layout_stats()
    }

    /// How many times effects have run, for checks that an update does only the work it should.
    pub fn effect_runs() -> u64 {
            crate::signal::effect_runs()
    }

    /// Microseconds of the recent layout passes, oldest first (at least the last 10,000).
    pub fn layout_times_us() -> Vec<f64> {
            crate::layout_times_us()
    }

    /// Layout passes so far. With [`layout_times_us`], the times of the passes since an earlier count: `let t = layout_times_us(); &t[t.len() - (layout_passes() - n0) as usize..]`.
    pub fn layout_passes() -> u64 {
            crate::layout_passes()
    }

    /// Value and spring animations running now.
    pub fn motion_running() -> usize {
        crate::motion::running()
    }

    /// How many native views are still animating out after a transition.
    pub fn leaving_views() -> usize {
        crate::motion::leaving()
    }

    /// Whether a clickable view's interaction is pressed (`None` if `view`
    /// isn't one).
    pub fn is_pressed(view: &objc2_app_kit::NSView) -> Option<bool> {
        crate::interact::is_pressed(view)
    }

    /// Moves the clock springs, [`animate`](crate::animate) and
    /// [`keyframes`](crate::keyframes) run on forward by `seconds` and runs a
    /// frame: what's animating lands where it would be that much later.
    pub fn step_motion(seconds: f64) {
        crate::motion::step(seconds);
    }

    /// Remembers the documents open now, as quitting does (they reopen at the
    /// next launch when the user keeps windows: `NSQuitAlwaysKeepsWindows`,
    /// which a launch argument can set for one run).
    pub fn remember_open_documents() {
        crate::documents::remember_open();
    }

    /// Collection cells made since launch: stays near a screenful however many
    /// items scroll past.
    pub fn collection_cells_made() -> u64 {
            crate::collection::collection_cells_made()
    }
}
