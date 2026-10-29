//! Helpers for automated checks of apps built with appkit-ui: the
//! accessibility tree as VoiceOver sees it, key equivalents through the menu,
//! simulated edits, memory footprint and window snapshots. Used by the
//! checklist and the example apps' self-checks.

use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSTextField};
use objc2_foundation::NSString;

use crate::{mtm, windows};

/// Physical footprint in MB (what Activity Monitor shows; RSS keeps freed
/// heap pages and overstates growth), via vmmap.
pub fn footprint_mb() -> f64 {
    let out = std::process::Command::new("vmmap").args(["--summary", &std::process::id().to_string()]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.starts_with("Physical footprint:")).unwrap_or("");
    let v = line.split_whitespace().last().unwrap_or("0M");
    let (num, unit) = v.split_at(v.len() - 1);
    num.parse::<f64>().unwrap_or(0.0) * if unit == "G" { 1024.0 } else if unit == "K" { 1.0 / 1024.0 } else { 1.0 }
}

/// The process's peak physical footprint so far, in MB (via vmmap).
pub fn peak_footprint_mb() -> f64 {
    let out = std::process::Command::new("vmmap").args(["--summary", &std::process::id().to_string()]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.starts_with("Physical footprint (peak):")).unwrap_or("");
    let v = line.split_whitespace().last().unwrap_or("0M");
    let (num, unit) = v.split_at(v.len() - 1);
    num.parse::<f64>().unwrap_or(0.0) * if unit == "G" { 1024.0 } else if unit == "K" { 1.0 / 1024.0 } else { 1.0 }
}
/// The window's accessibility tree, flattened: (role, subrole, label/title)
/// for every element under the content view, as VoiceOver sees it. On
/// macOS 27, AppKit's text fields and checkboxes host SwiftUI internally, so
/// the AX element is a child of the NSView, not the view itself.
pub fn ax_tree(window: &objc2_app_kit::NSWindow) -> Vec<(String, String, String)> {
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
/// Renders every window to `<dir>/<title>.png` (AppKit's own drawing, no
/// screen-recording permission needed). The window's frame view is drawn, so
/// the image includes the title bar and the window background (which the
/// content view alone doesn't draw).
pub fn snapshot_windows(dir: &str) {
    for w in windows() {
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
/// Sends a ⌘-key equivalent through the main menu, as a key press would.
pub fn key_equiv(w: &objc2_app_kit::NSWindow, chars: &str, shift: bool) -> bool {
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

/// Sends a key-down with `chars` and `modifiers` to the main menu's key
/// equivalents (no window needed); true if an enabled item took it.
pub fn menu_key(chars: &str, modifiers: objc2_app_kit::NSEventModifierFlags) -> bool {
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
/// `shift`; AppKit's rule: an uppercase key equivalent implies Shift) and
/// sends its action along `window`'s responder chain, starting at its first
/// responder. Returns whether an item matched and something handled it.
///
/// (A synthesized key event through `NSMenu.performKeyEquivalent` matches
/// the item but goes nowhere when the app isn't active: there's no key
/// window. A person pressing keys always has the app active.)
pub fn shortcut(window: &objc2_app_kit::NSWindow, chars: &str, shift: bool) -> bool {
    use objc2_app_kit::{NSEventModifierFlags, NSMenu};
    fn find(menu: &NSMenu, key: &str, shift: bool) -> Option<objc2::runtime::Sel> {
        for i in 0..menu.numberOfItems() {
            let item = menu.itemAtIndex(i)?;
            if let Some(sub) = item.submenu() {
                if let Some(sel) = find(&sub, key, shift) {
                    return Some(sel);
                }
                continue;
            }
            let k = item.keyEquivalent().to_string();
            let mask = item.keyEquivalentModifierMask();
            let wants_shift = mask.contains(NSEventModifierFlags::Shift) || (k.chars().all(char::is_uppercase) && !k.is_empty());
            if k.to_lowercase() == key && mask.contains(NSEventModifierFlags::Command) && wants_shift == shift {
                return item.action();
            }
        }
        None
    }
    let app = NSApplication::sharedApplication(mtm());
    let Some(action) = app.mainMenu().and_then(|m| find(&m, &chars.to_lowercase(), shift)) else { return false };
    // SAFETY: standard responder-chain dispatch with a nil sender.
    window.firstResponder().is_some_and(|r| unsafe { r.tryToPerform_with(action, None) })
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
pub fn capture_window(window: &objc2_app_kit::NSWindow) -> Option<Retained<objc2_app_kit::NSBitmapImageRep>> {
    let path = std::env::temp_dir().join(format!("appkit-ui-capture-{}-{}.png", std::process::id(), window.windowNumber()));
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
pub struct AxElement {
    /// Nesting depth under the window's content view.
    pub depth: usize,
    /// `AXButton`, `AXTextField`…
    pub role: String,
    /// `AXSwitch`, `AXSearchField`…
    pub subrole: String,
    /// What VoiceOver reads as the name: the label, else the title, else a
    /// linked title element's value, else the placeholder.
    pub name: String,
    /// The element's own label (set with `accessibility_label`).
    pub label: String,
}

/// Every accessibility element under `window`'s content view, depth first.
pub fn ax_elements(window: &objc2_app_kit::NSWindow) -> Vec<AxElement> {
    use objc2::runtime::AnyObject;
    fn string(obj: &AnyObject, sel: objc2::runtime::Sel) -> String {
        if !obj.class().responds_to(sel) {
            return String::new();
        }
        // SAFETY: the selector exists and returns an object (or nil).
        let v: Option<Retained<AnyObject>> = unsafe { objc2::msg_send![obj, performSelector: sel] };
        v.and_then(|v| v.downcast::<NSString>().ok()).map(|s| s.to_string()).unwrap_or_default()
    }
    fn walk(obj: &AnyObject, depth: usize, out: &mut Vec<AxElement>) {
        let label = string(obj, objc2::sel!(accessibilityLabel));
        let title = string(obj, objc2::sel!(accessibilityTitle));
        let linked = if obj.class().responds_to(objc2::sel!(accessibilityTitleUIElement)) {
            // SAFETY: returns an element or nil.
            let e: Option<Retained<AnyObject>> = unsafe { objc2::msg_send![obj, accessibilityTitleUIElement] };
            e.map(|e| string(&e, objc2::sel!(accessibilityValue))).unwrap_or_default()
        } else {
            String::new()
        };
        let placeholder = string(obj, objc2::sel!(accessibilityPlaceholderValue));
        let name = [&label, &title, &linked, &placeholder].into_iter().find(|s| !s.trim().is_empty()).cloned().unwrap_or_default();
        out.push(AxElement {
            depth,
            role: string(obj, objc2::sel!(accessibilityRole)),
            subrole: string(obj, objc2::sel!(accessibilitySubrole)),
            name,
            label,
        });
        if depth > 14 || !obj.class().responds_to(objc2::sel!(accessibilityChildren)) {
            return;
        }
        // SAFETY: returns an array of elements or nil.
        let kids: Option<Retained<objc2_foundation::NSArray<AnyObject>>> = unsafe { objc2::msg_send![obj, accessibilityChildren] };
        for k in kids.iter().flat_map(|k| k.iter()) {
            walk(&k, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    if let Some(content) = window.contentView() {
        walk(&content, 0, &mut out);
    }
    out
}

/// What VoiceOver users would miss in `window`: controls with no name to
/// read (fields, editors, sliders, pop-ups, checkboxes, switches, buttons,
/// steppers) and images read as a symbol name ("speaker.wave.2"). Empty
/// when every control can be told apart by ear.
pub fn ax_audit(window: &objc2_app_kit::NSWindow) -> Vec<String> {
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
            let symbol = e.role == "AXImage" && (e.name.is_empty() || symbol_like(&e.name));
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
