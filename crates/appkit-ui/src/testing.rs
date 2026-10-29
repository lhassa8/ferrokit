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
