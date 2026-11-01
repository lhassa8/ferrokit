//! Window furniture: split views, the toolbar, context menus and menu-bar
//! menus.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSResponder, NSSearchToolbarItem, NSSplitView, NSSplitViewDelegate, NSSplitViewDividerStyle, NSToolbar,
    NSToolbarDelegate, NSToolbarItem, NSView, NSWindow, NSWindowToolbarStyle,
};
use objc2_foundation::{NSArray, NSNotification, NSSize, NSString, NSUserDefaults};

use crate::controls::FieldDelegate;
use crate::signal::{Effect, Signal, batch};
use crate::view::{AnyView, Cx, Kind, Meta, Node, View};

// ---- split view ----

define_class!(
    // One pane of a split view: flipped, and it re-lays out its content when
    // AppKit resizes it (window resize, divider drag).
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitSplitPane"]
    #[ivars = RefCell<Weak<Meta>>]
    pub(crate) struct SplitPane;

    impl SplitPane {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            if crate::in_layout() {
                return; // the layout pass in progress places the content next
            }
            if let Some(meta) = self.ivars().borrow().upgrade() {
                meta.mark();
                crate::relayout_now();
            }
        }
    }
);

define_class!(
    // Keeps the first pane's width when the split view resizes (holding
    // priorities only apply under Auto Layout; this layout sets frames), and
    // saves the divider position the user chose under its defaults key.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitSplitDelegate"]
    #[ivars = SplitInfo]
    pub(crate) struct SplitDelegate;

    unsafe impl NSObjectProtocol for SplitDelegate {}

    unsafe impl NSSplitViewDelegate for SplitDelegate {
        #[unsafe(method(splitView:shouldAdjustSizeOfSubview:))]
        fn should_adjust(&self, split: &NSSplitView, view: &NSView) -> bool {
            // The first pane (a sidebar) and collapsible ones (an inspector)
            // keep their size; the others take the change.
            let panes = split.arrangedSubviews();
            let index = panes.iter().position(|p| std::ptr::eq(&*p, view));
            panes.count() < 2 || !index.is_some_and(|i| i == 0 || self.ivars().collapsible.iter().any(|(c, _)| *c == i))
        }

        #[unsafe(method(splitView:canCollapseSubview:))]
        fn can_collapse(&self, split: &NSSplitView, view: &NSView) -> bool {
            let index = split.arrangedSubviews().iter().position(|p| std::ptr::eq(&*p, view));
            index.is_some_and(|i| self.ivars().collapsible.iter().any(|(c, _)| *c == i))
        }

        #[unsafe(method(splitViewDidResizeSubviews:))]
        fn did_resize(&self, notification: &NSNotification) {
            let Some(split) = notification.object().and_then(|o| o.downcast::<NSSplitView>().ok()) else { return };
            if let Some(f) = &*self.ivars().on_resize.borrow() {
                f(&split);
            }
            // A pane the user dragged shut (or open) updates its signal.
            let panes = split.arrangedSubviews();
            for (i, collapsed) in self.ivars().collapsible.iter().filter(|_| !self.ivars().applying.get()) {
                if let Some(pane) = (*i < panes.count()).then(|| panes.objectAtIndex(*i)) {
                    let size = if split.isVertical() { pane.frame().size.width } else { pane.frame().size.height };
                    // Zero size counts only outside layout: a pane is zero
                    // wide in the first tiling, before its size is set.
                    collapsed.set_if_changed(split.isSubviewCollapsed(&pane) || pane.isHidden() || (size < 1.0 && !crate::in_layout()));
                }
            }
            // Layout passes place the divider themselves; only a drag (or a
            // programmatic move outside layout) is the user's choice.
            let (Some(key), false) = (&self.ivars().key, crate::in_layout()) else { return };
            if let Some(first) = panes.firstObject().filter(|f| !f.isHidden())
            {
                let size = first.frame().size;
                NSUserDefaults::standardUserDefaults().setDouble_forKey(if split.isVertical() { size.width } else { size.height }, key);
            }
        }
    }
);

pub(crate) struct SplitInfo {
    /// The defaults key the first divider's position is saved under.
    key: Option<Retained<NSString>>,
    /// Pane indices that collapse, with their signals.
    collapsible: Vec<(usize, Signal<bool>)>,
    /// Set while a signal's change is applied (the resizes it causes aren't
    /// the user's).
    applying: std::cell::Cell<bool>,
    /// Called as the panes resize (the toolbar's title follows a sidebar).
    on_resize: RefCell<Option<OnResize>>,
}

