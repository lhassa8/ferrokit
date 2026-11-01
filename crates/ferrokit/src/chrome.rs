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
    /// One of AppKit's own actions, sent along the responder chain (or to
    /// the font manager): Bold, Show Fonts, Align Left…
    Standard { title: String, action: objc2::runtime::Sel, key: &'static str, modifiers: NSEventModifierFlags, tag: isize, font_manager: bool },
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

/// TextEdit's Format menu, for rich text editors
/// ([`TextEditor::rich`](crate::TextEditor::rich)): Font (Show Fonts, Bold,
/// Italic, Underline, Bigger, Smaller, Show Colors, Copy and Paste Style)
/// and Text (alignment, Show Ruler, Copy and Paste Ruler), with their usual
/// shortcuts. AppKit enables what applies to the focused editor.
///
/// ```no_run
/// use ferrokit::*;
///
/// let app = App::new("Journal").menu("Format", format_menu());
/// # let _ = app;
/// ```
pub fn format_menu() -> Vec<MenuItem> {
    use objc2_app_kit::NSEventModifierFlags as M;
    let item = |title: &str, action, key, modifiers, tag, font_manager| MenuItem(Entry::Standard { title: crate::t(title), action, key, modifiers, tag, font_manager });
    let (cmd, shift, opt, ctrl) = (M::Command, M::Shift, M::Option, M::Control);
    vec![
        submenu(&crate::t("Font"), vec![
            item("Show Fonts", sel!(orderFrontFontPanel:), "t", cmd, 0, true),
            // NSFontManager's traits: italic 1, bold 2; sizes: bigger 3, smaller 4.
            item("Bold", sel!(addFontTrait:), "b", cmd, 2, true),
            item("Italic", sel!(addFontTrait:), "i", cmd, 1, true),
            item("Underline", sel!(underline:), "u", cmd, 0, false),
            menu_separator(),
            item("Bigger", sel!(modifyFont:), "+", cmd, 3, true),
            item("Smaller", sel!(modifyFont:), "-", cmd, 4, true),
            menu_separator(),
            item("Show Colors", sel!(orderFrontColorPanel:), "C", cmd | shift, 0, false),
            menu_separator(),
            item("Copy Style", sel!(copyFont:), "c", cmd | opt, 0, false),
            item("Paste Style", sel!(pasteFont:), "v", cmd | opt, 0, false),
        ]),
        submenu(&crate::t("Text"), vec![
            item("Align Left", sel!(alignLeft:), "{", cmd, 0, false),
            item("Center", sel!(alignCenter:), "|", cmd, 0, false),
            item("Justify", sel!(alignJustified:), "", cmd, 0, false),
            item("Align Right", sel!(alignRight:), "}", cmd, 0, false),
            menu_separator(),
            item("Show Ruler", sel!(toggleRuler:), "r", cmd | ctrl, 0, false),
            item("Copy Ruler", sel!(copyRuler:), "c", cmd | ctrl, 0, false),
            item("Paste Ruler", sel!(pasteRuler:), "v", cmd | ctrl, 0, false),
        ]),
    ]
}

/// Customize Toolbar…, for a View menu: opens the main window's toolbar
/// customisation sheet; enabled when that toolbar is customisable (see
/// [`Window::customizable_toolbar`](crate::Window::customizable_toolbar)).
pub fn customize_toolbar_item() -> MenuItem {
    let toolbar = || {
        let mtm = MainThreadMarker::new()?;
        objc2_app_kit::NSApplication::sharedApplication(mtm).mainWindow()?.toolbar().filter(|t| t.allowsUserCustomization())
    };
    menu_item(crate::t("Customize Toolbar…"), move || {
        if let Some(t) = toolbar() {
            // SAFETY: opening the sheet AppKit's own menu item opens.
            unsafe { t.runCustomizationPalette(None) };
        }
    })
    .disabled(move || toolbar().is_none())
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
            Entry::Standard { title, action, key, modifiers, tag, font_manager } => {
                let ns = plain_item(mtm, &title, Some(action), key);
                ns.setKeyEquivalentModifierMask(modifiers);
                ns.setTag(tag);
                if font_manager {
                    // SAFETY: the shared font manager lives as long as the app.
                    unsafe { ns.setTarget(Some(&objc2_app_kit::NSFontManager::sharedFontManager(mtm))) };
                }
                menu.addItem(&ns);
            }
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

/// An item of a window's toolbar: made by [`toolbar_button`],
/// [`toolbar_group`], [`toolbar_picker`], [`toolbar_search`],
/// [`toolbar_space`] or [`toolbar_fixed_space`].
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
    /// A fixed space between items.
    Space,
    /// A search field bound to a signal.
    Search {
        /// Stable identifier.
        id: String,
        /// The query.
        query: Signal<String>,
    },
    /// Buttons side by side in one capsule, as one item (see
    /// [`toolbar_group`]).
    Group {
        /// Stable identifier.
        id: String,
        /// The group's label (the overflow menu and customisation sheet).
        label: String,
        /// Its buttons, each made by [`toolbar_button`] (their own
        /// identifiers and disabled states aren't used).
        buttons: Vec<ToolbarItem>,
    },
    /// A choice of one, shown as segments (see [`toolbar_picker`]).
    Picker {
        /// Stable identifier.
        id: String,
        /// The picker's label.
        label: String,
        /// Each choice's label and SF Symbol.
        choices: Vec<(String, String)>,
        /// The chosen index (reading signals).
        selected: Box<dyn Fn() -> usize>,
        /// Called with the index the user chose.
        choose: Box<dyn Fn(usize)>,
    },
    /// An item the user can add from the customisation sheet, not in the
    /// toolbar at first (see [`ToolbarItem::optional`]).
    Optional(Box<ToolbarItem>),
}

impl ToolbarItem {
    /// Disabled: `true`, or a closure reading signals.
    pub fn disabled(self, when: impl crate::IntoValue<bool>) -> Self {
        match self {
            ToolbarItem::Button { id, label, symbol, action, .. } => {
                let when = when.into_value();
                ToolbarItem::Button { id, label, symbol, action, disabled: Some(Box::new(move || when.get())) }
            }
            ToolbarItem::Optional(item) => ToolbarItem::Optional(Box::new(item.disabled(when))),
            other => other,
        }
    }

    /// Offered in the customisation sheet (with
    /// [`Window::customizable_toolbar`](crate::Window::customizable_toolbar)),
    /// but not in the toolbar until the user adds it.
    pub fn optional(self) -> Self {
        match self {
            ToolbarItem::Optional(_) => self,
            other => ToolbarItem::Optional(Box::new(other)),
        }
    }

    fn id(&self) -> String {
        // SAFETY: AppKit's identifier constants.
        match self {
            ToolbarItem::Button { id, .. } | ToolbarItem::Search { id, .. } | ToolbarItem::Group { id, .. } | ToolbarItem::Picker { id, .. } => id.clone(),
            ToolbarItem::FlexibleSpace => unsafe { objc2_app_kit::NSToolbarFlexibleSpaceItemIdentifier }.to_string(),
            ToolbarItem::Space => unsafe { objc2_app_kit::NSToolbarSpaceItemIdentifier }.to_string(),
            ToolbarItem::Optional(item) => item.id(),
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

/// A fixed space in the toolbar.
pub fn toolbar_fixed_space() -> ToolbarItem {
    ToolbarItem::Space
}

/// A toolbar search field bound two ways to `query`.
pub fn toolbar_search(id: &str, query: &Signal<String>) -> ToolbarItem {
    ToolbarItem::Search { id: id.into(), query: *query }
}

/// Related buttons in one capsule (zoom out and in, back and forward), made
/// by [`toolbar_button`]; one item to the overflow menu and the
/// customisation sheet, under `label`.
///
/// ```no_run
/// use ferrokit::*;
///
/// let zoom = Signal::new(1.0);
/// let item = toolbar_group("zoom", "Zoom", vec![
///     toolbar_button("out", "Zoom Out", "minus.magnifyingglass", move || zoom.update(|z| *z /= 1.25)),
///     toolbar_button("in", "Zoom In", "plus.magnifyingglass", move || zoom.update(|z| *z *= 1.25)),
/// ]);
/// # let _ = item;
/// ```
pub fn toolbar_group(id: &str, label: &str, buttons: Vec<ToolbarItem>) -> ToolbarItem {
    ToolbarItem::Group { id: id.into(), label: label.into(), buttons }
}

/// A choice of one in the toolbar, as segments with SF Symbols (Finder's
/// view modes): `value` is set to the chosen choice's value, and the
/// segment follows it. Each choice is a label (for VoiceOver, tooltips and
/// the overflow menu), a symbol and a value.
///
/// ```no_run
/// use ferrokit::*;
///
/// #[derive(Clone, Copy, PartialEq)]
/// enum Mode { List, Grid }
/// let mode = Signal::new(Mode::List);
/// let item = toolbar_picker("mode", "View", &mode, vec![("List", "list.bullet", Mode::List), ("Grid", "square.grid.2x2", Mode::Grid)]);
/// # let _ = item;
/// ```
pub fn toolbar_picker<T: Clone + PartialEq + 'static>(id: &str, label: &str, value: &Signal<T>, choices: Vec<(&str, &str, T)>) -> ToolbarItem {
    let value = *value;
    let values: Rc<Vec<T>> = Rc::new(choices.iter().map(|c| c.2.clone()).collect());
    let v2 = values.clone();
    ToolbarItem::Picker {
        id: id.into(),
        label: label.into(),
        choices: choices.iter().map(|(l, s, _)| (l.to_string(), s.to_string())).collect(),
        selected: Box::new(move || value.with(|v| values.iter().position(|c| c == v).unwrap_or(0))),
        choose: Box::new(move |i| {
            if let Some(v) = v2.get(i) {
                value.set(v.clone());
            }
        }),
    }
}

/// A toolbar item as the delegate keeps it, to make it again (for the
/// customisation sheet, and when the user adds it back).
enum Spec {
    Button { label: String, symbol: String, action: Rc<dyn Fn(isize)>, disabled: Option<Rc<dyn Fn() -> bool>> },
    Search { query: Signal<String> },
    Group { label: String, buttons: Vec<(String, String)>, action: Rc<dyn Fn(isize)> },
    Picker { label: String, choices: Vec<(String, String)>, selected: Rc<dyn Fn() -> usize>, choose: Rc<dyn Fn(isize)> },
    Standard,
}

struct ToolbarEntry {
    id: String,
    spec: Spec,
    optional: bool,
}

impl ToolbarEntry {
    fn new(item: ToolbarItem) -> ToolbarEntry {
        let id = item.id();
        // Actions run with the owner of the window being built.
        let act = |f: Box<dyn Fn()>| -> Rc<dyn Fn(isize)> {
            let f = crate::signal::with_current_owner(f);
            Rc::new(move |_| f())
        };
        let (spec, optional) = match item {
            ToolbarItem::Optional(item) => (ToolbarEntry::new(*item).spec, true),
            ToolbarItem::Button { label, symbol, action, disabled, .. } => (Spec::Button { label, symbol, action: act(action), disabled: disabled.map(Rc::from) }, false),
            ToolbarItem::Search { query, .. } => (Spec::Search { query }, false),
            ToolbarItem::Group { label, buttons, .. } => {
                let (mut shown, mut actions) = (Vec::new(), Vec::new());
                for b in buttons {
                    if let ToolbarItem::Button { label, symbol, action, .. } = b {
                        shown.push((label, symbol));
                        actions.push(action);
                    }
                }
                let actions: Vec<Box<dyn Fn()>> = actions.into_iter().map(|a| Box::new(crate::signal::with_current_owner(a)) as Box<dyn Fn()>).collect();
                let action = move |i: isize| {
                    if let Some(a) = usize::try_from(i).ok().and_then(|i| actions.get(i)) {
                        a();
                    }
                };
                (Spec::Group { label, buttons: shown, action: Rc::new(action) }, false)
            }
            ToolbarItem::Picker { label, choices, selected, choose, .. } => {
                let choose: Rc<dyn Fn(usize)> = Rc::from(choose);
                (Spec::Picker { label, choices, selected: Rc::from(selected), choose: Rc::new(move |i| choose(i.max(0) as usize)) }, false)
            }
            ToolbarItem::FlexibleSpace | ToolbarItem::Space => (Spec::Standard, false),
        };
        ToolbarEntry { id, spec, optional }
    }
}

/// A toolbar item's action and its enabled state, both in its own window.
pub(crate) struct ToolbarAction {
    /// Called with the segment clicked (a group's), else 0.
    action: Rc<dyn Fn(isize)>,
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
        fn fire(&self, sender: Option<&AnyObject>) {
            crate::models::flush();
            let index = sender.and_then(|s| s.downcast_ref::<objc2_app_kit::NSToolbarItemGroup>()).map_or(0, |g| g.selectedIndex());
            let a = self.ivars();
            crate::commands::acting_in(a.window.load(), || batch(|| (a.action)(index)));
        }

        #[unsafe(method(validateToolbarItem:))]
        fn validate(&self, _item: &NSToolbarItem) -> bool {
            self.enabled()
        }
    }
);

impl ToolbarTarget {
    fn new(mtm: MainThreadMarker, action: Rc<dyn Fn(isize)>, disabled: Option<Rc<dyn Fn() -> bool>>, window: objc2::rc::Weak<NSWindow>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ToolbarAction { action, disabled, window });
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }

    fn enabled(&self) -> bool {
        let a = self.ivars();
        a.disabled.as_ref().is_none_or(|d| !crate::commands::acting_in(a.window.load(), || d()))
    }
}

/// What an item in the toolbar keeps alive and up to date.
#[derive(Default)]
struct Live {
    keep: Vec<Retained<NSObject>>,
    effects: Vec<Effect>,
}

pub(crate) struct ToolbarState {
    entries: Vec<ToolbarEntry>,
    /// What each item in the toolbar needs, by identifier (replaced when
    /// the user adds it again).
    live: RefCell<std::collections::HashMap<String, Live>>,
    /// The toolbar's window: its buttons act there.
    window: objc2::rc::Weak<NSWindow>,
    /// The width of the space before the title, over a sidebar (see
    /// [`track_sidebar`]).
    spacer: RefCell<Option<Retained<objc2_app_kit::NSLayoutConstraint>>>,
    /// Whether the toolbar has that space.
    tracks_sidebar: std::cell::Cell<bool>,
    /// Where a customisable toolbar's layout is remembered.
    remember: Option<String>,
    /// The window's shell has a sidebar, an inspector: their toggles and
    /// tracking separators lead and end the toolbar.
    shell: (bool, bool),
}

/// AppKit's own items for a shell's sidebar and inspector.
fn shell_ids(shell: (bool, bool)) -> (Vec<String>, Vec<String>) {
    // SAFETY: AppKit's identifier constants.
    unsafe {
        use objc2_app_kit::*;
        let lead = if shell.0 { vec![NSToolbarToggleSidebarItemIdentifier.to_string(), NSToolbarSidebarTrackingSeparatorItemIdentifier.to_string()] } else { Vec::new() };
        let end = if shell.1 { vec![NSToolbarInspectorTrackingSeparatorItemIdentifier.to_string(), NSToolbarFlexibleSpaceItemIdentifier.to_string(), NSToolbarToggleInspectorItemIdentifier.to_string()] } else { Vec::new() };
        (lead, end)
    }
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
        fn item_for(&self, _toolbar: &NSToolbar, id: &NSString, insert: bool) -> Option<Retained<NSToolbarItem>> {
            if id.to_string() == SIDEBAR_SPACER {
                Some(self.spacer_item(id))
            } else {
                self.make_item(&id.to_string(), insert)
            }
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_ids(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            let saved = self.saved();
            let ids = saved.unwrap_or_else(|| self.ivars().entries.iter().filter(|e| !e.optional).map(|e| e.id.clone()).collect());
            self.with_spacer(ids)
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_ids(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            let mut ids: Vec<String> = self.ivars().entries.iter().filter(|e| !matches!(e.spec, Spec::Standard)).map(|e| e.id.clone()).collect();
            // SAFETY: AppKit's identifier constants.
            ids.extend([unsafe { objc2_app_kit::NSToolbarSpaceItemIdentifier }.to_string(), unsafe { objc2_app_kit::NSToolbarFlexibleSpaceItemIdentifier }.to_string()]);
            self.with_spacer(ids)
        }

        // The space that keeps the title beside a sidebar stays first.
        #[unsafe(method_id(toolbarImmovableItemIdentifiers:))]
        fn immovable_ids(&self, _toolbar: &NSToolbar) -> Retained<objc2_foundation::NSSet<NSString>> {
            objc2_foundation::NSSet::from_retained_slice(&[NSString::from_str(SIDEBAR_SPACER)])
        }

        #[unsafe(method(toolbarWillAddItem:))]
        fn will_add(&self, _notification: &NSNotification) {
            self.save_soon();
        }

        #[unsafe(method(toolbarDidRemoveItem:))]
        fn did_remove(&self, notification: &NSNotification) {
            // A removed item's effects and targets go with it.
            let item = notification.userInfo().and_then(|u| u.objectForKey(&NSString::from_str("item"))).and_then(|i| i.downcast::<NSToolbarItem>().ok());
            if let Some(item) = item {
                self.ivars().live.borrow_mut().remove(&item.itemIdentifier().to_string());
            }
            self.save_soon();
        }
    }
);

impl ToolbarDelegate {
    fn with_spacer(&self, ids: Vec<String>) -> Retained<NSArray<NSString>> {
        let (lead, end) = shell_ids(self.ivars().shell);
        let mut ids: Vec<Retained<NSString>> = lead.iter().chain(ids.iter()).chain(end.iter()).map(|s| NSString::from_str(s)).collect();
        if self.ivars().tracks_sidebar.get() {
            ids.insert(0, NSString::from_str(SIDEBAR_SPACER));
        }
        NSArray::from_retained_slice(&ids)
    }

    fn key(&self) -> Option<Retained<NSString>> {
        self.ivars().remember.as_ref().map(|n| NSString::from_str(&format!("ferrokit.toolbar.{n}")))
    }

    /// The layout the user saved, without items the app no longer has.
    fn saved(&self) -> Option<Vec<String>> {
        let list = NSUserDefaults::standardUserDefaults().stringArrayForKey(&*self.key()?)?;
        let known = |id: &str| {
            // SAFETY: AppKit's identifier constants.
            let standard = [unsafe { objc2_app_kit::NSToolbarSpaceItemIdentifier }.to_string(), unsafe { objc2_app_kit::NSToolbarFlexibleSpaceItemIdentifier }.to_string()];
            standard.iter().any(|s| s == id) || self.ivars().entries.iter().any(|e| e.id == id)
        };
        Some(list.iter().map(|s| s.to_string()).filter(|s| known(s)).collect())
    }

    /// Saves a customisable toolbar's layout once the change is in.
    fn save_soon(&self) {
        let Some(key) = self.key() else { return };
        let Some(window) = self.ivars().window.load() else { return };
        let defaults: Vec<String> = self.ivars().entries.iter().filter(|e| !e.optional).map(|e| e.id.clone()).collect();
        let shell = self.ivars().shell;
        let had = NSUserDefaults::standardUserDefaults().stringArrayForKey(&key).map(|l| l.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let window = objc2::rc::Weak::from(&*window);
        crate::spawn(
            || (),
            move |()| {
                let Some(toolbar) = window.load().and_then(|w| w.toolbar()) else { return };
                // The shell's own items come back with it: not saved.
                let (lead, end) = shell_ids(shell);
                let mut ids: Vec<String> = toolbar.items().iter().map(|i| i.itemIdentifier().to_string()).filter(|i| i != SIDEBAR_SPACER).collect();
                if ids.starts_with(&lead) {
                    ids.drain(..lead.len());
                }
                if ids.ends_with(&end) {
                    ids.truncate(ids.len() - end.len());
                }
                // The app's own layout isn't saved (a later version's would
                // then show), only the user's.
                if had.as_ref() == Some(&ids) || (had.is_none() && ids == defaults) {
                    return;
                }
                let list = NSArray::from_retained_slice(&ids.iter().map(|s| NSString::from_str(s)).collect::<Vec<_>>());
                // SAFETY: an array of strings is a property list.
                unsafe { NSUserDefaults::standardUserDefaults().setObject_forKey(Some(&list), &key) };
            },
        );
    }

    /// An empty navigational item (those come before the title), as wide
    /// as its constraint says.
    fn spacer_item(&self, id: &NSString) -> Retained<NSToolbarItem> {
        let mtm = MainThreadMarker::from(self);
        let view = NSView::new(mtm);
        view.setTranslatesAutoresizingMaskIntoConstraints(false);
        let before = self.ivars().spacer.borrow().as_ref().map_or(MIN_SPACE, |w| w.constant());
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

    /// Makes the item `id`: live (acting, following its signals) when it's
    /// going into the toolbar, a picture of itself for the customisation
    /// sheet otherwise.
    fn make_item(&self, id: &str, insert: bool) -> Option<Retained<NSToolbarItem>> {
        let mtm = MainThreadMarker::from(self);
        let state = self.ivars();
        let entry = state.entries.iter().find(|e| e.id == id)?;
        let ns_id = NSString::from_str(id);
        let mut live = Live::default();
        let symbol = |name: &str, label: &str| NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), Some(&NSString::from_str(label)));
        use objc2::Message as _;
        let target = |live: &mut Live, item: &NSToolbarItem, action: &Rc<dyn Fn(isize)>, disabled: Option<Rc<dyn Fn() -> bool>>| {
            let t = ToolbarTarget::new(mtm, action.clone(), disabled.clone(), state.window.clone());
            // SAFETY: the target responds to `fire:`; kept alive in `live`.
            unsafe {
                item.setTarget(Some(&t));
                item.setAction(Some(sel!(fire:)));
            }
            if disabled.is_some() {
                // Validated as events arrive, and at once when signals it reads change.
                let (i, t) = (item.retain(), t.clone());
                live.effects.push(Effect::new(move || i.setEnabled(t.enabled())));
            }
            live.keep.push(Retained::into_super(t));
        };
        let item: Retained<NSToolbarItem> = match &entry.spec {
            Spec::Button { label, symbol: name, action, disabled } => {
                let ti = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &ns_id);
                ti.setLabel(&NSString::from_str(label));
                ti.setToolTip(Some(&NSString::from_str(label)));
                ti.setImage(symbol(name, label).as_deref());
                target(&mut live, &ti, action, disabled.clone());
                ti
            }
            Spec::Search { query } => {
                let si = NSSearchToolbarItem::initWithItemIdentifier(NSSearchToolbarItem::alloc(mtm), &ns_id);
                if insert {
                    let query = *query;
                    let field = si.searchField();
                    let delegate = FieldDelegate::new(mtm, query);
                    // SAFETY: the delegate is weak in AppKit; kept alive in `live`.
                    unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
                    live.effects.push(Effect::new(move || {
                        let v = query.get();
                        if field.stringValue().to_string() != v {
                            field.setStringValue(&NSString::from_str(&v));
                        }
                    }));
                    live.keep.push(Retained::into_super(delegate));
                }
                Retained::into_super(si)
            }
            Spec::Group { label, buttons, action } => {
                let group = group_item(mtm, &ns_id, label, buttons, objc2_app_kit::NSToolbarItemGroupSelectionMode::Momentary);
                target(&mut live, &group, action, None);
                group
            }
            Spec::Picker { label, choices, selected, choose } => {
                let group = group_item(mtm, &ns_id, label, choices, objc2_app_kit::NSToolbarItemGroupSelectionMode::SelectOne);
                target(&mut live, &group, choose, None);
                if let Some(g) = group.downcast_ref::<objc2_app_kit::NSToolbarItemGroup>().map(|g| g.retain()) {
                    let selected = selected.clone();
                    live.effects.push(Effect::new(move || g.setSelectedIndex(selected() as isize)));
                }
                group
            }
            Spec::Standard => return None,
        };
        if insert {
            state.live.borrow_mut().insert(id.to_string(), live);
        }
        Some(item)
    }
}

/// A group of segments with SF Symbols, each labelled (VoiceOver, tooltips,
/// the overflow menu), under `label`.
fn group_item(mtm: MainThreadMarker, id: &NSString, label: &str, parts: &[(String, String)], mode: objc2_app_kit::NSToolbarItemGroupSelectionMode) -> Retained<NSToolbarItem> {
    let images: Vec<Retained<NSImage>> = parts
        .iter()
        .map(|(l, s)| NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(s), Some(&NSString::from_str(l))).unwrap_or_default())
        .collect();
    let labels: Vec<Retained<NSString>> = parts.iter().map(|(l, _)| NSString::from_str(l)).collect();
    // SAFETY: images and labels match in count; the target and action are
    // set by the caller.
    let group = unsafe {
        objc2_app_kit::NSToolbarItemGroup::groupWithItemIdentifier_images_selectionMode_labels_target_action(
            id,
            &NSArray::from_retained_slice(&images),
            mode,
            Some(&NSArray::from_retained_slice(&labels)),
            None,
            None,
            mtm,
        )
    };
    group.setLabel(&NSString::from_str(label));
    for (sub, (l, _)) in group.subitems().iter().zip(parts) {
        sub.setToolTip(Some(&NSString::from_str(l)));
    }
    Retained::into_super(group)
}

