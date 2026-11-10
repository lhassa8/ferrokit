//! A native macOS UI framework for Rust: every view is a real AppKit view
//! (`NSTableView`, `NSTextView`, `NSSplitView`…), built once and updated in
//! place by fine-grained signals. No virtual DOM, no diffing, no web view.
//!
//! ```no_run
//! use ferrokit::*;
//!
//! let count = Signal::new(0);
//! App::new("Counter")
//!     .window(Window::new("Counter").size(320.0, 140.0).view(
//!         vstack((
//!             text(move || format!("Clicked {} times", count.get())),
//!             button("Click me", move || count.update(|n| *n += 1)),
//!         ))
//!         .padding(20.0),
//!     ))
//!     .run();
//! ```
//!
//! - **State:** [`Signal`] and [`Memo`] (`Copy` handles, freed with the
//!   window, row or document that made them), [`Effect`], [`batch`],
//!   [`spawn`] and [`Resource`] for background work and loading,
//!   [`stored`] for settings that persist.
//! - **Views and layout:** [`vstack`], [`hstack`], [`grid`], [`spacer`],
//!   [`scroll`], [`for_each`] (keyed lists), [`dynamic`], and the
//!   [`ViewExt`] modifiers (padding, sizes, context menus, accessibility
//!   labels), and [`ViewExt::on_command`] for menu [`Command`]s that act
//!   on the focus.
//! - **Controls:** [`text_field`], [`toggle`], [`checkbox`], [`picker`],
//!   [`slider`], [`stepper`], [`progress`], [`image`], [`button`].
//! - **Deep widgets:** [`table`] (millions of rows, sorting, editing, drag
//!   and drop), [`outline`] and [`sidebar`], [`text_editor`].
//! - **App shell:** menus ([`App::menu`]), toolbars, [`split`] views and
//!   [`Window::sidebar`]/[`Window::inspector`], [`sheet`]s, [`alert`]s,
//!   popovers ([`ViewExt::popover`]), open and save panels, document apps, a Settings window,
//!   window restoration, undo ([`undoable`]), [`on_frame`] for live data.
//! - **Forms and more controls:** [`form`], [`date_picker`], [`tabs`],
//!   [`gauge`], token fields, path controls, ratings.
//! - **Look:** text styles, cards and shadows ([`ViewExt::font`],
//!   [`ViewExt::background`], [`ViewExt::corner_radius`],
//!   [`ViewExt::shadow`]), materials and Liquid Glass ([`ViewExt::material`],
//!   [`ViewExt::glass`]), motion ([`animated`], [`ViewExt::transition`],
//!   [`ViewExt::on_hover`]).
//! - **Charts and drawing:** [`bar_chart`], [`line_chart`], [`area_chart`],
//!   [`scatter_chart`], [`pie_chart`], and a [`canvas`].
//! - **Web, PDF, video and services:** [`web_view`], [`pdf_view`],
//!   [`video_player`], [`share_button`], [`print`](fn@print), [`notify`].
//! - **Testing:** [`testing`] (self-checks, real input, accessibility
//!   audits, window captures, soaks).
//!
//! **Start with the [guide], 14 chapters whose examples are compiled as doc
//! tests.** `API.md`, in the crate's source and
//! [on GitHub](https://github.com/lhassa8/ferrokit/blob/main/crates/ferrokit/API.md),
//! lists every public item on one page. macOS 15 or later. Main thread only.

#![warn(missing_docs)]
#![doc(html_logo_url = "https://raw.githubusercontent.com/lhassa8/ferrokit/main/assets/icon.png", html_favicon_url = "https://raw.githubusercontent.com/lhassa8/ferrokit/main/assets/icon-128.png")]

mod appkit_views;
mod canvas;
mod charts;
mod chrome;
mod collection;
mod commands;
mod controls;
mod dialogs;
mod documents;
mod frame;
mod l10n;
mod layout;
mod material;
mod browser;
mod crash;
mod failure;
mod filter;
mod form;
mod media;
mod quicklook;
mod services;
mod status;
mod motion;
mod native;
mod related;
mod widgets;
mod interact;
mod menu;
mod models;
mod outline;
mod platform;
mod popover;
mod resource;
mod signal;
mod store;
mod style;
mod surface;
mod syntax;
mod transfer;
mod undo;
pub mod updates;
mod value;
pub mod testing;
mod view;

use ferrokit_core::kept::{UiCell, UiLocal};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSResponder, NSView, NSWindowStyleMask,
};
use objc2_foundation::NSObject;

use platform::{Native, Platform};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub use ferrokit_core::{is_dark, system_colors};
pub use frame::{FrameLoop, on_frame};
pub use store::{Stored, read_stored, remove_stored, stored, write_stored};
pub use undo::{set_undoable, undoable, undoable_in};
pub use commands::{Command, can_perform, can_perform_in, focused_value, focused_value_in, focused_window, menu_command, perform, perform_in, toolbar_command};
pub use resource::{Cancel, Load, Resource};
pub use signal::{Effect, Memo, Signal, after_batch, batch, effect_runs, live_signals, untrack};
pub use controls::{
    ColorWell, ComboBox, DatePicker, SearchField, Segmented, TimePicker, color_well, combo_box, date_picker, search_field, segmented, segmented_with, time_picker,
    Checkbox, Image, Picker, Progress, Slider, Stepper, SymbolEffect, TextField, checkbox, image, image_data, image_file, picker, progress, secure_field, slider,
    stepper, text_field, toggle,
};
pub use chrome::{MenuItem, Modifier, Split, customize_toolbar_item, dynamic_submenu, format_menu, submenu, ToolbarItem, menu_item, menu_separator, split, toolbar_button, toolbar_fixed_space, toolbar_group, toolbar_picker, toolbar_search, toolbar_space};
pub use dialogs::{Alert, OpenOptions, SheetHandle, alert, open_panel, save_panel, sheet, sheet_with};
pub use documents::{
    Document, DocumentType, choose_documents, document_edited, new_document, open_document, recent_documents, regain_access, remember_access, revert_document, save_document, save_document_as, show_welcome, welcome_window,
};
pub use layout::layout_stats;
pub use models::{Table, TextEditor, table, text_editor};
pub use outline::{Outline, TreeNode, outline, sidebar};
pub use collection::{Collection, CollectionItem, ItemImage, collection, collection_cells_made};
pub use ferrokit_core::table::{CellImage, CellStyle, Column, ColumnState, Sort, TableKey, TableModel};
pub use ferrokit_core::text::{Color, Completion, TextModel, TextStyle};
pub use syntax::Syntax;
pub use style::{Font, Weight};
pub use surface::Shadow;
pub use material::{Glass, Material, has_liquid_glass};
pub use widgets::{Gauge, Link, PathControl, RadioGroup, TabStyle, Tabs, TokenField, gauge, link, path_control, radio_group, rating, tabs, token_field};
pub use media::{PdfView, VideoControls, VideoPlayer, WebPage, WebView, pdf_view, pdf_view_data, video_player, video_stream, web_view};
pub use browser::{ColumnBrowser, column_browser};
pub use crash::CrashReport;
pub use updates::UpdateEvent;
pub use failure::Failure;
pub use filter::{FieldKind, FieldValue, Filter, FilterEditor, FilterField, FilterRule, FilterTest, Match, filter_editor};
pub use form::{Form, FormRow, FormStyle, Section, disclosure, form, form_content, form_row, group_box, section};
pub use quicklook::{Thumbnail, close_quick_look, is_quick_look_open, quick_look, thumbnail};
pub use services::{FULL_DISK_ACCESS_SETTINGS, FileDates, Notification, SentNotification, ShareButton, ShareItem, asks_before_reading, cancel_notification, file_dates, has_full_disk_access, move_to_trash, notifications_supported, notify, open_url, pdf_data, print, reveal_in_finder, save_pdf, share_button};
pub use status::MenuBarExtra;
pub use motion::{Animation, Keyframes, Transition, animate, animated, animated_with, keyframes, reduce_motion, stop_animation};
pub use popover::{Edge, toolbar_popover};
pub use canvas::{Canvas, CanvasDrag, Painter, canvas};
pub use native::{NativeView, native};
pub use related::{RelatedFiles, related_files};
pub use value::{IntoValue, Value};
pub use transfer::{Clipboard, DragItem, WriteFile};
pub use l10n::{format_date, format_number, format_number_for, local_day, local_day_start, today, weekday, language, parse_strings, plural_category, set_language, t, t_plural, t_with};
pub use charts::{Chart, Format, Rule, Series, area_chart, bar_chart, heatmap, histogram, line_chart, pie_chart, scatter_chart, sparkline, stacked_bar_chart};
pub use ferrokit_core::ViewId;
/// The AppKit window type the window functions take and return.
pub use objc2_app_kit::NSWindow;
/// The owning pointer to an AppKit object (as returned by [`open`]).
pub use objc2::rc::Retained;
pub use view::{
    Align, AnyView, Axis, Cx, Dynamic, ForEach, Grid, Identifiable, Modified, Node, Size, Spacer, Stack, View, ViewExt, Views,
    dynamic, for_each, grid, hstack, zstack, Layers, live_nodes, spacer, vstack, watch, Watch,
};
#[allow(deprecated)]
pub use appkit_views::{Button, ButtonStyle, ControlSize, Divider, HelpButton, Label, MenuButton, Scroll, Text, button, divider, help_button, label, menu_button, scroll, text};

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("ferrokit must be used on the main thread")
}

