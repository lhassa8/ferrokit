//! The default main menu: App, Edit, Window and Help, wired to AppKit's
//! standard actions through the responder chain (nil targets).

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

fn item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str) -> Retained<NSMenuItem> {
    // SAFETY: plain menu item construction; actions go up the responder chain.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    }
}

fn with_modifiers(item: Retained<NSMenuItem>, flags: NSEventModifierFlags) -> Retained<NSMenuItem> {
    item.setKeyEquivalentModifierMask(flags);
    item
}

fn submenu(mtm: MainThreadMarker, title: &str, items: Vec<Option<Retained<NSMenuItem>>>) -> (Retained<NSMenuItem>, Retained<NSMenu>) {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    for i in items {
        match i {
            Some(i) => menu.addItem(&i),
            None => menu.addItem(&NSMenuItem::separatorItem(mtm)),
        }
    }
    let holder = item(mtm, title, None, "");
    holder.setSubmenu(Some(&menu));
    (holder, menu)
}

/// Builds and installs the standard menu bar for an app called `name`.
pub(crate) fn install(mtm: MainThreadMarker, name: &str) {
    let app = NSApplication::sharedApplication(mtm);
    let bar = NSMenu::new(mtm);
    let opt_cmd = NSEventModifierFlags::Option | NSEventModifierFlags::Command;

    let (app_item, _) = submenu(
        mtm,
        name,
        vec![
            Some(item(mtm, &format!("About {name}"), Some(sel!(orderFrontStandardAboutPanel:)), "")),
            None,
            Some(item(mtm, &format!("Hide {name}"), Some(sel!(hide:)), "h")),
            Some(with_modifiers(item(mtm, "Hide Others", Some(sel!(hideOtherApplications:)), "h"), opt_cmd)),
            Some(item(mtm, "Show All", Some(sel!(unhideAllApplications:)), "")),
            None,
            Some(item(mtm, &format!("Quit {name}"), Some(sel!(terminate:)), "q")),
        ],
    );
    bar.addItem(&app_item);

    let (edit_item, _) = submenu(
        mtm,
        "Edit",
        vec![
            Some(item(mtm, "Undo", Some(sel!(undo:)), "z")),
            // Shift is implied by the uppercase key (AppKit's convention); a
            // lowercase "z" with an explicit Shift mask doesn't match ⇧⌘Z.
            Some(item(mtm, "Redo", Some(sel!(redo:)), "Z")),
            None,
            Some(item(mtm, "Cut", Some(sel!(cut:)), "x")),
            Some(item(mtm, "Copy", Some(sel!(copy:)), "c")),
            Some(item(mtm, "Paste", Some(sel!(paste:)), "v")),
            Some(item(mtm, "Delete", Some(sel!(delete:)), "")),
            Some(item(mtm, "Select All", Some(sel!(selectAll:)), "a")),
        ],
    );
    bar.addItem(&edit_item);

    let (window_item, window_menu) = submenu(
        mtm,
        "Window",
        vec![
            Some(item(mtm, "Minimize", Some(sel!(performMiniaturize:)), "m")),
            Some(item(mtm, "Zoom", Some(sel!(performZoom:)), "")),
            None,
            Some(item(mtm, "Bring All to Front", Some(sel!(arrangeInFront:)), "")),
        ],
    );
    bar.addItem(&window_item);

    // The Help menu's search field is added by AppKit.
    let (help_item, help_menu) = submenu(mtm, "Help", vec![]);
    bar.addItem(&help_item);

    app.setMainMenu(Some(&bar));
    // AppKit lists open windows in the Window menu.
    app.setWindowsMenu(Some(&window_menu));
    app.setHelpMenu(Some(&help_menu));
}
