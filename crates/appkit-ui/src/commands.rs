//! Commands and focus: menu items and toolbar buttons that act on whatever
//! has focus.
//!
//! A [`Command`] is a name. Views, windows and the app handle it
//! ([`ViewExt::on_command`](crate::ViewExt::on_command),
//! [`Window::on_command`](crate::Window::on_command),
//! [`App::on_command`](crate::App::on_command)). When a [`menu_command`]
//! fires, the handler nearest the focus runs: the focused view's, then its
//! ancestors' up to the window's, then the app's. The item is enabled only
//! while one of them handles it, as in AppKit's responder chain. Values
//! work the same way ([`ViewExt::focused_value`](crate::ViewExt::focused_value),
//! [`focused_value`]).

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSView, NSWindow};

use crate::chrome::{MenuItem, ToolbarItem, menu_item, toolbar_button};
use crate::signal::batch;
use crate::view::Meta;

/// A named action that menus, toolbars and code invoke, and views, windows
/// or the app handle. Declare one as a constant:
///
/// ```
/// use appkit_ui::Command;
/// const DELETE_NOTE: Command = Command::new("delete-note");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Command(&'static str);

impl Command {
    /// A command named `name` (names identify commands, so keep them unique).
    pub const fn new(name: &'static str) -> Self {
        Command(name)
    }

    /// The command's name.
    pub fn name(&self) -> &'static str {
        self.0
    }
}

/// What a view, window or the app does for a command.
pub(crate) struct Handler {
    action: Box<dyn Fn()>,
    enabled: Option<Box<dyn Fn() -> bool>>,
}

impl Handler {
    pub(crate) fn new(action: impl Fn() + 'static, enabled: Option<Box<dyn Fn() -> bool>>) -> Rc<Handler> {
        // Signals the action makes belong to the tree it was declared in.
        Rc::new(Handler { action: Box::new(crate::signal::with_current_owner(action)), enabled })
    }

    fn enabled(&self) -> bool {
        self.enabled.as_ref().is_none_or(|f| f())
    }
}

/// Command handlers and focused values of one node (kept on its `Meta`).
#[derive(Default)]
pub(crate) struct Responds {
    pub(crate) commands: Vec<(Command, Rc<Handler>)>,
    pub(crate) values: Vec<(&'static str, Rc<dyn Any>)>,
}

thread_local! {
    /// Leaf native views to their nodes, for finding the focused node.
    static LEAVES: RefCell<HashMap<usize, Weak<Meta>>> = RefCell::new(HashMap::new());
    static APP: RefCell<Responds> = RefCell::new(Responds::default());
}

pub(crate) fn register_leaf(key: usize, meta: &Rc<Meta>) {
    LEAVES.with_borrow_mut(|l| l.insert(key, Rc::downgrade(meta)));
}

pub(crate) fn unregister_leaf(key: usize) {
    LEAVES.with_borrow_mut(|l| l.remove(&key));
}

pub(crate) fn app_command(command: Command, handler: Rc<Handler>) {
    APP.with_borrow_mut(|a| a.commands.push((command, handler)));
}

/// The window commands go to: the key window, else the main window, else
/// the most recently opened visible one (an inactive app has neither).
pub fn focused_window() -> Option<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(crate::mtm());
    app.keyWindow().or_else(|| app.mainWindow()).or_else(|| crate::windows().into_iter().rev().find(|w| w.isVisible()))
}

/// The node with focus in `window` (the window's root if no view of ours has
/// it), then its ancestors.
fn chain(window: &NSWindow) -> Vec<Rc<Meta>> {
    let mut start = None;
    let mut view: Option<Retained<NSView>> = window.firstResponder().and_then(|r| r.downcast::<NSView>().ok());
    while let Some(v) = view {
        let key = Retained::as_ptr(&v) as usize;
        if let Some(meta) = LEAVES.with_borrow(|l| l.get(&key).and_then(Weak::upgrade)) {
            start = Some(meta);
            break;
        }
        // SAFETY: reading the view hierarchy on the main thread.
        view = unsafe { v.superview() };
    }
    let mut out = Vec::new();
    let mut cur = start.or_else(|| crate::root_meta(window));
    while let Some(m) = cur {
        cur = m.parent();
        out.push(m);
    }
    out
}

/// The handler nearest the focus in `window` (or the app's).
fn resolve(window: Option<&NSWindow>, command: Command) -> Option<Rc<Handler>> {
    let find = |r: &Responds| r.commands.iter().rev().find(|(c, _)| *c == command).map(|(_, h)| h.clone());
    let near = window.and_then(|w| chain(w).iter().find_map(|m| find(&m.responds.borrow())));
    near.or_else(|| APP.with_borrow(find))
}

/// Runs `command` in the focused window (see [`focused_window`]); false if
/// nothing handles it there or its handler is disabled.
pub fn perform(command: Command) -> bool {
    perform_in(focused_window().as_deref(), command)
}

/// Whether [`perform`] would run something now.
pub fn can_perform(command: Command) -> bool {
    can_perform_in(focused_window().as_deref(), command)
}

/// Runs `command` as if `window` were focused (`None`: the app's handler).
pub fn perform_in(window: Option<&NSWindow>, command: Command) -> bool {
    match resolve(window, command) {
        Some(h) if h.enabled() => {
            batch(|| (h.action)());
            true
        }
        _ => false,
    }
}

/// Whether [`perform_in`] would run something.
pub fn can_perform_in(window: Option<&NSWindow>, command: Command) -> bool {
    resolve(window, command).is_some_and(|h| h.enabled())
}

/// The value published under `name` nearest the focus (see
/// [`ViewExt::focused_value`](crate::ViewExt::focused_value)): what a menu
/// command acts on, such as the selected note of the focused list. `None` if
/// nothing near the focus publishes one of type `T`.
pub fn focused_value<T: Clone + 'static>(name: &str) -> Option<T> {
    focused_value_in(focused_window().as_deref()?, name)
}

/// [`focused_value`] as if `window` were focused.
pub fn focused_value_in<T: Clone + 'static>(window: &NSWindow, name: &str) -> Option<T> {
    chain(window).iter().find_map(|m| {
        let r = m.responds.borrow();
        r.values.iter().rev().find(|(n, _)| *n == name).and_then(|(_, v)| v.downcast_ref::<T>().cloned())
    })
}

/// A menu item that performs `command` on whatever has focus, enabled only
/// while something handles it. Add `.key(..)` for a shortcut.
pub fn menu_command(title: &str, command: Command) -> MenuItem {
    menu_item(title, move || {
        perform(command);
    })
    .enabled_when(move || can_perform(command))
}

/// A toolbar button that performs `command` on its window's focus,
/// disabled while nothing there handles it.
pub fn toolbar_command(id: &str, label: &str, symbol: &str, command: Command) -> ToolbarItem {
    toolbar_button(id, label, symbol, move || {
        perform(command);
    })
    .disabled(move || !can_perform(command))
}