/// A window to open when the app starts.
pub struct Window {
    title: String,
    size: (f64, f64),
    content: Option<Box<dyn FnOnce() -> Retained<NSView>>>,
    view: Option<AnyView>,
    toolbar: Vec<ToolbarItem>,
    customizable_toolbar: Option<String>,
    autosave: Option<String>,
    fixed: bool,
    responds: commands::Responds,
    drops: Option<FileDrop>,
    chrome: WindowChrome,
    panel: Option<PanelStyle>,
    tabbing: Option<(Tabbing, Option<String>)>,
    full_screen: bool,
    /// The sidebar and inspector shell's panes, and their hidden signals.
    sidebar: Option<(AnyView, Option<Signal<bool>>)>,
    inspector: Option<(AnyView, Option<Signal<bool>>)>,
}

/// Whether a window opens as a tab of its app's other windows
/// ([`Window::tabbing`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Tabbing {
    /// As the user set in System Settings ▸ Desktop & Dock ("Prefer tabs").
    Automatic,
    /// Always as a tab, as Safari's and the Finder's windows do.
    Preferred,
    /// Never as a tab, and no tab bar (a settings window, a viewer).
    Disallowed,
}

/// A panel's kind ([`Window::panel`]): a secondary window serving the main
/// ones, which hides while the app is in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PanelStyle {
    /// Stays above the app's other windows: an inspector, a palette, a
    /// find panel. It becomes key only when a field in it is clicked, so
    /// the main window keeps the keyboard.
    Floating,
    /// As `Floating`, with a slimmer title bar: tool palettes.
    Utility,
    /// A dark, translucent heads-up panel over media or a canvas.
    Hud,
}

/// How a window's title bar and toolbar look.
#[derive(Default)]
struct WindowChrome {
    toolbar_style: Option<ToolbarStyle>,
    hide_title: bool,
    transparent_title_bar: bool,
    full_size_content: bool,
    background: Option<Color>,
}

/// How a window's toolbar sits with its title (see [`Window::toolbar_style`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolbarStyle {
    /// The title and toolbar items in one bar (the default).
    Unified,
    /// One bar, shorter: for windows with few toolbar items.
    UnifiedCompact,
    /// The title above the toolbar, items labelled (as in Settings windows).
    /// The other styles show symbols alone.
    Expanded,
    /// The title above the toolbar, items centred (Settings' tabs).
    Preference,
}

impl ToolbarStyle {
    fn ns(self) -> objc2_app_kit::NSWindowToolbarStyle {
        use objc2_app_kit::NSWindowToolbarStyle as S;
        match self {
            ToolbarStyle::Unified => S::Unified,
            ToolbarStyle::UnifiedCompact => S::UnifiedCompact,
            ToolbarStyle::Expanded => S::Expanded,
            ToolbarStyle::Preference => S::Preference,
        }
    }
}

/// What a window takes when files are dropped on it.
struct FileDrop {
    /// Extensions it takes (lowercase; empty: any file).
    extensions: Vec<String>,
    action: Rc<dyn Fn(Vec<std::path::PathBuf>)>,
}

impl FileDrop {
    fn accepted(&self, pasteboard: &objc2_app_kit::NSPasteboard) -> Vec<std::path::PathBuf> {
        transfer::files_on(pasteboard)
            .into_iter()
            .filter(|p| {
                self.extensions.is_empty()
                    || p.extension().is_some_and(|e| self.extensions.contains(&e.to_string_lossy().to_lowercase()))
            })
            .collect()
    }
}

thread_local! {
    /// Window roots that take dropped files, and what they do with them.
    static DROPS: UiCell<Vec<(usize, Rc<FileDrop>)>> = const { UiCell::new(Vec::new()) };
}