type OnResize = Box<dyn Fn(&NSSplitView)>;

/// Side-by-side panes with draggable dividers (`NSSplitView`). The first
/// pane keeps its width when the window resizes (a sidebar); the others
/// share the change. [`stacked`](Self::stacked) puts them top to bottom.
/// [`collapsible`](Self::collapsible) panes (a sidebar, an inspector) hide
/// and show with a signal, or when dragged shut.
pub struct Split {
    panes: Vec<AnyView>,
    position: Option<f64>,
    remember: Option<String>,
    stacked: bool,
    collapsible: Vec<(usize, Signal<bool>)>,
    trailing: Option<f64>,
}

impl Split {
    /// Pane `index` collapses while `collapsed` is true, and can be dragged
    /// shut (which sets it). Like the first pane, it keeps its size when
    /// the window resizes. For a sidebar (`0`) or an inspector (the last):
    ///
    /// ```no_run
    /// use ferrokit::*;
    ///
    /// let (no_sidebar, no_inspector) = (Signal::new(false), Signal::new(true));
    /// let view = split((text("Sidebar"), text("Content"), text("Inspector")))
    ///     .collapsible(0, &no_sidebar)
    ///     .collapsible(2, &no_inspector)
    ///     .position(200.0)
    ///     .trailing(260.0);
    /// let menu = menu_item("Toggle Sidebar", move || no_sidebar.update(|c| *c = !*c));
    /// # let _ = (view, menu);
    /// ```
    pub fn collapsible(mut self, index: usize, collapsed: &Signal<bool>) -> Self {
        self.collapsible.push((index, *collapsed));
        self
    }

    /// The last pane's initial width (height when stacked), in points: an
    /// inspector's.
    pub fn trailing(mut self, points: f64) -> Self {
        self.trailing = Some(points);
        self
    }

    /// Panes top to bottom, with dividers across (an editor over its
    /// results). The first pane keeps its height when the window resizes.
    pub fn stacked(mut self) -> Self {
        self.stacked = true;
        self
    }

    /// Initial position of the first divider, in points from the left (from
    /// the top when [`stacked`](Self::stacked)).
    pub fn position(mut self, points: f64) -> Self {
        self.position = Some(points);
        self
    }

    /// Saves the divider position the user drags to under `name` (in user
    /// defaults) and starts there next time, instead of at
    /// [`position`](Self::position).
    pub fn remember(mut self, name: &str) -> Self {
        self.remember = Some(name.into());
        self
    }
}

/// The defaults key a remembered split's position is kept under.
pub(crate) fn split_key(name: &str) -> Retained<NSString> {
    NSString::from_str(&format!("ferrokit split {name}"))
}