/// Gives `window` a unified toolbar with `items` (kept alive by the window
/// list for the app's lifetime); `remember` makes it customisable, its
/// layout saved under that name.
pub(crate) fn install_toolbar(window: &NSWindow, items: Vec<ToolbarItem>, remember: Option<String>, shell: (bool, bool)) -> Retained<NSObject> {
    let mtm = MainThreadMarker::from(window);
    let entries = items.into_iter().map(ToolbarEntry::new).collect();
    let customizable = remember.is_some();
    let state = ToolbarState { entries, live: RefCell::default(), window: objc2::rc::Weak::from(window), spacer: RefCell::default(), tracks_sidebar: std::cell::Cell::new(false), remember, shell };
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
    toolbar.setAllowsUserCustomization(customizable);
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

/// The space's narrowest width: AppKit warns about toolbar item views of
/// zero width.
const MIN_SPACE: f64 = 1.0;

/// Sizes the space before the title so the title starts 18 points past the
/// sidebar's edge (none while the sidebar is hidden). Measured, not
/// computed: AppKit's spacing around toolbar items varies by version.
fn fit_title(window: &NSWindow, split: &NSSplitView, width: &objc2_app_kit::NSLayoutConstraint) {
    let sidebar = split.arrangedSubviews().firstObject().filter(|p| !p.isHidden()).map_or(0.0, |p| p.frame().size.width);
    let Some(frame) = window.contentView().and_then(|c| unsafe { c.superview() }) else { return };
    let title = window.title().to_string();
    for _ in 0..3 {
        if sidebar < 1.0 {
            width.setConstant(MIN_SPACE);
            return;
        }
        frame.layoutSubtreeIfNeeded();
        let Some(label) = title_label(&frame, &title) else {
            // No title shown: start the items past the traffic lights' area.
            width.setConstant((sidebar - 80.0).max(MIN_SPACE));
            return;
        };
        let x = label.convertRect_toView(label.bounds(), None).origin.x;
        let want = (width.constant() + sidebar + 18.0 - x).max(MIN_SPACE);
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