fn drop_operation(root: &RootView, info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> objc2_app_kit::NSDragOperation {
    use objc2_app_kit::NSDraggingInfo;
    let takes = file_drop(root).is_some_and(|d| !d.accepted(&info.draggingPasteboard()).is_empty());
    if takes { objc2_app_kit::NSDragOperation::Copy } else { objc2_app_kit::NSDragOperation::None }
}

fn perform_drop(root: &RootView, info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> bool {
    use objc2_app_kit::NSDraggingInfo;
    let Some(drop) = file_drop(root) else { return false };
    let paths = drop.accepted(&info.draggingPasteboard());
    if paths.is_empty() {
        return false;
    }
    // After AppKit's drag ends: the action may open windows.
    let action = drop.action.clone();
    let paths = RefCell::new(Some(paths));
    later(move || {
        if let Some(paths) = paths.borrow_mut().take() {
            batch(|| action(paths));
        }
    });
    true
}

fn file_drop(root: &RootView) -> Option<Rc<FileDrop>> {
    DROPS.with_borrow(|d| d.iter().find(|(k, _)| *k == root as *const RootView as usize).map(|(_, f)| f.clone()))
}

impl Window {
    /// Files dropped anywhere on the window (that its views don't take
    /// themselves) go to `action`: those with one of `extensions`, or any
    /// file if it's empty. The pointer shows a copy badge over the window
    /// while they're dragged.
    pub fn on_drop_files(mut self, extensions: &[&str], action: impl Fn(Vec<std::path::PathBuf>) + 'static) -> Self {
        self.drops = Some(FileDrop { extensions: extensions.iter().map(|e| e.to_lowercase()).collect(), action: Rc::new(action) });
        self
    }

    /// A titled, resizable window of 800 × 600 points.
    pub fn new(title: &str) -> Self {
        Window {
            title: title.into(),
            size: (800.0, 600.0),
            content: None,
            view: None,
            toolbar: Vec::new(),
            customizable_toolbar: None,
            autosave: None,
            fixed: false,
            responds: commands::Responds::default(),
            drops: None,
            chrome: WindowChrome::default(),
            panel: None,
            tabbing: None,
            full_screen: true,
            sidebar: None,
            inspector: None,
        }
    }

    /// A sidebar beside the window's [`view`](Self::view), as in Mail,
    /// Notes and the Finder (AppKit's split view controller): full height,
    /// under the title bar, on the sidebar material; the title and toolbar
    /// items start beside it, a Toggle Sidebar button leads the toolbar,
    /// and the user can drag it narrower, wider or shut. Put an
    /// [`outline`] or a source-list [`sidebar`](fn@crate::sidebar) in it.
    pub fn sidebar(mut self, view: impl View) -> Self {
        self.sidebar = Some((AnyView::new(view), self.sidebar.and_then(|s| s.1)));
        self
    }

    /// Whether the sidebar is hidden, bound two ways: setting it shows or
    /// hides the sidebar (animated), and the user's toggling sets it.
    pub fn sidebar_hidden(mut self, hidden: &Signal<bool>) -> Self {
        if let Some(s) = &mut self.sidebar {
            s.1 = Some(*hidden);
        }
        self
    }

    /// An inspector after the window's [`view`](Self::view), as in Keynote
    /// and Xcode (AppKit's inspector split item, macOS 14): full height, a
    /// Toggle Inspector button ending the toolbar, the user can drag it
    /// wider or shut.
    pub fn inspector(mut self, view: impl View) -> Self {
        self.inspector = Some((AnyView::new(view), self.inspector.and_then(|s| s.1)));
        self
    }

    /// Whether the inspector is hidden, bound two ways.
    pub fn inspector_hidden(mut self, hidden: &Signal<bool>) -> Self {
        if let Some(s) = &mut self.inspector {
            s.1 = Some(*hidden);
        }
        self
    }

    /// How the window joins its app's other windows as tabs. Windows join
    /// those with the same `group` (all the app's, if `None`); the Window
    /// menu's Show Tab Bar, Merge All Windows and the tab bar's + work as in
    /// any Mac app.
    pub fn tabbing(mut self, tabbing: Tabbing, group: Option<&str>) -> Self {
        self.tabbing = Some((tabbing, group.map(Into::into)));
        self
    }

    /// No full screen: the green button only zooms (a palette, a small
    /// utility). Resizable windows can go full screen by default.
    pub fn no_full_screen(mut self) -> Self {
        self.full_screen = false;
        self
    }

    /// Makes it a panel (`NSPanel`): floating above the app's windows,
    /// hidden while the app is inactive, and not in the Window menu's list
    /// of documents.
    pub fn panel(mut self, style: PanelStyle) -> Self {
        self.panel = Some(style);
        self
    }

    /// How the toolbar sits with the title ([`ToolbarStyle::Unified`] by
    /// default).
    pub fn toolbar_style(mut self, style: ToolbarStyle) -> Self {
        self.chrome.toolbar_style = Some(style);
        self
    }

    /// Hides the title in the title bar (it still names the window in the
    /// Window menu and Mission Control): for windows whose content says
    /// what they are.
    pub fn hide_title(mut self) -> Self {
        self.chrome.hide_title = true;
        self
    }

    /// A title bar without its own background: the window's background
    /// shows through it, so the title bar and content read as one surface.
    pub fn transparent_title_bar(mut self) -> Self {
        self.chrome.transparent_title_bar = true;
        self
    }

    /// Content under the title bar and toolbar, which become transparent.
    /// The content's backgrounds and materials, and a [`split`]'s panes,
    /// reach the window's top edge (a sidebar's material runs up behind the
    /// window buttons, as in Apple's apps), while the content itself starts
    /// below the title bar.
    pub fn full_size_content(mut self) -> Self {
        self.chrome.full_size_content = true;
        self.chrome.transparent_title_bar = true;
        self
    }

    /// The window's background colour (the system's window background by
    /// default). Named colours follow Dark Mode.
    pub fn background(mut self, color: Color) -> Self {
        self.chrome.background = Some(color);
        self
    }

    /// The content size in points.
    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.size = (width, height);
        self
    }

    /// The window's content: `view`, laid out to fill the window.
    pub fn view(mut self, view: impl View) -> Self {
        self.view = Some(AnyView::new(view));
        self
    }

    /// A unified toolbar with these items.
    pub fn toolbar(mut self, items: Vec<ToolbarItem>) -> Self {
        self.toolbar = items;
        self
    }

    /// Lets the user customise the toolbar: Customize Toolbar… on its
    /// right-click menu (and in a View menu, with
    /// [`customize_toolbar_item`]) opens AppKit's sheet, where items can be
    /// added, removed and moved, including [optional](ToolbarItem::optional)
    /// ones and spaces; Command-dragging moves them too. The layout is
    /// remembered under `name` across launches; until the user changes it,
    /// the app's own layout shows.
    pub fn customizable_toolbar(mut self, name: &str) -> Self {
        self.customizable_toolbar = Some(name.into());
        self
    }

    /// Handles `command` while this window is focused and nothing nearer the
    /// focus handles it (see [`Command`]), wherever the focus is: in the
    /// content, the sidebar or the inspector.
    pub fn on_command(mut self, command: Command, action: impl Fn() + 'static) -> Self {
        self.responds.commands.push((command, commands::Handler::new(action, None)));
        self
    }

    /// Handles `command` like [`on_command`](Self::on_command), enabled only
    /// while `enabled` returns true.
    pub fn on_command_when(mut self, command: Command, enabled: impl Fn() -> bool + 'static, action: impl Fn() + 'static) -> Self {
        self.responds.commands.push((command, commands::Handler::new(action, Some(Box::new(enabled)))));
        self
    }

    /// Saves the window's position and size under `name` whenever they
    /// change, and reopens it there next time (AppKit's frame autosave, in
    /// the app's user defaults).
    pub fn remember_frame(mut self, name: &str) -> Self {
        self.autosave = Some(name.into());
        self
    }

    /// A plain `NSView` as the window's content (escape hatch; no layout).
    pub fn content(mut self, build: impl FnOnce() -> Retained<NSView> + 'static) -> Self {
        self.content = Some(Box::new(build));
        self
    }

    pub(crate) fn open(mut self, mtm: MainThreadMarker) -> Retained<NSWindow> {
        let (sidebar, inspector) = (self.sidebar.take(), self.inspector.take());
        let shell = (sidebar.is_some(), inspector.is_some());
        // A sidebar runs up under the title bar.
        if shell.0 {
            self.chrome.full_size_content = true;
        }
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(self.size.0, self.size.1));
        let mut style = if self.fixed {
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable
        } else {
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable
        };
        if self.chrome.full_size_content {
            style |= NSWindowStyleMask::FullSizeContentView;
        }
        let window: Retained<NSWindow> = match self.panel {
            None => {
                // SAFETY: standard window creation on the main thread.
                unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), rect, style, NSBackingStoreType::Buffered, false) }
            }
            Some(kind) => {
                style.remove(NSWindowStyleMask::Miniaturizable);
                match kind {
                    PanelStyle::Floating => {}
                    PanelStyle::Utility => style |= NSWindowStyleMask::UtilityWindow,
                    PanelStyle::Hud => style |= NSWindowStyleMask::UtilityWindow | NSWindowStyleMask::HUDWindow,
                }
                let panel =
                    objc2_app_kit::NSPanel::initWithContentRect_styleMask_backing_defer(objc2_app_kit::NSPanel::alloc(mtm), rect, style, NSBackingStoreType::Buffered, false);
                panel.setFloatingPanel(true);
                panel.setBecomesKeyOnlyIfNeeded(true);
                panel.setHidesOnDeactivate(true);
                panel.setExcludedFromWindowsMenu(true);
                Retained::into_super(panel)
            }
        };
        // SAFETY: the app keeps its windows (see `App::windows`), so AppKit
        // must not release them on close.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(&self.title));
        if let Some((tabbing, group)) = &self.tabbing {
            use objc2_app_kit::NSWindowTabbingMode;
            window.setTabbingMode(match tabbing {
                Tabbing::Automatic => NSWindowTabbingMode::Automatic,
                Tabbing::Preferred => NSWindowTabbingMode::Preferred,
                Tabbing::Disallowed => NSWindowTabbingMode::Disallowed,
            });
            if let Some(g) = group {
                window.setTabbingIdentifier(&NSString::from_str(g));
            }
        }
        if !self.full_screen {
            use objc2_app_kit::NSWindowCollectionBehavior;
            window.setCollectionBehavior((window.collectionBehavior() - NSWindowCollectionBehavior::FullScreenPrimary) | NSWindowCollectionBehavior::FullScreenNone);
        }
        let mut toolbar_delegate = None;
        if !self.toolbar.is_empty() {
            let delegate = chrome::install_toolbar(&window, self.toolbar, self.customizable_toolbar, shell);
            toolbar_delegate = Some(delegate.clone());
            WINDOW_EXTRAS.with_borrow_mut(|e| e.push((window.clone(), delegate)));
        }
        if let Some(style) = self.chrome.toolbar_style {
            window.setToolbarStyle(style.ns());
            // The title-above styles label their items.
            if matches!(style, ToolbarStyle::Expanded | ToolbarStyle::Preference)
                && let Some(t) = window.toolbar()
            {
                t.setDisplayMode(objc2_app_kit::NSToolbarDisplayMode::IconAndLabel);
            }
        }
        if self.chrome.hide_title {
            window.setTitleVisibility(objc2_app_kit::NSWindowTitleVisibility::Hidden);
        }
        window.setTitlebarAppearsTransparent(self.chrome.transparent_title_bar);
        if let Some(color) = self.chrome.background {
            window.setBackgroundColor(Some(&color.ns()));
        }
        let (delegate, undo) = undo::install(mtm, &window);
        WINDOW_EXTRAS.with_borrow_mut(|e| e.push((window.clone(), delegate)));
        // Tab follows the views' positions (top-left to bottom-right).
        window.setAutorecalculatesKeyViewLoop(true);
        if let Some(build) = self.content {
            window.setContentView(Some(&build()));
        }
        // (Taken only for a shell: other windows build it below.)
        if let Some(view) = if shell.0 || shell.1 { self.view.take() } else { None } {
            let make = |v: AnyView| {
                // SAFETY: NSView's designated initialiser.
                let root: Retained<RootView> = unsafe { msg_send![RootView::alloc(mtm), initWithFrame: rect] };
                let node = undo::building(&undo, || batch(|| Node::build_owned(v, &mut Cx::new(Retained::into_super(root.clone())))));
                (root, node)
            };
            let (croot, mut cnode) = make(view);
            {
                let mut r = cnode.meta.responds.borrow_mut();
                r.commands.splice(0..0, std::mem::take(&mut self.responds.commands));
                r.values.splice(0..0, std::mem::take(&mut self.responds.values));
            }
            let controller = objc2_app_kit::NSSplitViewController::new(mtm);
            let item = |root: &RootView, kind: u8| {
                let vc = objc2_app_kit::NSViewController::new(mtm);
                vc.setView(root);
                match kind {
                    0 => objc2_app_kit::NSSplitViewItem::sidebarWithViewController(&vc),
                    2 => objc2_app_kit::NSSplitViewItem::inspectorWithViewController(&vc),
                    _ => objc2_app_kit::NSSplitViewItem::splitViewItemWithViewController(&vc),
                }
            };
            let mut panes: Vec<(Retained<objc2_app_kit::NSSplitViewItem>, Option<Signal<bool>>)> = Vec::new();
            let mut roots = Vec::new();
            // The sidebar and inspector start at their usual widths; the
            // content takes the rest, and gives way first as the window resizes.
            if let Some((v, hidden)) = sidebar {
                let (root, node) = make(v);
                root.setFrameSize(NSSize::new(220.0, rect.size.height));
                let it = item(&root, 0);
                it.setMinimumThickness(180.0);
                it.setMaximumThickness(400.0);
                controller.addSplitViewItem(&it);
                panes.push((it, hidden));
                roots.push((root, node));
            }
            let content_item = item(&croot, 1);
            content_item.setMinimumThickness(300.0);
            content_item.setHoldingPriority(objc2_app_kit::NSLayoutPriorityDefaultLow - 10.0);
            croot.setFrameSize(NSSize::new((rect.size.width - 220.0 * f64::from(u8::from(shell.0)) - 260.0 * f64::from(u8::from(shell.1))).max(300.0), rect.size.height));
            controller.addSplitViewItem(&content_item);
            if let Some((v, hidden)) = inspector {
                let (root, node) = make(v);
                root.setFrameSize(NSSize::new(260.0, rect.size.height));
                let it = item(&root, 2);
                it.setMinimumThickness(220.0);
                it.setMaximumThickness(420.0);
                controller.addSplitViewItem(&it);
                panes.push((it, hidden));
                roots.push((root, node));
            }
            // The hidden signals, both ways: set, the pane slides shut or
            // open; the user's toggling or dragging sets them.
            for (it, hidden) in &panes {
                let Some(hidden) = *hidden else { continue };
                it.setCollapsed(hidden.get_untracked());
                let it = it.clone();
                cnode.own(signal::Effect::new(move || {
                    let h = hidden.get();
                    if it.isCollapsed() != h {
                        // SAFETY: the item's animator proxy takes its setters.
                        unsafe {
                            let a: Retained<objc2::runtime::AnyObject> = msg_send![&*it, animator];
                            let () = msg_send![&*a, setCollapsed: h];
                        }
                    }
                }));
            }
            let followed: Vec<(Retained<objc2_app_kit::NSSplitViewItem>, Signal<bool>)> = panes.iter().filter_map(|(i, h)| h.map(|h| (i.clone(), h))).collect();
            if !followed.is_empty() {
                // SAFETY: AppKit's notification name.
                let name = unsafe { objc2_app_kit::NSSplitViewDidResizeSubviewsNotification };
                let observer = media::Observer::new(name, Some(&controller.splitView()), move || {
                    for (it, h) in &followed {
                        h.set_if_changed(it.isCollapsed());
                    }
                });
                cnode.hold(Rc::new(observer));
            }
            SHELL_ROOTS.with_borrow_mut(|s| s.push((Retained::as_ptr(&window) as usize, Retained::into_super(croot.clone()))));
            SHELL_PANES.with_borrow_mut(|p| p.extend(std::iter::once(&croot).chain(roots.iter().map(|(r, _)| r)).map(|r| Retained::as_ptr(r) as usize)));
            ROOTS.with_borrow_mut(|r| {
                r.push((croot.clone(), cnode));
                r.extend(roots);
            });
            window.setContentViewController(Some(&controller));
            window.setContentSize(rect.size);
            relayout_all();
        }
        if let Some(view) = self.view {
            // SAFETY: NSView's designated initialiser.
            let root: Retained<RootView> = unsafe { msg_send![RootView::alloc(mtm), initWithFrame: rect] };
            // The window's tree owns the signals made while building it.
            let node = undo::building(&undo, || batch(|| Node::build_owned(view, &mut Cx::new(Retained::into_super(root.clone())))));
            {
                // The window's handlers answer after its views'.
                let mut r = node.meta.responds.borrow_mut();
                r.commands.splice(0..0, self.responds.commands);
                r.values.splice(0..0, self.responds.values);
            }
            // Side-by-side panes at the top: the first is a sidebar, and the
            // toolbar's title and items start beside it (once it's in the
            // window).
            let sidebar = match &node.kind {
                view::Kind::Split { split, panes, stacked: false, .. } if panes.len() >= 2 => Some(split.clone()),
                _ => None,
            };
            ROOTS.with_borrow_mut(|r| r.push((root.clone(), node)));
            if let Some(drop) = self.drops {
                // SAFETY: AppKit's pasteboard type constant.
                root.registerForDraggedTypes(&objc2_foundation::NSArray::from_slice(&[unsafe { objc2_app_kit::NSPasteboardTypeFileURL }]));
                DROPS.with_borrow_mut(|d| d.push((Retained::as_ptr(&root) as usize, Rc::new(drop))));
            }
            window.setContentView(Some(&root));
            relayout_all();
            if let (Some(split), Some(delegate)) = (sidebar, toolbar_delegate.as_ref()) {
                chrome::track_sidebar(&window, delegate, &split);
            }
        }
        window.center();
        // In a preview (`cargo ferrokit preview`), every window keeps its
        // place and size across restarts, by its title.
        let preview_name = (self.autosave.is_none() && preview_mode()).then(|| format!("ferrokit preview {}", self.title));
        if let Some(name) = self.autosave.as_ref().or(preview_name.as_ref()) {
            let name = NSString::from_str(name);
            // Restore first: setting the name alone doesn't move the window.
            window.setFrameUsingName(&name);
            window.setFrameAutosaveName(&name);
        }
        window
    }
}