impl View for Split {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        let split = NSSplitView::new(mtm);
        split.setVertical(!self.stacked);
        split.setDividerStyle(NSSplitViewDividerStyle::Thin);
        let key = self.remember.as_deref().map(split_key);
        let defaults = NSUserDefaults::standardUserDefaults();
        let saved = key.as_ref().filter(|k| defaults.objectForKey(k).is_some()).map(|k| defaults.doubleForKey(k));
        let delegate = SplitDelegate::alloc(mtm).set_ivars(SplitInfo { key, collapsible: self.collapsible.clone(), applying: std::cell::Cell::new(false), on_resize: RefCell::default() });
        // SAFETY: NSObject's designated initialiser.
        let delegate: Retained<SplitDelegate> = unsafe { msg_send![super(delegate), init] };
        // Weak in AppKit; the node keeps the delegate.
        split.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        cx.container.addSubview(&split);
        let (mut panes, mut typed) = (Vec::new(), Vec::new());
        for child in self.panes {
            let pane = SplitPane::alloc(mtm).set_ivars(RefCell::new(Weak::new()));
            // SAFETY: NSView's designated initialiser.
            let pane: Retained<SplitPane> = unsafe { msg_send![super(pane), init] };
            split.addArrangedSubview(&pane);
            let node = child.build_node(&mut Cx::new(Retained::into_super(pane.clone())));
            panes.push((Retained::into_super(pane.clone()), node));
            typed.push(pane);
        }
        let mut node = Node::new(Kind::Split {
            split,
            panes,
            position: std::cell::Cell::new(saved.map(|p| (p, false)).or(self.position.map(|p| (p, true)))),
            trailing: std::cell::Cell::new(self.trailing),
            stacked: self.stacked,
        });
        node.keep(Retained::into_super(delegate.clone()));
        // Collapsible panes follow their signals, reopening at the size they
        // had (or the size given for them).
        let count = typed.len();
        for (i, collapsed) in self.collapsible {
            let Some(pane) = typed.get(i).cloned() else { continue };
            let meta = Rc::downgrade(&node.meta);
            let given = if i == 0 { self.position } else if i + 1 == count { self.trailing } else { None };
            let size = std::cell::Cell::new(given.unwrap_or(220.0));
            let stacked = self.stacked;
            let delegate = delegate.clone();
            node.own(Effect::new(move || {
                let hide = collapsed.get();
                if pane.isHidden() != hide {
                    delegate.ivars().applying.set(true);
                    let along = |r: objc2_foundation::NSRect| if stacked { r.size.height } else { r.size.width };
                    if hide && along(pane.frame()) > 20.0 {
                        size.set(along(pane.frame()));
                    }
                    pane.setHidden(hide);
                    // SAFETY: reading the view hierarchy on the main thread.
                    if let Some(split) = unsafe { pane.superview() }.and_then(|s| s.downcast::<NSSplitView>().ok()) {
                        split.adjustSubviews();
                        if !hide && along(pane.frame()) < 20.0 {
                            if i == 0 {
                                split.setPosition_ofDividerAtIndex(size.get(), 0);
                            } else if i + 1 == count {
                                let total = along(split.bounds());
                                split.setPosition_ofDividerAtIndex(total - size.get() - split.dividerThickness(), (count - 2) as isize);
                            }
                        }
                    }
                    delegate.ivars().applying.set(false);
                    // The panes that grew or shrank keep a stale strip where
                    // their old edge was (a line across a table) until they're
                    // laid out again: AppKit's views inside (a scroll view's
                    // background) don't follow the new size by themselves.
                    // SAFETY: reading the view hierarchy on the main thread.
                    if let Some(split) = unsafe { pane.superview() } {
                        relayout_all(&split);
                    }
                    if let Some(meta) = meta.upgrade() {
                        meta.mark();
                    }
                    crate::view::invalidate_layout();
                }
            }));
        }
        // Each pane re-lays out its subtree when AppKit resizes it.
        for pane in typed {
            *pane.ivars().borrow_mut() = Rc::downgrade(&node.meta);
        }
        node
    }
}

/// Marks `view` and everything inside it as needing layout and display.
fn relayout_all(view: &NSView) {
    view.setNeedsLayout(true);
    view.setNeedsDisplay(true);
    for sub in view.subviews().iter() {
        relayout_all(&sub);
    }
}

/// Panes side by side with draggable dividers (see [`Split`]).
pub fn split(panes: impl crate::view::Views) -> Split {
    Split { panes: panes.into_views(), position: None, remember: None, stacked: false, collapsible: Vec::new(), trailing: None }
}

// ---- context menus ----

define_class!(
    // Action and validation for one menu item.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitMenuTarget"]
    #[ivars = MenuAction]
    pub(crate) struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, _sender: Option<&AnyObject>) {
            crate::models::flush();
            batch(|| (self.ivars().action)());
        }

        #[unsafe(method(validateMenuItem:))]
        fn validate(&self, item: &NSMenuItem) -> bool {
            if let Some(checked) = &self.ivars().checked {
                item.setState(if checked() { NSControlStateValueOn } else { NSControlStateValueOff });
            }
            if let Some(titled) = &self.ivars().titled {
                item.setTitle(&NSString::from_str(&titled()));
            }
            self.ivars().enabled.as_ref().is_none_or(|f| f())
        }
    }
);

pub(crate) struct MenuAction {
    action: Box<dyn Fn()>,
    enabled: Option<Box<dyn Fn() -> bool>>,
    checked: Option<Box<dyn Fn() -> bool>>,
    titled: Option<Box<dyn Fn() -> String>>,
}

/// A modifier key of a menu item's shortcut (Command is always included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    /// ⇧ (an uppercase key implies it too).
    Shift,
    /// ⌥
    Option,
    /// ⌃
    Control,
}

/// An item of a menu: a command, a separator or a submenu. Used for context
/// menus and the menu bar (see [`App::menu`](crate::App::menu)).
pub struct MenuItem(Entry);

