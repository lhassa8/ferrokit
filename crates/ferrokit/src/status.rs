//! Menu-bar extras: an icon (and title) in the menu bar's right side, with a
//! menu or a panel of the app's own views.

use ferrokit_core::kept::{UiCell, UiLocal};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::sel;
use objc2_app_kit::{NSImage, NSPopover, NSPopoverBehavior, NSStatusBar, NSStatusItem, NSVariableStatusItemLength, NSView, NSViewController};
use objc2_foundation::{NSRectEdge, NSString};

use crate::appkit_views::ActionTarget;
use crate::chrome::MenuItem;
use crate::signal::Effect;
use crate::view::{AnyView, View};

enum Content {
    None,
    Menu(Vec<MenuItem>),
    Panel(AnyView),
}

/// An item in the menu bar (see [`App::menu_bar_extra`](crate::App::menu_bar_extra)):
/// SwiftUI's `MenuBarExtra`.
///
/// ```no_run
/// use ferrokit::*;
///
/// let done = Signal::new(3);
/// App::new("Timer")
///     .menu_bar_extra(
///         MenuBarExtra::new("timer")
///             .title(move || format!("{} done", done.get()))
///             .menu(vec![menu_item("Reset", move || done.set(0)), menu_separator(), menu_item("Quit", quit)]),
///     )
///     .run();
/// ```
pub struct MenuBarExtra {
    symbol: String,
    label: String,
    title: Option<Box<dyn Fn() -> String>>,
    content: Content,
}

impl MenuBarExtra {
    /// An extra showing the SF Symbol `symbol` (drawn as a template, so it
    /// follows the menu bar's colours). Give it a [`menu`](Self::menu) or a
    /// [`panel`](Self::panel).
    pub fn new(symbol: &str) -> Self {
        MenuBarExtra { symbol: symbol.into(), label: String::new(), title: None, content: Content::None }
    }

    /// Text beside the icon, updated when the signals it reads change: a
    /// timer, a count, a temperature.
    pub fn title(mut self, title: impl Fn() -> String + 'static) -> Self {
        self.title = Some(Box::new(title));
        self
    }

    /// The name VoiceOver reads for it (the app's name if not set).
    pub fn accessibility_label(mut self, label: &str) -> Self {
        self.label = label.into();
        self
    }

    /// A menu shown when it's clicked.
    pub fn menu(mut self, items: Vec<MenuItem>) -> Self {
        self.content = Content::Menu(items);
        self
    }

    /// A panel of the app's own views shown under it when it's clicked, as
    /// the Wi-Fi and Sound panels show. It's built once and keeps its
    /// state; clicking elsewhere closes it.
    pub fn panel(mut self, view: impl View) -> Self {
        self.content = Content::Panel(AnyView::new(view));
        self
    }
}

/// What keeps an extra alive.
pub(crate) struct Installed {
    pub(crate) item: Retained<NSStatusItem>,
    pub(crate) popover: Option<Retained<NSPopover>>,
    _keep: Vec<Retained<NSObject>>,
    _effect: Option<Effect>,
    _root: Option<Retained<NSView>>,
}

thread_local! {
    pub(crate) static EXTRAS: UiCell<Vec<Rc<Installed>>> = const { UiCell::new(Vec::new()) };
}

/// Puts `extra` in the menu bar.
pub(crate) fn install(extra: MenuBarExtra, app_name: &str) {
    let mtm = crate::mtm();
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    let Some(button) = item.button(mtm) else { return };
    let label = if extra.label.is_empty() { app_name.to_string() } else { extra.label };
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(&extra.symbol), Some(&NSString::from_str(&label))) {
        image.setTemplate(true);
        button.setImage(Some(&image));
    }
    objc2_app_kit::NSAccessibility::setAccessibilityLabel(&*button, Some(&NSString::from_str(&label)));
    let effect = extra.title.map(|title| {
        let b = button.clone();
        Effect::new(move || {
            let t = title();
            b.setTitle(&NSString::from_str(&if t.is_empty() { t } else { format!(" {t}") }));
        })
    });
    let mut keep = Vec::new();
    let mut popover = None;
    let mut root = None;
    match extra.content {
        Content::None => {}
        Content::Menu(items) => {
            let (menu, targets) = crate::chrome::build_menu(mtm, items);
            item.setMenu(Some(&menu));
            keep.extend(targets);
        }
        Content::Panel(view) => {
            let (r, size) = crate::host_view(view);
            let p = NSPopover::new(mtm);
            p.setBehavior(NSPopoverBehavior::Transient);
            p.setAnimates(true);
            let controller = NSViewController::new(mtm);
            controller.setView(&r);
            p.setContentViewController(Some(&controller));
            p.setContentSize(size);
            let (p2, b, r2) = (p.clone(), button.clone(), r.clone());
            let target = ActionTarget::new(mtm, move || {
                if p2.isShown() {
                    // SAFETY: closing a popover on the main thread.
                    unsafe { p2.performClose(None) };
                } else {
                    // Measured again: the panel is built once, and its
                    // content (a list) may have grown or shrunk since.
                    if let Some(size) = crate::hosted_ideal(&r2) {
                        p2.setContentSize(size);
                    }
                    p2.showRelativeToRect_ofView_preferredEdge(b.bounds(), &b, NSRectEdge::MinY);
                }
            });
            // SAFETY: the target answers fire:; it's kept with the extra.
            unsafe {
                button.setTarget(Some(&target));
                button.setAction(Some(sel!(fire:)));
            }
            keep.push(Retained::into_super(target));
            popover = Some(p);
            root = Some(r);
        }
    }
    EXTRAS.with_borrow_mut(|e| e.push(Rc::new(Installed { item, popover, _keep: keep, _effect: effect, _root: root })));
}