/// What the app hears about its environment.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AppEvent {
    /// Dark Mode or the accent colour changed. Read [`is_dark`] /
    /// [`system_colors`].
    AppearanceChanged,
}

type Handler = Rc<dyn Fn(AppEvent)>;

define_class!(
    // A window's content view: flipped (top-left origin), lays its node out
    // whenever its size changes (same transaction as the window resize).
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitRootView"]
    struct RootView;

    impl RootView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            crate::failure::guard("a window's content view (set_frame_size)", (), || {
                let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
                relayout(self);
            })
        }

        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> objc2_app_kit::NSDragOperation {
            crate::failure::guard("a window's content view (dragging_entered)", objc2_app_kit::NSDragOperation::None, || {
                drop_operation(self, info)
            })
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> objc2_app_kit::NSDragOperation {
            crate::failure::guard("a window's content view (dragging_updated)", objc2_app_kit::NSDragOperation::None, || {
                drop_operation(self, info)
            })
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> objc2::runtime::Bool {
            crate::failure::guard("a window's content view (perform_drag)", objc2::runtime::Bool::NO, || {
                objc2::runtime::Bool::new(perform_drop(self, info))
            })
        }

        // Escape in a popover's content closes it: the popover's window
        // answers `cancelOperation:`, but a plain view never turns the key
        // into it (only text views interpret keys).
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &objc2_app_kit::NSEvent) {
            crate::failure::guard("a window's content view (key_down)", (), || {
                let window = self.window();
                let escape = event.keyCode() == 53;
                match window.filter(|w| escape && popover::is_popover_window(w)) {
                    Some(w) => {
                        let _: () = unsafe { msg_send![&*w, cancelOperation: Option::<&objc2::runtime::AnyObject>::None] };
                    }
                    None => {
                        let _: () = unsafe { msg_send![super(self), keyDown: event] };
                    }
                }
            })
        }
    }
);

thread_local! {
    static ROOTS: UiCell<Vec<(Retained<RootView>, Node)>> = const { UiCell::new(Vec::new()) };
    static IN_LAYOUT: Cell<bool> = const { Cell::new(false) };
    /// Objects windows need for the app's lifetime (toolbar delegates).
    static EXTRAS: UiCell<Vec<Retained<NSObject>>> = const { UiCell::new(Vec::new()) };
    /// Objects a window needs while it's open (its delegate, its toolbar's
    /// delegate); released when it closes.
    static WINDOW_EXTRAS: UiCell<Vec<(Retained<NSWindow>, Retained<NSObject>)>> = const { UiCell::new(Vec::new()) };
    static LAYOUT_US: UiCell<Vec<f64>> = const { UiCell::new(Vec::new()) };
    static LAYOUT_PASSES: Cell<u64> = const { Cell::new(0) };
}

fn relayout(root: &RootView) {
    // At exit the thread's storage is torn down, and dropping the windows'
    // views resizes them: there's nothing to lay out then.
    if ROOTS.try_with(|_| ()).is_err() || IN_LAYOUT.try_with(|_| ()).is_err() || LAYOUT_US.try_with(|_| ()).is_err() || LAYOUT_PASSES.try_with(|_| ()).is_err() {
        return;
    }
    if IN_LAYOUT.with(|l| l.replace(true)) {
        return;
    }
    let t = std::time::Instant::now();
    ROOTS.with_borrow(|roots| {
        for (view, node) in roots {
            if std::ptr::eq(&**view, root) {
                let b = view.bounds().size;
                let window = view.window();
                let scale = window.as_ref().map_or(2.0, |w| w.backingScaleFactor());
                let top = window.as_deref().map_or(0.0, |w| title_bar_inset(w, view));
                layout::mirrored(right_to_left(), b.width, || layout::place_under_bar(node, b.width, b.height, top, &layout::Snap(scale)));
            }
        }
    });
    LAYOUT_US.with_borrow_mut(|v| {
        // Keep the recent passes only: an app resized for hours mustn't grow.
        if v.len() >= 2 * LAYOUT_KEPT {
            v.drain(..LAYOUT_KEPT);
        }
        v.push(t.elapsed().as_secs_f64() * 1e6);
    });
    LAYOUT_PASSES.with(|n| n.set(n.get() + 1));
    IN_LAYOUT.with(|l| l.set(false));
}

/// How far a window with full-size content has its title bar and toolbar
/// over `root`, its content view (0 for other windows).
fn title_bar_inset(window: &NSWindow, root: &RootView) -> f64 {
    if !window.styleMask().contains(NSWindowStyleMask::FullSizeContentView) {
        return 0.0;
    }
    // The window's content view, or a shell's pane (in the split view that
    // is the content view): both run the window's full height, up under
    // the title bar and toolbar.
    let Some(content) = window.contentView() else { return 0.0 };
    let key = root as *const RootView as usize;
    let pane = SHELL_PANES.with_borrow(|p| p.contains(&key));
    if !std::ptr::eq(&*content, &**root) && !pane {
        return 0.0;
    }
    (content.bounds().size.height - window.contentLayoutRect().size.height).max(0.0)
}