enum Entry {
    Action {
        title: String,
        action: Box<dyn Fn()>,
        enabled: Option<Box<dyn Fn() -> bool>>,
        checked: Option<Box<dyn Fn() -> bool>>,
        titled: Option<Box<dyn Fn() -> String>>,
        key: Option<(String, NSEventModifierFlags)>,
    },
    Separator,
    Submenu { title: String, items: Vec<MenuItem> },
    Dynamic { title: String, items: Rc<dyn Fn() -> Vec<MenuItem>> },
}

impl MenuItem {
    /// Disabled: `true`, or a closure (read when the menu opens, and before
    /// the item's key equivalent fires).
    pub fn disabled(mut self, when: impl crate::IntoValue<bool>) -> Self {
        if let Entry::Action { enabled, .. } = &mut self.0 {
            let when = when.into_value();
            *enabled = Some(Box::new(move || !when.get()));
        }
        self
    }

    /// A checkmark: `true`, or a closure (read when the menu opens).
    pub fn checked(mut self, when: impl crate::IntoValue<bool>) -> Self {
        if let Entry::Action { checked, .. } = &mut self.0 {
            let when = when.into_value();
            *checked = Some(Box::new(move || when.get()));
        }
        self
    }

    /// Enabled while `f` returns true (now [`disabled`](Self::disabled)).
    #[deprecated(since = "0.4.0", note = "use `.disabled(move || !..)`")]
    pub fn enabled_when(self, f: impl Fn() -> bool + 'static) -> Self {
        self.disabled(move || !f())
    }

    /// A checkmark while `f` returns true (now [`checked`](Self::checked)).
    #[deprecated(since = "0.4.0", note = "`checked` takes a closure: `.checked(move || ..)`")]
    pub fn checked_when(self, f: impl Fn() -> bool + 'static) -> Self {
        self.checked(f)
    }

    /// The title `f` returns (now [`menu_item`] takes a closure as its title).
    #[deprecated(since = "0.4.0", note = "`menu_item` takes a closure as its title")]
    pub fn title_when(mut self, f: impl Fn() -> String + 'static) -> Self {
        if let Entry::Action { title, titled, .. } = &mut self.0 {
            *title = f();
            *titled = Some(Box::new(f));
        }
        self
    }

    /// A ⌘ shortcut: `"n"` is ⌘N, `"N"` is ⇧⌘N, `","` is ⌘,.
    pub fn key(self, key: &str) -> Self {
        self.key_with(key, &[])
    }

    /// A shortcut with more modifiers than ⌘, e.g. `key_with("n", &[Modifier::Option])` for ⌥⌘N.
    pub fn key_with(mut self, key: &str, modifiers: &[Modifier]) -> Self {
        let mut flags = NSEventModifierFlags::Command;
        for m in modifiers {
            flags |= match m {
                Modifier::Shift => NSEventModifierFlags::Shift,
                Modifier::Option => NSEventModifierFlags::Option,
                Modifier::Control => NSEventModifierFlags::Control,
            };
        }
        if let Entry::Action { key: k, .. } = &mut self.0 {
            *k = Some((key.into(), flags));
        }
        self
    }
}

/// A menu command. The title is fixed, or a closure read each time the
/// menu opens ("Show Sidebar" or "Hide Sidebar").
pub fn menu_item(title: impl crate::IntoValue<String>, action: impl Fn() + 'static) -> MenuItem {
    let (title, titled): (String, Option<Box<dyn Fn() -> String>>) = match title.into_value() {
        crate::Value::Fixed(t) => (t, None),
        crate::Value::Computed(f) => (crate::signal::untrack(|| f()), Some(Box::new(move || f()))),
    };
    MenuItem(Entry::Action { title, action: Box::new(action), enabled: None, checked: None, titled, key: None })
}

/// A separator line.
pub fn menu_separator() -> MenuItem {
    MenuItem(Entry::Separator)
}

/// A submenu.
pub fn submenu(title: &str, items: Vec<MenuItem>) -> MenuItem {
    MenuItem(Entry::Submenu { title: title.into(), items })
}

/// A submenu whose items `items` builds each time it opens (Open Recent, a
/// list of servers…), so it always shows the current state.
pub fn dynamic_submenu(title: &str, items: impl Fn() -> Vec<MenuItem> + 'static) -> MenuItem {
    MenuItem(Entry::Dynamic { title: title.into(), items: Rc::new(items) })
}

pub(crate) struct DynamicItems {
    items: Rc<dyn Fn() -> Vec<MenuItem>>,
    /// The current items' targets.
    targets: RefCell<Vec<Retained<NSObject>>>,
}

