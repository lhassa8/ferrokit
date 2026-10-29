//! Sheets: alerts, views in a sheet, and the Open and Save panels. Every
//! callback runs in a batch, once.

use std::cell::RefCell;
use std::path::PathBuf;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSModalResponse, NSModalResponseOK, NSOpenPanel, NSSavePanel,
    NSWindow,
};
use objc2_foundation::{NSArray, NSString, NSURL};

use crate::signal::batch;
use crate::Window;

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("main thread")
}

/// Wraps a one-shot callback for an AppKit completion block.
fn once<A: 'static>(f: impl FnOnce(A) + 'static) -> impl Fn(A) + 'static {
    let cell = RefCell::new(Some(f));
    move |a| {
        if let Some(f) = cell.borrow_mut().take() {
            batch(|| f(a));
        }
    }
}

/// An alert shown as a sheet (see [`alert`]).
pub struct Alert {
    title: String,
    message: String,
    buttons: Vec<String>,
    destructive: Option<usize>,
    warning: bool,
}

impl Alert {
    /// An alert with a title and an informative message.
    pub fn new(title: &str, message: &str) -> Self {
        Alert { title: title.into(), message: message.into(), buttons: Vec::new(), destructive: None, warning: false }
    }

    /// Adds a button (the first is the default; Return presses it, Escape
    /// presses one titled "Cancel").
    pub fn button(mut self, title: &str) -> Self {
        self.buttons.push(title.into());
        self
    }

    /// Marks button `index` as destructive (shown in red).
    pub fn destructive(mut self, index: usize) -> Self {
        self.destructive = Some(index);
        self
    }

    /// Shows the warning style (a caution icon).
    pub fn warning(mut self) -> Self {
        self.warning = true;
        self
    }
}

/// Shows `alert` as a sheet on `window`; `on_response` gets the index of the
/// button chosen.
pub fn alert(window: &NSWindow, alert: Alert, on_response: impl FnOnce(usize) + 'static) {
    let a = NSAlert::new(mtm());
    a.setMessageText(&NSString::from_str(&alert.title));
    a.setInformativeText(&NSString::from_str(&alert.message));
    a.setAlertStyle(if alert.warning { NSAlertStyle::Warning } else { NSAlertStyle::Informational });
    for (i, title) in alert.buttons.iter().enumerate() {
        let b = a.addButtonWithTitle(&NSString::from_str(title));
        if alert.destructive == Some(i) {
            b.setHasDestructiveAction(true);
        }
    }
    let respond = once(on_response);
    let block = RcBlock::new(move |r: NSModalResponse| respond((r - NSAlertFirstButtonReturn).max(0) as usize));
    a.beginSheetModalForWindow_completionHandler(window, Some(&block));
}

/// A sheet showing a view tree (see [`sheet`]). Closing it tears the tree
/// down.
#[derive(Clone)]
pub struct SheetHandle {
    parent: Retained<NSWindow>,
    sheet: Retained<NSWindow>,
}

impl SheetHandle {
    /// Closes the sheet (its `on_close` runs).
    pub fn close(&self) {
        self.parent.endSheet(&self.sheet);
    }

    /// The sheet's window.
    pub fn window(&self) -> Retained<NSWindow> {
        self.sheet.clone()
    }
}

/// Shows `content` (a [`Window`] built as usual: size and view) as a sheet on
/// `parent`. `on_close` runs when it closes; the view tree is then dropped.
pub fn sheet(parent: &NSWindow, content: Window, on_close: impl FnOnce() + 'static) -> SheetHandle {
    let sheet = objc2::rc::autoreleasepool(|_| content.open(mtm()));
    let handle = SheetHandle { parent: parent.retain(), sheet: sheet.clone() };
    let closed = once(move |()| on_close());
    let s = sheet.clone();
    let block = RcBlock::new(move |_r: NSModalResponse| {
        s.orderOut(None);
        // Views, undo history, delegates: as for a closed window.
        crate::teardown(&s);
        closed(());
    });
    parent.beginSheet_completionHandler(&sheet, Some(&block));
    handle
}

use objc2::Message as _;

/// What the Open panel allows.
#[derive(Debug, Clone, Default)]
pub struct OpenOptions {
    /// File name extensions to allow (e.g. `["csv", "txt"]`); empty: any.
    pub extensions: Vec<String>,
    /// Allow choosing several files.
    pub multiple: bool,
    /// Choose folders instead of files.
    pub directories: bool,
}

fn content_types(extensions: &[String]) -> Option<Retained<NSArray<objc2_uniform_type_identifiers::UTType>>> {
    if extensions.is_empty() {
        return None;
    }
    let types: Vec<_> = extensions
        .iter()
        .filter_map(|e| objc2_uniform_type_identifiers::UTType::typeWithFilenameExtension(&NSString::from_str(e)))
        .collect();
    Some(NSArray::from_retained_slice(&types))
}

/// Shows the Open panel as a sheet on `window`; `on_done` gets the chosen
/// paths, or `None` if cancelled.
pub fn open_panel(window: &NSWindow, options: OpenOptions, on_done: impl FnOnce(Option<Vec<PathBuf>>) + 'static) {
    let panel = NSOpenPanel::openPanel(mtm());
    panel.setAllowsMultipleSelection(options.multiple);
    panel.setCanChooseDirectories(options.directories);
    panel.setCanChooseFiles(!options.directories);
    if let Some(types) = content_types(&options.extensions) {
        panel.setAllowedContentTypes(&types);
    }
    let done = once(on_done);
    let p = panel.clone();
    let block = RcBlock::new(move |r: NSModalResponse| {
        let paths = (r == NSModalResponseOK)
            .then(|| p.URLs().iter().filter_map(|u| u.path()).map(|s| PathBuf::from(s.to_string())).collect());
        done(paths);
    });
    panel.beginSheetModalForWindow_completionHandler(window, &block);
}

/// Shows the Save panel as a sheet on `window`, proposing `name` (in
/// `directory`, if given); `on_done` gets the chosen path, or `None` if
/// cancelled. Nothing is written: saving is the app's job.
pub fn save_panel(window: &NSWindow, name: &str, directory: Option<&std::path::Path>, on_done: impl FnOnce(Option<PathBuf>) + 'static) {
    let panel = NSSavePanel::savePanel(mtm());
    panel.setNameFieldStringValue(&NSString::from_str(name));
    if let Some(dir) = directory {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
        panel.setDirectoryURL(Some(&url));
    }
    let done = once(on_done);
    let p = panel.clone();
    let block = RcBlock::new(move |r: NSModalResponse| {
        let path = (r == NSModalResponseOK).then(|| p.URL()).flatten().and_then(|u| u.path()).map(|s| PathBuf::from(s.to_string()));
        done(path);
    });
    panel.beginSheetModalForWindow_completionHandler(window, &block);
}