thread_local! {
    static FORCE_RTL: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Whether the app lays out right to left: its language is Arabic, Hebrew,
/// Persian or Urdu (the app's localisation and the user's languages), or
/// the `NSForceRightToLeftWritingDirection` default is set. Ferrokit
/// mirrors its layout then, as AppKit mirrors its own.
pub fn right_to_left() -> bool {
    FORCE_RTL.with(Cell::get).unwrap_or_else(|| {
        MainThreadMarker::new().is_some_and(|mtm| NSApplication::sharedApplication(mtm).userInterfaceLayoutDirection() == objc2_app_kit::NSUserInterfaceLayoutDirection::RightToLeft)
    })
}

/// Overrides [`right_to_left`] (`None`: the app's own), for checks.
pub(crate) fn force_right_to_left(on: Option<bool>) {
    FORCE_RTL.with(|f| f.set(on));
}

/// The root view of `window`'s main tree: its content view, or a shell's
/// content pane.
fn main_root(window: &NSWindow) -> Option<Retained<NSView>> {
    let key = window as *const NSWindow as usize;
    SHELL_ROOTS.with_borrow(|s| s.iter().find(|(w, _)| *w == key).map(|(_, r)| r.clone())).or_else(|| window.contentView())
}

/// The root node of `window`'s view tree.
pub(crate) fn root_meta(window: &NSWindow) -> Option<Rc<view::Meta>> {
    let content = main_root(window)?;
    ROOTS.with_borrow(|r| r.iter().find(|(v, _)| std::ptr::eq(&***v as *const NSView, &*content as *const NSView)).map(|(_, n)| n.meta.clone()))
}

/// Tears down the view tree of `window`'s content (its node is dropped:
/// effects stop, native views go).
pub(crate) fn drop_root(window: &NSWindow) {
    let Some(content) = main_root(window) else { return };
    let key = Retained::as_ptr(&content) as usize;
    DROPS.with_borrow_mut(|d| d.retain(|(k, _)| *k != key));
    // The window's roots: its content view's, or each of its shell's panes'.
    let wkey = window as *const NSWindow as usize;
    SHELL_ROOTS.with_borrow_mut(|s| s.retain(|(w, _)| *w != wkey));
    let nodes: Vec<(Retained<RootView>, Node)> = ROOTS.with_borrow_mut(|r| {
        let mut gone = Vec::new();
        let mut i = 0;
        while i < r.len() {
            let mine = std::ptr::eq(&**r[i].0 as *const NSView, &*content as *const NSView) || r[i].0.window().is_some_and(|w| std::ptr::eq(&*w, window));
            if mine {
                gone.push(r.remove(i));
            } else {
                i += 1;
            }
        }
        gone
    });
    SHELL_PANES.with_borrow_mut(|p| p.retain(|k| !nodes.iter().any(|(r, _)| Retained::as_ptr(r) as usize == *k)));
    drop(nodes);
    // The shell's controller goes with its window.
    window.setContentViewController(None);
}

/// Builds `view` (under its own owner) in a new root view of `size`
/// points, laid out like a window's content: for content AppKit hosts in a
/// window of its own (a popover). Returns the root and the view's ideal
/// size. Release it with [`drop_hosted`].
pub(crate) fn host_view(view: view::AnyView) -> (Retained<NSView>, NSSize) {
    let mtm = mtm();
    // SAFETY: NSView's designated initialiser.
    let root: Retained<RootView> = unsafe { msg_send![RootView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::ZERO, NSSize::new(100.0, 100.0))] };
    let node = batch(|| Node::build_owned(view, &mut Cx::new(Retained::into_super(root.clone()))));
    let size = layout::ideal(&node);
    ROOTS.with_borrow_mut(|r| r.push((root.clone(), node)));
    root.setFrameSize(NSSize::new(size.w.ceil(), size.h.ceil()));
    (Retained::into_super(root), NSSize::new(size.w.ceil(), size.h.ceil()))
}

/// The ideal size of a root made by [`host_view`], measured again.
pub(crate) fn hosted_ideal(root: &NSView) -> Option<NSSize> {
    ROOTS.with_borrow(|r| {
        let (_, node) = r.iter().find(|(v, _)| std::ptr::eq(&***v as *const NSView, root as *const NSView))?;
        let s = layout::ideal(node);
        Some(NSSize::new(s.w.ceil(), s.h.ceil()))
    })
}

/// Drops the tree of a root made by [`host_view`].
pub(crate) fn drop_hosted(root: &NSView) {
    // `try_with`: this also runs while the window roots are being dropped
    // at exit.
    let node = ROOTS
        .try_with(|r| {
            let mut r = r.try_borrow_mut().ok()?;
            let i = r.iter().position(|(v, _)| std::ptr::eq(&***v as *const NSView, root as *const NSView))?;
            Some(r.remove(i))
        })
        .ok()
        .flatten();
    drop(node);
}

/// Whether a layout pass is running.
pub(crate) fn in_layout() -> bool {
    IN_LAYOUT.with(Cell::get)
}

/// Lays every window out now (after AppKit resized part of one).
pub(crate) fn relayout_now() {
    relayout_all();
}

fn relayout_all() {
    view::LAYOUT_DIRTY.with(|d| d.set(false));
    let roots: Vec<Retained<RootView>> = ROOTS.with_borrow(|r| r.iter().map(|(v, _)| v.clone()).collect());
    for r in roots {
        relayout(&r);
    }
}

/// How many layout passes [`layout_times_us`] keeps at least.
const LAYOUT_KEPT: usize = 10_000;

/// Microseconds of the recent layout passes, oldest first (diagnostic; at
/// least the last 10,000).
pub fn layout_times_us() -> Vec<f64> {
    LAYOUT_US.with_borrow(Clone::clone)
}

/// Layout passes so far. With [`layout_times_us`], the times of the passes
/// since an earlier count: `let t = layout_times_us(); &t[t.len() - (layout_passes() - n0) as usize..]`.
pub fn layout_passes() -> u64 {
    LAYOUT_PASSES.with(Cell::get)
}

thread_local! {
    /// Windows with a sidebar or inspector shell, and their content pane's
    /// root (their main tree's).
    static SHELL_ROOTS: UiCell<Vec<(usize, Retained<NSView>)>> = const { UiCell::new(Vec::new()) };
    /// The root views of shells' panes (each AppKit wraps in a view of its own).
    static SHELL_PANES: UiCell<Vec<usize>> = const { UiCell::new(Vec::new()) };
    static HANDLERS: UiCell<Vec<Handler>> = const { UiCell::new(Vec::new()) };
    static WINDOWS: UiCell<Vec<Retained<NSWindow>>> = const { UiCell::new(Vec::new()) };
}

/// The application: a name, its windows and the AppKit run loop.
pub struct App {
    name: String,
    windows: Vec<Window>,
    on_launch: Option<Box<dyn FnOnce()>>,
    menus: Vec<(String, Vec<MenuItem>)>,
    settings: Option<Window>,
    documents: Option<Box<dyn FnOnce()>>,
    extras: Vec<MenuBarExtra>,
    menu_bar_only: bool,
    welcome: Option<Rc<dyn Fn() -> Window>>,
    dock_menu: Option<Rc<dyn Fn() -> Vec<MenuItem>>>,
    crash_reports: Option<Box<dyn Fn(CrashReport)>>,
    about: Option<(String, Option<(String, String)>)>,
}

impl App {
    /// An app called `name` (shown in the menu bar).
    pub fn new(name: &str) -> Self {
        failure::install();
        App { name: name.into(), windows: Vec::new(), on_launch: None, menus: Vec::new(), settings: None, documents: None, extras: Vec::new(), menu_bar_only: false, welcome: None, dock_menu: None, crash_reports: None, about: None }
    }

    /// Opens `window` at launch. Call more than once for several windows.
    pub fn window(mut self, window: Window) -> Self {
        self.windows.push(window);
        self
    }

    /// The app's translations: `(language, .strings text)` pairs, e.g.
    /// `("de", include_str!("../locales/de.strings"))`. The languages here
    /// are the ones the app speaks; see [`t`] and the [`l10n`](crate::t)
    /// functions.
    pub fn translations(self, tables: &[(&str, &str)]) -> Self {
        l10n::add_translations(tables);
        self
    }

    /// Adds a menu to the menu bar, between Edit and Window ("View", "Go"…).
    /// A menu titled "File" fills the standard File menu instead, above its
    /// Close item. Items run in a [`batch`]; their enabled and checked
    /// states are evaluated when the menu opens or a key equivalent fires.
    pub fn menu(mut self, title: &str, items: Vec<MenuItem>) -> Self {
        self.menus.push((title.into(), items));
        self
    }

    /// Credits for About {app}: `credits` under the app's name, icon and
    /// version (from the bundle), and `link` (title, URL) after it, e.g.
    /// `.about("Made with Ferrokit.", Some(("ferrokit.dev", "https://…")))`.
    pub fn about(mut self, credits: &str, link: Option<(&str, &str)>) -> Self {
        self.about = Some((credits.into(), link.map(|(t, u)| (t.into(), u.into()))));
        self
    }

    /// The app's Settings window: "Settings…" (⌘,) in the app menu opens it
    /// (built on first use, then brought to the front). It isn't resizable
    /// or listed in the Window menu, and it remembers its position.
    pub fn settings(mut self, window: Window) -> Self {
        self.settings = Some(window);
        self
    }