define_class!(
    // Rebuilds a dynamic submenu before it's shown (and before its items'
    // key equivalents are matched).
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitDynamicMenu"]
    #[ivars = DynamicItems]
    pub(crate) struct DynamicMenu;

    unsafe impl NSObjectProtocol for DynamicMenu {}

    unsafe impl NSMenuDelegate for DynamicMenu {
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: &NSMenu) {
            self.refill(menu);
        }
    }
);

impl DynamicMenu {
    pub(crate) fn refill(&self, menu: &NSMenu) {
        let mtm = MainThreadMarker::new().expect("main thread");
        let (built, targets) = build_menu(mtm, (self.ivars().items)());
        menu.removeAllItems();
        for item in built.itemArray().iter() {
            built.removeItem(&item);
            menu.addItem(&item);
        }
        *self.ivars().targets.borrow_mut() = targets;
    }
}

/// Builds an `NSMenu`; the returned targets must be kept alive with it.
pub(crate) fn build_menu(mtm: MainThreadMarker, items: Vec<MenuItem>) -> (Retained<NSMenu>, Vec<Retained<NSObject>>) {
    build_titled_menu(mtm, "", items)
}

/// As [`build_menu`], with a title (menu-bar menus and submenus).
pub(crate) fn build_titled_menu(mtm: MainThreadMarker, title: &str, items: Vec<MenuItem>) -> (Retained<NSMenu>, Vec<Retained<NSObject>>) {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    let mut keep = Vec::new();
    for item in items {
        match item.0 {
            Entry::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            Entry::Submenu { title, items } => {
                let (sub, targets) = build_titled_menu(mtm, &title, items);
                let holder = plain_item(mtm, &title, None, "");
                holder.setSubmenu(Some(&sub));
                menu.addItem(&holder);
                keep.extend(targets);
            }
            Entry::Dynamic { title, items } => {
                let sub = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(&title));
                let delegate = DynamicMenu::alloc(mtm).set_ivars(DynamicItems { items, targets: RefCell::default() });
                // SAFETY: NSObject's designated initialiser.
                let delegate: Retained<DynamicMenu> = unsafe { msg_send![super(delegate), init] };
                delegate.refill(&sub);
                // Weak in AppKit; the caller keeps the delegate.
                sub.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                let holder = plain_item(mtm, &title, None, "");
                holder.setSubmenu(Some(&sub));
                menu.addItem(&holder);
                keep.push(Retained::into_super(delegate));
            }
            Entry::Action { title, action, enabled, checked, titled, key } => {
                let target = MenuTarget::alloc(mtm).set_ivars(MenuAction { action, enabled, checked, titled });
                // SAFETY: NSObject's designated initialiser.
                let target: Retained<MenuTarget> = unsafe { msg_send![super(target), init] };
                let (k, flags) = key.unwrap_or((String::new(), NSEventModifierFlags::Command));
                let ns = plain_item(mtm, &title, Some(sel!(fire:)), &k);
                ns.setKeyEquivalentModifierMask(flags);
                // SAFETY: the target outlives the item (kept by the caller).
                unsafe { ns.setTarget(Some(&target)) };
                menu.addItem(&ns);
                keep.push(Retained::into_super(target));
            }
        }
    }
    (menu, keep)
}

/// A menu item with a nil target (or one set later).
pub(crate) fn plain_item(mtm: MainThreadMarker, title: &str, action: Option<objc2::runtime::Sel>, key: &str) -> Retained<NSMenuItem> {
    // SAFETY: plain menu item construction.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), action, &NSString::from_str(key))
    }
}

/// The view that should carry a context menu for this leaf (a scroll view's
/// document view, so right-clicks on the content reach it).
pub(crate) fn menu_host(view: &NSView) -> Retained<NSView> {
    use objc2::Message as _;
    match view.downcast_ref::<objc2_app_kit::NSScrollView>() {
        Some(s) => s.documentView().unwrap_or_else(|| view.retain()),
        None => view.retain(),
    }
}

// ---- toolbar ----

/// An item of a window's toolbar.
pub enum ToolbarItem {
    /// A button with an SF Symbol and a label.
    Button {
        /// Stable identifier.
        id: String,
        /// The label (also the tooltip and VoiceOver name).
        label: String,
        /// SF Symbol name.
        symbol: String,
        /// What it does (runs in a batch).
        action: Box<dyn Fn()>,
        /// Disabled while this (reading signals) returns true.
        disabled: Option<Box<dyn Fn() -> bool>>,
    },
    /// Flexible space between items.
    FlexibleSpace,
    /// A search field bound to a signal.
    Search {
        /// Stable identifier.
        id: String,
        /// The query.
        query: Signal<String>,
    },
}

