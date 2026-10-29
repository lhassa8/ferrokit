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
    fn s(obj: &objc2::runtime::AnyObject, sel: &str) -> String {
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