    /// Makes this a document app for `kind` (see [`DocumentType`]): the File
    /// menu gets New, Open…, Open Recent, Save, Save As… and Revert to
    /// Saved; closing an edited document asks to save it, and quitting
    /// reviews the unsaved ones. With no windows given, the app starts with
    /// an untitled document.
    pub fn documents<T: Clone + PartialEq + 'static>(mut self, kind: DocumentType<T>) -> Self {
        self.documents = Some(Box::new(move || documents::install(kind)));
        self
    }

    /// A welcome window for a document app, shown at launch instead of an
    /// untitled document (as Xcode's is): recent documents, a sample,
    /// Open… `make` builds it each time it opens. It closes when a document
    /// opens, and Window ▸ Welcome to … (⇧⌘1) or [`show_welcome`] brings it
    /// back.
    pub fn welcome(mut self, make: impl Fn() -> Window + 'static) -> Self {
        self.welcome = Some(Rc::new(make));
        self
    }

    /// Adds an item to the right side of the menu bar, with a menu or a
    /// panel (see [`MenuBarExtra`]).
    pub fn menu_bar_extra(mut self, extra: MenuBarExtra) -> Self {
        self.extras.push(extra);
        self
    }

    /// Makes this a menu-bar app: no Dock icon and no menu bar of its own,
    /// only its [`menu_bar_extra`](Self::menu_bar_extra)s (and any windows
    /// it opens).
    pub fn menu_bar_only(mut self) -> Self {
        self.menu_bar_only = true;
        self
    }

    /// Items for the Dock icon's menu (Control-click or a long press on the
    /// icon), above the system's own: "New Window", a recent item. `items`
    /// runs each time the menu opens, so it can read the app's state.
    pub fn dock_menu(mut self, items: impl Fn() -> Vec<MenuItem> + 'static) -> Self {
        self.dock_menu = Some(Rc::new(items));
        self
    }

    /// Crash reports. From now on, a panic Ferrokit doesn't catch (one
    /// outside a control's action, menu item, timer or the like, which
    /// [`on_error`](Self::on_error) hears about and survives; or one on a
    /// background thread) writes a report before the app ends: the app,
    /// its version, macOS, the thread, the message and where, and the
    /// backtrace, in `~/Library/Logs/<app>/Crashes`. At the next launch,
    /// after the windows are on screen, `f` gets each report not yet
    /// [marked sent](CrashReport::mark_sent): ask the user to send it, send
    /// it to a service, or keep it.
    ///
    /// ```no_run
    /// use ferrokit::*;
    ///
    /// let app = App::new("Notes").crash_reports(|report| {
    ///     // Show it, send it, then:
    ///     let _ = report.mark_sent();
    /// });
    /// # let _ = app;
    /// ```
    ///
    /// A crash that isn't a panic (a bad memory access in unsafe code or a
    /// C library) ends the app at once; macOS writes those to
    /// `~/Library/Logs/DiagnosticReports`.
    pub fn crash_reports(mut self, f: impl Fn(CrashReport) + 'static) -> Self {
        crash::install(&self.name);
        self.crash_reports = Some(Box::new(f));
        self
    }

    /// Runs `f` on the first run-loop pass after the windows are on screen.
    pub fn on_launch(mut self, f: impl FnOnce() + 'static) -> Self {
        self.on_launch = Some(Box::new(f));
        self
    }

    /// Handles `command` when nothing nearer the focus does (or no window is
    /// open). See [`Command`].
    pub fn on_command(self, command: Command, action: impl Fn() + 'static) -> Self {
        commands::app_command(command, commands::Handler::new(action, None));
        self
    }

    /// Calls `f` when something fails where AppKit called into the app (an
    /// action, a menu item, a timer, a notification, a sheet's completion):
    /// an Objective-C exception or a Rust panic, caught there so the app
    /// goes on (it's also printed to stderr). Log it, or tell the user.
    ///
    /// ```no_run
    /// use ferrokit::*;
    ///
    /// let app = App::new("Notes").on_error(|failure| eprintln!("send to the crash log: {failure}"));
    /// # let _ = app;
    /// ```
    pub fn on_error(self, f: impl Fn(&Failure) + 'static) -> Self {
        failure::set_hook(f);
        self
    }

    /// Calls `f` for every [`AppEvent`].
    pub fn on_event(self, f: impl Fn(AppEvent) + 'static) -> Self {
        HANDLERS.with_borrow_mut(|h| h.push(Rc::new(f)));
        self
    }

    /// Creates the menu bar and windows, then runs the AppKit run loop. Does
    /// not return while the app is running.
    pub fn run(self) {
        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(if self.menu_bar_only { NSApplicationActivationPolicy::Accessory } else { NSApplicationActivationPolicy::Regular });
        let settings = self.settings.map(|w| {
            SETTINGS.with_borrow_mut(|s| *s = Some(Err(w)));
            Retained::into_super(appkit_views::ActionTarget::new(mtm, || {
                open_settings();
            }))
        });
        let about = self.about.map(|(credits, link)| Retained::into_super(appkit_views::ActionTarget::new(mtm, move || show_about(&credits, link.as_ref()))));
        let is_documents = self.documents.is_some();
        let mut menus = self.menus;
        let mut file_after = Vec::new();
        if let Some(install) = self.documents {
            install();
            let (above, after) = documents::file_items();
            file_after = after;
            match menus.iter_mut().find(|(t, _)| t == "File") {
                Some((_, items)) => {
                    let own = std::mem::take(items);
                    *items = above.into_iter().chain(own).collect();
                }
                None => menus.insert(0, ("File".into(), above)),
            }
        }
        let welcome = self.welcome.filter(|_| is_documents);
        let start_welcome = welcome.is_some() && self.windows.is_empty();
        let start_untitled = is_documents && self.windows.is_empty() && !start_welcome;
        let mut window_items = Vec::new();
        if let Some(make) = welcome {
            documents::set_welcome(make);
            window_items.push(menu_item(t_with("Welcome to {app}", &[("app", &self.name)]), || {
                show_welcome();
            }).key_with("1", &[Modifier::Shift]));
        }
        // Sparkle, when the bundle has it: Check for Updates… in the app menu.
        let updater = objc2::rc::autoreleasepool(|_| updates::start());
        let targets = menu::install(mtm, &self.name, menus, menu::AppTargets { about, settings, updater }, file_after, window_items);
        EXTRAS.with_borrow_mut(|e| e.extend(targets));
        for extra in self.extras {
            status::install(extra, &self.name);
        }
        if let Some(items) = self.dock_menu {
            documents::DOCK_MENU.with_borrow_mut(|d| *d = Some(items));
            documents::install_delegate();
        }
        after_batch(|| {
            motion::run_layout(|| {
                if view::LAYOUT_DIRTY.with(Cell::get) {
                    relayout_all();
                }
            });
        });

        ferrokit_core::observe_appearance();
        ferrokit_core::set_event_sink(|event| match event {
            ferrokit_core::Event::AppearanceChanged => emit(AppEvent::AppearanceChanged),
            // A model queued events: deliver them on the next pass (they
            // arrive inside AppKit callbacks).
            ferrokit_core::Event::Redraw => later(models::drain_all),
            _ => {}
        });

        // Startup runs before the run loop exists, so it needs its own
        // autorelease pool (see `later`).
        objc2::rc::autoreleasepool(|_| {
            for window in self.windows {
                let w = window.open(mtm);
                w.makeKeyAndOrderFront(None);
                WINDOWS.with_borrow_mut(|ws| ws.push(w));
            }
        });
        // What was open at the last quit, if the user keeps windows; else
        // an untitled document, or the welcome window.
        let restored = if start_untitled || start_welcome { objc2::rc::autoreleasepool(|_| documents::restore_open()) } else { 0 };
        if start_untitled && restored == 0 {
            objc2::rc::autoreleasepool(|_| documents::new_launch_document());
        }
        if start_welcome && restored == 0 {
            objc2::rc::autoreleasepool(|_| show_welcome());
        }
        if preview_mode() {
            // A preview restarts on every save: show the windows above the
            // others without taking the keyboard from the editor.
            WINDOWS.with_borrow(|ws| ws.iter().for_each(|w| w.orderFrontRegardless()));
        } else {
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }
        if let Some(f) = self.on_launch {
            let f = RefCell::new(Some(f));
            later(move || {
                if let Some(f) = f.borrow_mut().take() {
                    f();
                }
            });
        }
        if let Some(f) = self.crash_reports {
            // The last run's reports, once the windows are up.
            let dir = crash::folder(&self.name);
            later(move || {
                for report in crash::pending(&dir) {
                    failure::entry("a crash report's handler", (), || f(report));
                }
            });
        }
        app.run();
    }
}

fn emit(event: AppEvent) {
    let handlers: Vec<Handler> = HANDLERS.with_borrow(|h| h.clone());
    for h in handlers {
        h(event.clone());
    }
}