impl ToolbarItem {
    /// Disabled: `true`, or a closure reading signals.
    pub fn disabled(self, when: impl crate::IntoValue<bool>) -> Self {
        match self {
            ToolbarItem::Button { id, label, symbol, action, .. } => {
                let when = when.into_value();
                ToolbarItem::Button { id, label, symbol, action, disabled: Some(Box::new(move || when.get())) }
            }
            other => other,
        }
    }

    fn id(&self) -> String {
        match self {
            ToolbarItem::Button { id, .. } | ToolbarItem::Search { id, .. } => id.clone(),
            ToolbarItem::FlexibleSpace => {
                // SAFETY: AppKit's identifier constant.
                unsafe { objc2_app_kit::NSToolbarFlexibleSpaceItemIdentifier }.to_string()
            }
        }
    }
}

/// A toolbar button: identifier, label, SF Symbol, action.
pub fn toolbar_button(id: &str, label: &str, symbol: &str, action: impl Fn() + 'static) -> ToolbarItem {
    ToolbarItem::Button { id: id.into(), label: label.into(), symbol: symbol.into(), action: Box::new(action), disabled: None }
}

/// Flexible space in the toolbar.
pub fn toolbar_space() -> ToolbarItem {
    ToolbarItem::FlexibleSpace
}

/// A toolbar search field bound two ways to `query`.
pub fn toolbar_search(id: &str, query: &Signal<String>) -> ToolbarItem {
    ToolbarItem::Search { id: id.into(), query: *query }
}

/// A toolbar button's action and its enabled state, both in its own window.
pub(crate) struct ToolbarAction {
    action: Box<dyn Fn()>,
    disabled: Option<Rc<dyn Fn() -> bool>>,
    window: objc2::rc::Weak<NSWindow>,
}

define_class!(
    // Action and validation for one toolbar button. AppKit asks
    // `validateToolbarItem:` as events arrive, so the enabled state follows
    // what the window can do (it was only known once its content was in).
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitToolbarTarget"]
    #[ivars = ToolbarAction]
    pub(crate) struct ToolbarTarget;

    impl ToolbarTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, _sender: Option<&AnyObject>) {
            crate::models::flush();
            let a = self.ivars();
            crate::commands::acting_in(a.window.load(), || batch(|| (a.action)()));
        }

        #[unsafe(method(validateToolbarItem:))]
        fn validate(&self, _item: &NSToolbarItem) -> bool {
            self.enabled()
        }
    }
);

impl ToolbarTarget {
    fn new(mtm: MainThreadMarker, action: Box<dyn Fn()>, disabled: Option<Rc<dyn Fn() -> bool>>, window: objc2::rc::Weak<NSWindow>) -> Retained<Self> {
        let action = Box::new(crate::signal::with_current_owner(action)) as Box<dyn Fn()>;
        let this = Self::alloc(mtm).set_ivars(ToolbarAction { action, disabled, window });
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }

    fn enabled(&self) -> bool {
        let a = self.ivars();
        a.disabled.as_ref().is_none_or(|d| !crate::commands::acting_in(a.window.load(), || d()))
    }
}

pub(crate) struct ToolbarState {
    items: RefCell<Vec<ToolbarItem>>,
    order: Vec<String>,
    /// What the created items need kept alive.
    keep: RefCell<Vec<Retained<NSObject>>>,
    effects: RefCell<Vec<Effect>>,
    /// The toolbar's window: its buttons act there.
    window: objc2::rc::Weak<NSWindow>,
    /// The width of the space before the title, over a sidebar (see
    /// [`track_sidebar`]).
    spacer: RefCell<Option<Retained<objc2_app_kit::NSLayoutConstraint>>>,
    /// Whether the toolbar has that space.
    tracks_sidebar: std::cell::Cell<bool>,
}

