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
use crate::appkit_views::ActionTarget;
use crate::view::{AnyView, Cx, Kind, Meta, Node, View};

// ---- split view ----

define_class!(
    // One pane of a split view: flipped, and it re-lays out its content when
    // AppKit resizes it (window resize, divider drag).
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiSplitPane"]
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
    #[name = "AppKitUiSplitDelegate"]
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
}

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
    /// use appkit_ui::*;
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
    NSString::from_str(&format!("appkit-ui split {name}"))
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
        let delegate = SplitDelegate::alloc(mtm).set_ivars(SplitInfo { key, collapsible: self.collapsible.clone(), applying: std::cell::Cell::new(false) });
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
            position: std::cell::Cell::new(saved.or(self.position)),
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

/// Panes side by side with draggable dividers (see [`Split`]).
pub fn split(panes: impl crate::view::Views) -> Split {
    Split { panes: panes.into_views(), position: None, remember: None, stacked: false, collapsible: Vec::new(), trailing: None }
}

// ---- context menus ----

define_class!(
    // Action and validation for one menu item.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiMenuTarget"]
    #[ivars = MenuAction]
    pub(crate) struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, _sender: Option<&AnyObject>) {
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
    /// Only enabled while `f` returns true (checked when the menu opens, and
    /// before its key equivalent fires).
    pub fn enabled_when(mut self, f: impl Fn() -> bool + 'static) -> Self {
        if let Entry::Action { enabled, .. } = &mut self.0 {
            *enabled = Some(Box::new(f));
        }
        self
    }

    /// Shows a checkmark while `f` returns true (checked when the menu opens).
    pub fn checked_when(mut self, f: impl Fn() -> bool + 'static) -> Self {
        if let Entry::Action { checked, .. } = &mut self.0 {
            *checked = Some(Box::new(f));
        }
        self
    }

    /// The title `f` returns, read when the menu opens ("Show Sidebar" /
    /// "Hide Sidebar").
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

/// A menu command.
pub fn menu_item(title: &str, action: impl Fn() + 'static) -> MenuItem {
    MenuItem(Entry::Action { title: title.into(), action: Box::new(action), enabled: None, checked: None, titled: None, key: None })
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
    #[name = "AppKitUiDynamicMenu"]
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
    /// Disabled while `f` (reading signals) returns true.
    pub fn disabled(self, f: impl Fn() -> bool + 'static) -> Self {
        match self {
            ToolbarItem::Button { id, label, symbol, action, .. } => {
                ToolbarItem::Button { id, label, symbol, action, disabled: Some(Box::new(f)) }
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

pub(crate) struct ToolbarState {
    items: RefCell<Vec<ToolbarItem>>,
    order: Vec<String>,
    /// What the created items need kept alive.
    keep: RefCell<Vec<Retained<NSObject>>>,
    effects: RefCell<Vec<Effect>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiToolbarDelegate"]
    #[ivars = ToolbarState]
    pub(crate) struct ToolbarDelegate;

    unsafe impl NSObjectProtocol for ToolbarDelegate {}

    unsafe impl NSToolbarDelegate for ToolbarDelegate {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item_for(&self, _toolbar: &NSToolbar, id: &NSString, _insert: bool) -> Option<Retained<NSToolbarItem>> {
            self.make_item(&id.to_string())
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
        let ids: Vec<Retained<NSString>> = self.ivars().order.iter().map(|s| NSString::from_str(s)).collect();
        NSArray::from_retained_slice(&ids)
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
                let target = ActionTarget::new(mtm, action);
                // SAFETY: the target responds to `fire:`; kept alive below.
                unsafe {
                    ti.setTarget(Some(&target));
                    ti.setAction(Some(sel!(fire:)));
                }
                ti.setAutovalidates(false);
                if let Some(disabled) = disabled {
                    let t = ti.clone();
                    state.effects.borrow_mut().push(Effect::new(move || t.setEnabled(!disabled())));
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
    let state = ToolbarState { items: RefCell::new(items), order, keep: RefCell::default(), effects: RefCell::default() };
    let delegate = ToolbarDelegate::alloc(mtm).set_ivars(state);
    // SAFETY: NSObject's designated initialiser.
    let delegate: Retained<ToolbarDelegate> = unsafe { msg_send![super(delegate), init] };
    let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str("appkit-ui.toolbar"));
    toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    toolbar.setAllowsUserCustomization(false);
    window.setToolbar(Some(&toolbar));
    window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    Retained::into_super(delegate)
}