/// The standard About panel, with `credits` and a link under them.
fn show_about(credits: &str, link: Option<&(String, String)>) {
    use objc2_app_kit::{NSFont, NSForegroundColorAttributeName, NSFontAttributeName, NSLinkAttributeName, NSParagraphStyleAttributeName};
    use objc2_foundation::{NSAttributedString, NSMutableAttributedString, NSRange, NSURL};
    let text = match link {
        Some((title, _)) => format!("{credits}\n{title}"),
        None => credits.to_string(),
    };
    let s = NSMutableAttributedString::from_nsstring(&NSString::from_str(&text));
    let all = NSRange::new(0, text.encode_utf16().count());
    let centred = objc2_app_kit::NSMutableParagraphStyle::new();
    centred.setAlignment(objc2_app_kit::NSTextAlignment::Center);
    // SAFETY: AppKit's attribute names with values of their kinds.
    unsafe {
        s.addAttribute_value_range(NSFontAttributeName, &NSFont::systemFontOfSize(11.0), all);
        s.addAttribute_value_range(NSForegroundColorAttributeName, &objc2_app_kit::NSColor::labelColor(), all);
        s.addAttribute_value_range(NSParagraphStyleAttributeName, &centred, all);
        if let Some((title, url)) = link
            && let Some(u) = NSURL::URLWithString(&NSString::from_str(url))
        {
            let start = text.encode_utf16().count() - title.encode_utf16().count();
            s.addAttribute_value_range(NSLinkAttributeName, &u, NSRange::new(start, title.encode_utf16().count()));
        }
    }
    let credits: Retained<NSAttributedString> = Retained::into_super(s);
    let options = objc2_foundation::NSDictionary::from_retained_objects(&[unsafe { objc2_app_kit::NSAboutPanelOptionCredits }], &[Retained::into_super(Retained::into_super(credits))]);
    // SAFETY: the options AppKit documents.
    unsafe { NSApplication::sharedApplication(mtm()).orderFrontStandardAboutPanelWithOptions(&options) };
    ABOUT_SHOWN.with(|a| a.set(a.get() + 1));
}

thread_local! {
    /// How many times About {app} was shown with credits (for checks).
    pub(crate) static ABOUT_SHOWN: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Opens `window` as a new tab of `beside` (the tab bar's + and a "New
/// Tab" command use this).
pub fn open_tab(beside: &NSWindow, window: Window) -> Retained<NSWindow> {
    let w = open(window);
    beside.addTabbedWindow_ordered(&w, objc2_app_kit::NSWindowOrderingMode::Above);
    w.makeKeyAndOrderFront(None);
    w
}

/// Puts `window` into full screen, or takes it out (as the green button
/// and View ▸ Enter Full Screen do).
pub fn toggle_full_screen(window: &NSWindow) {
    window.toggleFullScreen(None);
}

/// Whether `window` is in full screen.
pub fn is_full_screen(window: &NSWindow) -> bool {
    window.styleMask().contains(NSWindowStyleMask::FullScreen)
}

/// The Dock icon's badge: a count or a short word (`Some("3")`), or none.
pub fn set_dock_badge(badge: Option<&str>) {
    let tile = NSApplication::sharedApplication(mtm()).dockTile();
    tile.setBadgeLabel(badge.map(NSString::from_str).as_deref());
}

/// The Dock icon's badge, if any.
pub fn dock_badge() -> Option<String> {
    NSApplication::sharedApplication(mtm()).dockTile().badgeLabel().map(|b| b.to_string())
}

/// Opens the window `make` builds, now, and returns it: the signals `make`
/// creates (a form's fields, a selection) belong to the window and are
/// freed when it closes. Prefer it to [`open`] in an action: a signal made
/// in an action outside any view lives as long as the app.
///
/// ```no_run
/// use ferrokit::*;
///
/// let add = button("New Note", || {
///     open_with(|| {
///         let title = Signal::new(String::new());
///         Window::new("New Note").size(320.0, 120.0).view(text_field(&title).padding(16.0))
///     });
/// });
/// # let _ = add;
/// ```
pub fn open_with(make: impl FnOnce() -> Window) -> Retained<NSWindow> {
    let owner = signal::Owner::new();
    let window = owner.with(make);
    let w = open(window);
    keep_until_closed(&w, owner);
    w
}

thread_local! {
    /// What windows keep until they close (their builders' signals).
    static KEPT: UiCell<Vec<(usize, Box<dyn std::any::Any>)>> = const { UiCell::new(Vec::new()) };
}

/// Keeps `value` until `window` closes (then drops it with the window's
/// views).
pub(crate) fn keep_until_closed(window: &NSWindow, value: impl std::any::Any) {
    KEPT.with_borrow_mut(|k| k.push((window as *const NSWindow as usize, Box::new(value))));
}

/// Opens `window` now (while the app runs) and returns it.
pub fn open(window: Window) -> Retained<NSWindow> {
    let mtm = mtm();
    objc2::rc::autoreleasepool(|_| {
        let w = window.open(mtm);
        w.makeKeyAndOrderFront(None);
        WINDOWS.with_borrow_mut(|ws| ws.push(w.clone()));
        w
    })
}

thread_local! {
    /// The Settings window: not yet built, or open.
    static SETTINGS: UiCell<Option<Result<Retained<NSWindow>, Window>>> = const { UiCell::new(None) };
}

/// Opens the app's Settings window (see [`App::settings`]), or brings it to
/// the front, and returns it. `None` if the app has none.
pub fn open_settings() -> Option<Retained<NSWindow>> {
    let mtm = mtm();
    let pending = SETTINGS.with_borrow_mut(|s| match s.take() {
        Some(Err(w)) => Some(w),
        other => {
            *s = other;
            None
        }
    });
    if let Some(mut w) = pending {
        w.fixed = true;
        if w.autosave.is_none() {
            w.autosave = Some("ferrokit settings".into());
        }
        let window = objc2::rc::autoreleasepool(|_| w.open(mtm));
        window.setExcludedFromWindowsMenu(true);
        SETTINGS.with_borrow_mut(|s| *s = Some(Ok(window)));
    }
    let window = SETTINGS.with_borrow(|s| s.as_ref().and_then(|w| w.as_ref().ok().cloned()))?;
    window.makeKeyAndOrderFront(None);
    Some(window)
}

/// Closes `window` and tears down its views (its node is dropped: effects
/// stop, native views go). The app no longer lists it in [`windows`].
///
/// The close button and ⌘W do the same (except for the Settings window,
/// which is kept and reused).
pub fn close_window(window: &NSWindow) {
    window.close();
    teardown(window);
}

/// Called by the window delegate as `window` closes (close button, ⌘W,
/// `close`): tears it down on the next pass, outside AppKit's close
/// sequence and the delegate's own call.
pub(crate) fn window_will_close(window: &NSWindow) {
    if is_settings(window) {
        return;
    }
    let window = objc2::Message::retain(window);
    later(move || {
        // Reopened since (`makeKeyAndOrderFront` on a kept handle): keep it.
        if !window.isVisible() {
            teardown(&window);
        }
    });
}

fn is_settings(window: &NSWindow) -> bool {
    SETTINGS.with_borrow(|s| matches!(s, Some(Ok(w)) if std::ptr::eq(&**w, window)))
}

/// Drops a closed window's undo history, view tree, delegates and entry in
/// [`windows`]. Running it again does nothing.
pub(crate) fn teardown(window: &NSWindow) {
    if let Some(undo) = window.undoManager() {
        undo.removeAllActions();
    }
    drop_root(window);
    // What the window's builder made goes with it.
    let key = window as *const NSWindow as usize;
    let kept: Vec<_> = KEPT.with_borrow_mut(|k| {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(k).into_iter().partition(|(w, _)| *w == key);
        *k = rest;
        mine
    });
    drop(kept);
    documents::forget(window);
    WINDOWS.with_borrow_mut(|ws| ws.retain(|w| !std::ptr::eq(&**w, window)));
    let extras: Vec<Retained<NSObject>> = WINDOW_EXTRAS.with_borrow_mut(|e| {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(e).into_iter().partition(|(w, _)| std::ptr::eq(&**w, window));
        *e = rest;
        mine.into_iter().map(|(_, x)| x).collect()
    });
    if !extras.is_empty() {
        // The delegates are about to go: AppKit mustn't call them again.
        window.setDelegate(None);
        if let Some(toolbar) = window.toolbar() {
            toolbar.setDelegate(None);
        }
    }
    drop(extras);
}

/// The app's windows, in the order they were opened.
pub fn windows() -> Vec<Retained<NSWindow>> {
    WINDOWS.with_borrow(|w| w.clone())
}

/// Runs `f` on a later pass of the main run loop.
pub fn later(f: impl Fn() + 'static) {
    Native::later(Box::new(f));
}

/// The app's appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    /// Follow the system setting.
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// Sets the app's appearance; every native view follows, and an
/// [`AppEvent::AppearanceChanged`] is delivered.
pub fn set_appearance(appearance: Appearance) {
    let name = match appearance {
        Appearance::System => None,
        // SAFETY: AppKit's appearance name constants.
        Appearance::Light => Some(unsafe { objc2_app_kit::NSAppearanceNameAqua }),
        Appearance::Dark => Some(unsafe { objc2_app_kit::NSAppearanceNameDarkAqua }),
    };
    let appearance = name.and_then(objc2_app_kit::NSAppearance::appearanceNamed);
    NSApplication::sharedApplication(mtm()).setAppearance(appearance.as_deref());
}

/// Ends the app (as ⌘Q does), on the next pass of the run loop. Quitting
/// can wait for answers (a document app asks about unsaved changes) inside
/// a nested run loop; starting it outside the caller's batch keeps the UI
/// updating meanwhile.
pub fn quit() {
    later(|| NSApplication::sharedApplication(mtm()).terminate(None));
}

type Continuation = Box<dyn FnOnce(Result<Box<dyn Any + Send>, String>)>;

thread_local! {
    static CONTINUATIONS: UiCell<HashMap<u64, Continuation>> = UiCell::new(HashMap::new());
    static NEXT_TASK: Cell<u64> = const { Cell::new(0) };
}

/// Runs `work` on a background thread, then `then` with its result on the
/// main thread, inside a [`batch`]. Call from the main thread.
///
/// If `work` panics, `then` doesn't run: the panic is reported like any
/// other failure (see [`App::on_error`], as "a background task") and the
/// app goes on. Return a `Result` from `work` for errors you expect.
pub fn spawn<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, then: impl FnOnce(T) + 'static) {
    spawn_outcome(work, move |outcome| {
        if let Ok(result) = outcome {
            then(result);
        }
    });
}

/// [`spawn`], with `then` always run: with the result, or with the message
/// of the panic that ended `work` (already reported).
pub(crate) fn spawn_outcome<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, then: impl FnOnce(Result<T, String>) + 'static) {
    let _ = mtm();
    let id = NEXT_TASK.with(|n| n.replace(n.get() + 1));
    CONTINUATIONS.with_borrow_mut(|c| {
        c.insert(
            id,
            Box::new(move |outcome: Result<Box<dyn Any + Send>, String>| {
                then(outcome.map(|result| *result.downcast::<T>().expect("spawned task result type")));
            }),
        )
    });
    std::thread::spawn(move || {
        // A panic here is caught and reported, not a crash.
        let outcome: Result<Box<dyn Any + Send>, String> = crash::catching(|| std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)))
            .map(|result| Box::new(result) as Box<dyn Any + Send>)
            .map_err(|payload| payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into()));
        // On the main thread the continuation (a main-thread value) is
        // looked up by id: it runs, and goes, however the work ended.
        Native::on_main(Box::new(move || {
            if let Err(message) = &outcome {
                failure::report(Failure { place: "a background task", message: message.clone(), exception: false });
            }
            if let Some(then) = CONTINUATIONS.with_borrow_mut(|c| c.remove(&id)) {
                batch(|| then(outcome));
            }
        }));
    });
}