/// The toolbar item that keeps the title beside a sidebar.
const SIDEBAR_SPACER: &str = "ferrokit.sidebar-spacer";

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitToolbarDelegate"]
    #[ivars = ToolbarState]
    pub(crate) struct ToolbarDelegate;

    unsafe impl NSObjectProtocol for ToolbarDelegate {}

    unsafe impl NSToolbarDelegate for ToolbarDelegate {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item_for(&self, _toolbar: &NSToolbar, id: &NSString, _insert: bool) -> Option<Retained<NSToolbarItem>> {
            if id.to_string() == SIDEBAR_SPACER {
                Some(self.spacer_item(id))
            } else {
                self.make_item(&id.to_string())
            }
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_ids(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            self.ids()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_ids(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            self.ids()
        }
    }
);

impl ToolbarDelegate {
    fn ids(&self) -> Retained<NSArray<NSString>> {
        let mut ids: Vec<Retained<NSString>> = self.ivars().order.iter().map(|s| NSString::from_str(s)).collect();
        if self.ivars().tracks_sidebar.get() {
            ids.insert(0, NSString::from_str(SIDEBAR_SPACER));
        }
        NSArray::from_retained_slice(&ids)
    }

    /// An empty navigational item (those come before the title), as wide
    /// as its constraint says.
    fn spacer_item(&self, id: &NSString) -> Retained<NSToolbarItem> {
        let mtm = MainThreadMarker::from(self);
        let view = NSView::new(mtm);
        view.setTranslatesAutoresizingMaskIntoConstraints(false);
        let before = self.ivars().spacer.borrow().as_ref().map_or(0.0, |w| w.constant());
        let width = view.widthAnchor().constraintEqualToConstant(before);
        width.setActive(true);
        view.heightAnchor().constraintEqualToConstant(1.0).setActive(true);
        self.ivars().spacer.replace(Some(width));
        let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), id);
        item.setView(Some(&view));
        item.setNavigational(true);
        // No glass capsule around the space (macOS 26 gives items one).
        item.setBordered(false);
        objc2_app_kit::NSAccessibility::setAccessibilityElement(&*view, false);
        item
    }

    fn make_item(&self, id: &str) -> Option<Retained<NSToolbarItem>> {
        let mtm = MainThreadMarker::from(self);
        let state = self.ivars();
        let mut items = state.items.borrow_mut();
        let index = items.iter().position(|i| i.id() == id)?;
        // Each item is created once (the toolbar asks once per identifier).
        let item = items.remove(index);
        drop(items);
        let ns_id = NSString::from_str(id);
        match item {
            ToolbarItem::Button { label, symbol, action, disabled, .. } => {
                let ti = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &ns_id);
                ti.setLabel(&NSString::from_str(&label));
                ti.setToolTip(Some(&NSString::from_str(&label)));
                ti.setImage(NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(&symbol), Some(&NSString::from_str(&label))).as_deref());
                let disabled: Option<Rc<dyn Fn() -> bool>> = disabled.map(Rc::from);
                let target = ToolbarTarget::new(mtm, action, disabled.clone(), state.window.clone());
                // SAFETY: the target responds to `fire:`; kept alive below.
                unsafe {
                    ti.setTarget(Some(&target));
                    ti.setAction(Some(sel!(fire:)));
                }
                if disabled.is_some() {
                    // Validated as events arrive, and at once when signals it reads change.
                    let (t, target) = (ti.clone(), target.clone());
                    state.effects.borrow_mut().push(Effect::new(move || t.setEnabled(target.enabled())));
                }
                state.keep.borrow_mut().push(Retained::into_super(target));
                Some(ti)
            }
            ToolbarItem::Search { query, .. } => {
                let si = NSSearchToolbarItem::initWithItemIdentifier(NSSearchToolbarItem::alloc(mtm), &ns_id);
                let field = si.searchField();
                let delegate = FieldDelegate::new(mtm, query);
                // SAFETY: the delegate is weak in AppKit; kept alive below.
                unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
                state.effects.borrow_mut().push(Effect::new(move || {
                    let v = query.get();
                    if field.stringValue().to_string() != v {
                        field.setStringValue(&NSString::from_str(&v));
                    }
                }));
                state.keep.borrow_mut().push(Retained::into_super(delegate));
                Some(Retained::into_super(si))
            }
            ToolbarItem::FlexibleSpace => None,
        }
    }
}

/// Gives `window` a unified toolbar with `items` (kept alive by the window
/// list for the app's lifetime).
pub(crate) fn install_toolbar(window: &NSWindow, items: Vec<ToolbarItem>) -> Retained<NSObject> {
    let mtm = MainThreadMarker::from(window);
    let order = items.iter().map(ToolbarItem::id).collect();
    let state = ToolbarState { items: RefCell::new(items), order, keep: RefCell::default(), effects: RefCell::default(), window: objc2::rc::Weak::from(window), spacer: RefCell::default(), tracks_sidebar: std::cell::Cell::new(false) };
    let delegate = ToolbarDelegate::alloc(mtm).set_ivars(state);
    // SAFETY: NSObject's designated initialiser.
    let delegate: Retained<ToolbarDelegate> = unsafe { msg_send![super(delegate), init] };
    // One identifier per toolbar: toolbars sharing one keep their items in
    // sync across windows, and each window's title spacer is its own.
    thread_local! {
        static TOOLBARS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }
    let n = TOOLBARS.with(|t| {
        t.set(t.get() + 1);
        t.get()
    });
    let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str(&format!("ferrokit.toolbar.{n}")));
    toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    toolbar.setAllowsUserCustomization(false);
    // Symbols alone, as Apple's apps show them (the labels stay for
    // VoiceOver, tooltips and the overflow menu).
    toolbar.setDisplayMode(objc2_app_kit::NSToolbarDisplayMode::IconOnly);
    window.setToolbar(Some(&toolbar));
    window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    Retained::into_super(delegate)
}

/// Keeps `window`'s title and toolbar items beside the sidebar (`split`'s
/// first pane), as Apple's apps with a sidebar have them, instead of over
/// it: a navigational space before the title, as wide as the sidebar,
/// following its divider. (AppKit's own tracking separator needs a split
/// view controller, whose title-bar sections kept every closed window's
/// frame alive when hosted in Ferrokit's views.)
pub(crate) fn track_sidebar(window: &NSWindow, delegate: &NSObject, split: &NSSplitView) {
    let info = split.delegate().filter(|d| {
        // SAFETY: reading the class of an object AppKit holds.
        let obj: &AnyObject = unsafe { &*Retained::as_ptr(d).cast::<AnyObject>() };
        obj.class().name().to_str() == Ok("FerrokitSplitDelegate")
    });
    let (Some(toolbar), Some(delegate), Some(info)) = (window.toolbar(), delegate.downcast_ref::<ToolbarDelegate>(), info) else {
        return;
    };
    // SAFETY: checked above that it's a `SplitDelegate`.
    let info: Retained<SplitDelegate> = unsafe { Retained::cast_unchecked(info) };
    delegate.ivars().tracks_sidebar.set(true);
    toolbar.insertItemWithItemIdentifier_atIndex(&NSString::from_str(SIDEBAR_SPACER), 0);
    let Some(width) = delegate.ivars().spacer.borrow().clone() else { return };
    let window = objc2::rc::Weak::from(window);
    let fit = move |split: &NSSplitView| {
        if let Some(window) = window.load() {
            fit_title(&window, split, &width);
        }
    };
    fit(split);
    info.ivars().on_resize.replace(Some(Box::new(fit)));
}

/// Sizes the space before the title so the title starts 18 points past the
/// sidebar's edge (none while the sidebar is hidden). Measured, not
/// computed: AppKit's spacing around toolbar items varies by version.
fn fit_title(window: &NSWindow, split: &NSSplitView, width: &objc2_app_kit::NSLayoutConstraint) {
    let sidebar = split.arrangedSubviews().firstObject().filter(|p| !p.isHidden()).map_or(0.0, |p| p.frame().size.width);
    let Some(frame) = window.contentView().and_then(|c| unsafe { c.superview() }) else { return };
    let title = window.title().to_string();
    for _ in 0..3 {
        if sidebar < 1.0 {
            width.setConstant(0.0);
            return;
        }
        frame.layoutSubtreeIfNeeded();
        let Some(label) = title_label(&frame, &title) else {
            // No title shown: start the items past the traffic lights' area.
            width.setConstant((sidebar - 80.0).max(0.0));
            return;
        };
        let x = label.convertRect_toView(label.bounds(), None).origin.x;
        let want = (width.constant() + sidebar + 18.0 - x).max(0.0);
        if (want - width.constant()).abs() < 0.5 {
            return;
        }
        width.setConstant(want);
    }
}

/// The text field in the title bar showing `title`.
fn title_label(view: &NSView, title: &str) -> Option<Retained<objc2_app_kit::NSTextField>> {
    for sub in view.subviews().iter() {
        if let Some(f) = sub.clone().downcast::<objc2_app_kit::NSTextField>().ok().filter(|f| f.stringValue().to_string() == title && !f.isHidden()) {
            return Some(f);
        }
        // Not into the window's content.
        if sub.class().name().to_str() != Ok("FerrokitRootView")
            && let Some(f) = title_label(&sub, title)
        {
            return Some(f);
        }
    }
    None
}