thread_local! {
    /// The progress signal of each job started by [`spawn_with_progress`].
    static PROGRESS: UiCell<HashMap<u64, Signal<Option<f64>>>> = UiCell::new(HashMap::new());
}

/// What a job started by [`spawn_with_progress`] reports its progress
/// through, from its background thread.
pub struct Reporter {
    id: u64,
    /// The latest fraction's bits, and whether an update is on its way to
    /// the main thread (so a job reporting often posts only now and then).
    latest: std::sync::Arc<(std::sync::atomic::AtomicU64, std::sync::atomic::AtomicBool)>,
}

impl Reporter {
    /// How far the job is, from 0 to 1 (clamped). Cheap to call often: the
    /// signal gets the latest value on the main thread's next pass.
    pub fn report(&self, fraction: f64) {
        use std::sync::atomic::Ordering;
        let (latest, pending) = &*self.latest;
        latest.store(fraction.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        if pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let (id, shared) = (self.id, self.latest.clone());
        Native::on_main(Box::new(move || {
            let (latest, pending) = &*shared;
            pending.store(false, Ordering::Release);
            let value = f64::from_bits(latest.load(Ordering::Relaxed));
            if let Some(signal) = PROGRESS.with_borrow(|p| p.get(&id).copied()) {
                batch(|| signal.set_if_changed(Some(value)));
            }
        }));
    }
}

/// [`spawn`] for a long job that reports how far it is: `progress` reads
/// `Some(0.0)` at once, the fractions `work` reports through its
/// [`Reporter`] as it goes, and `None` once it's done (just before `then`
/// runs; also if `work` panics, when `then` doesn't run). Show it with
/// [`progress`]:
///
/// ```no_run
/// # use ferrokit::*;
/// let done = Signal::new(None);
/// spawn_with_progress(&done, |r| {
///     for i in 0..100 {
///         // … a hundredth of the work …
///         r.report((i + 1) as f64 / 100.0);
///     }
///     42
/// }, |answer| println!("{answer}"));
/// let bar = progress(move || done.get());
/// ```
pub fn spawn_with_progress<T: Send + 'static>(progress: &Signal<Option<f64>>, work: impl FnOnce(&Reporter) -> T + Send + 'static, then: impl FnOnce(T) + 'static) {
    let id = NEXT_TASK.with(|n| n.replace(n.get() + 1));
    PROGRESS.with_borrow_mut(|p| p.insert(id, *progress));
    progress.set(Some(0.0));
    let reporter = Reporter { id, latest: std::sync::Arc::new((std::sync::atomic::AtomicU64::new(0), std::sync::atomic::AtomicBool::new(false))) };
    let signal = *progress;
    spawn_outcome(
        move || work(&reporter),
        move |outcome| {
            // Also when the work panicked: the job is over.
            PROGRESS.with_borrow_mut(|p| p.remove(&id));
            signal.set(None);
            if let Ok(result) = outcome {
                then(result);
            }
        },
    );
}

/// The guide: how to build an app with Ferrokit, chapter by chapter. Its
/// examples are compiled as doc tests, so they match the API. Start with
/// [chapter 1](guide::ch01_getting_started); [`API.md`](https://github.com/lhassa8/ferrokit/blob/main/crates/ferrokit/API.md)
/// lists every public item on one page.
///
/// - [Getting started](guide::ch01_getting_started): an app, a window, a button.
/// - [State](guide::ch02_state): signals, memos, effects, batching, background work.
/// - [Views and layout](guide::ch03_views_and_layout): stacks, grids, modifiers, lists and identity.
/// - [Controls and forms](guide::ch04_controls_and_forms): fields, toggles, pickers, sliders, and naming them for VoiceOver.
/// - [Tables, outlines and editors](guide::ch05_tables_outlines_editors): the deep widgets.
/// - [The app shell](guide::ch06_app_shell): menus, commands, toolbars, split views, sheets, panels, Settings, restoration and undo.
/// - [Live data](guide::ch07_live_data): per-frame updates and growing tables.
/// - [Testing and shipping](guide::ch08_testing_and_shipping): self-checks, accessibility audits, packaging and notarisation.
/// - [Styling](guide::ch09_styling): text styles, colours, cards, shadows, symbols and pictures.
/// - [Materials and windows](guide::ch10_materials_and_windows): sidebar and panel materials, Liquid Glass, title bar and toolbar styles.
/// - [Motion](guide::ch11_motion): animated changes, transitions, hover and click, and Reduce Motion.
/// - [More controls and charts](guide::ch12_more_controls_and_charts): tabs, radio groups, gauges, token fields, tooltips, and charts.
/// - [Web, PDF, video and services](guide::ch13_web_pdf_video_and_services): web pages, PDFs, video, Share, printing, notifications, menu-bar extras.
/// - [Custom native views](guide::ch14_custom_native_views): any `NSView` as a Ferrokit view (a map, a view of your own drawing).
#[cfg(any(doc, doctest))]
pub mod guide {
    // The chapters' links name the crate's items.
    #[allow(unused_imports)]
    use crate::*;
    #[cfg(doctest)]
    #[doc = include_str!("../README.md")]
    mod readme {}
    #[doc = include_str!("../guide/01-getting-started.md")]
    pub mod ch01_getting_started {}
    #[doc = include_str!("../guide/02-state.md")]
    pub mod ch02_state {}
    #[doc = include_str!("../guide/03-views-and-layout.md")]
    pub mod ch03_views_and_layout {}
    #[doc = include_str!("../guide/04-controls-and-forms.md")]
    pub mod ch04_controls_and_forms {}
    #[doc = include_str!("../guide/05-tables-outlines-editors.md")]
    pub mod ch05_tables_outlines_editors {}
    #[doc = include_str!("../guide/06-app-shell.md")]
    pub mod ch06_app_shell {}
    #[doc = include_str!("../guide/07-live-data.md")]
    pub mod ch07_live_data {}
    #[doc = include_str!("../guide/08-testing-and-shipping.md")]
    pub mod ch08_testing_and_shipping {}
    #[doc = include_str!("../guide/09-styling.md")]
    pub mod ch09_styling {}
    #[doc = include_str!("../guide/10-materials-and-windows.md")]
    pub mod ch10_materials_and_windows {}
    #[doc = include_str!("../guide/11-motion.md")]
    pub mod ch11_motion {}
    #[doc = include_str!("../guide/12-more-controls-and-charts.md")]
    pub mod ch12_more_controls_and_charts {}
    #[doc = include_str!("../guide/13-web-pdf-video-and-services.md")]
    pub mod ch13_web_pdf_video_and_services {}
    #[doc = include_str!("../guide/14-custom-native-views.md")]
    pub mod ch14_custom_native_views {}
}

thread_local! {
    /// Preview mode, overridden by checks (`testing::set_preview_mode`).
    static PREVIEW: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Whether the app runs under `cargo ferrokit preview` (`FERROKIT_PREVIEW=1`):
/// windows keep their frames across restarts and launches don't take the
/// focus.
pub(crate) fn preview_mode() -> bool {
    PREVIEW.with(|p| match p.get() {
        Some(on) => on,
        None => {
            let on = std::env::var("FERROKIT_PREVIEW").is_ok_and(|v| v == "1");
            p.set(Some(on));
            on
        }
    })
}

pub(crate) fn set_preview_mode(on: bool) {
    PREVIEW.with(|p| p.set(Some(on)));
}
