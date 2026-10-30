//! A real `NSTextView` (in its `NSScrollView`) as a plain-text editor.
//!
//! What AppKit brings: IME with the candidate window at the caret, spell and
//! grammar checking with the suggestion menu, the find bar, undo, text
//! services (Look Up, Translate, Writing Tools), Dark Mode and VoiceOver.
//!
//! Offsets in this API are UTF-8 byte offsets into the text, as Rust strings
//! use; they're converted from AppKit's UTF-16 ranges at the boundary.
//! Like [`crate::table::TableModel`], user actions are queued on the model
//! and announced with [`crate::request_redraw`].
//!
//! Styled text: [`TextModel::set_highlighter`] colours and emphasises runs
//! of the text (syntax highlighting, search matches), re-run on every edit
//! before it's drawn. The text stays plain: styles aren't typed, pasted,
//! copied or undone.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ops::Range;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSColor, NSFont, NSResponder, NSScrollView, NSText, NSTextDelegate, NSTextInputClient, NSTextStorage, NSTextStorageDelegate,
    NSTextStorageEditActions, NSTextView, NSTextViewDelegate, NSAutoresizingMaskOptions, NSView,
};
use objc2_foundation::{NSNotification, NSPoint, NSRange, NSRect, NSSize, NSString, NSUndoManager};

use crate::ViewId;
use crate::host::mtm;

/// Something the user did in the editor.
#[derive(Debug, Clone, PartialEq)]
pub enum TextEvent {
    /// The text changed (typing, paste, undo, a committed IME composition…).
    /// Read it with [`TextModel::text`]. Not sent while composing.
    Changed,
    /// The selection or caret moved (byte range).
    Selection(Range<usize>),
    /// An input method is composing (`Some(marked text)`), or stopped (`None`).
    Composition(Option<String>),
}

trait Data {
    fn push(&self, event: TextEvent);
    fn quiet(&self) -> bool;
    fn composing(&self) -> bool;
    fn undo_manager(&self) -> Retained<NSUndoManager>;
    fn highlight(&self, edited: Option<NSRange>);
}

struct ViewIvars {
    data: RefCell<Option<Weak<dyn Data>>>,
    composing: Cell<bool>,
}

define_class!(
    // NSTextView that reports input-method composition.
    #[unsafe(super(NSTextView, NSText, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitHostTextView"]
    #[ivars = ViewIvars]
    struct HostTextView;

    impl HostTextView {
        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, selected: NSRange, replacement: NSRange) {
            let () = unsafe { msg_send![super(self), setMarkedText: string, selectedRange: selected, replacementRange: replacement] };
            self.composition_changed();
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            let () = unsafe { msg_send![super(self), unmarkText] };
            self.composition_changed();
        }

        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, replacement: NSRange) {
            let () = unsafe { msg_send![super(self), insertText: string, replacementRange: replacement] };
            self.composition_changed();
        }
    }
);

impl HostTextView {
    fn data(&self) -> Option<Rc<dyn Data>> {
        self.ivars().data.borrow().as_ref().and_then(Weak::upgrade)
    }

    /// Emits `Composition` when marked text appears, changes or goes away.
    fn composition_changed(&self) {
        let composing = self.hasMarkedText();
        let was = self.ivars().composing.replace(composing);
        let Some(data) = self.data() else { return };
        if composing {
            let range = self.markedRange();
            let marked = self.string().to_string();
            let marked = utf16_slice(&marked, range.location, range.length);
            data.push(TextEvent::Composition(Some(marked)));
        } else if was {
            data.push(TextEvent::Composition(None));
        }
    }
}

struct DelegateIvars {
    data: RefCell<Option<Weak<dyn Data>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitHostTextDelegate"]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSTextDelegate for Delegate {
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, _notification: &NSNotification) {
            if let Some(data) = self.data().filter(|d| !d.quiet()) {
                data.push(TextEvent::Changed);
            }
        }
    }

    // Every character edit, including undo and redo (for which AppKit sends
    // no textDidChange:), reaches the text storage.
    unsafe impl NSTextStorageDelegate for Delegate {
        #[unsafe(method(textStorage:didProcessEditing:range:changeInLength:))]
        fn did_process_editing(&self, _storage: &NSTextStorage, edited: NSTextStorageEditActions, range: NSRange, _delta: isize) {
            if !edited.contains(NSTextStorageEditActions::EditedCharacters) {
                return;
            }
            let Some(data) = self.data().filter(|d| !d.composing()) else { return };
            // Restyle before the layout manager hears of the edit, so new
            // text never shows in the previous run's style.
            data.highlight(Some(range));
            if !data.quiet() {
                data.push(TextEvent::Changed);
            }
        }
    }

    unsafe impl NSTextViewDelegate for Delegate {
        #[unsafe(method(textViewDidChangeSelection:))]
        fn selection_did_change(&self, notification: &NSNotification) {
            let Some(data) = self.data().filter(|d| !d.quiet()) else { return };
            let Some(view) = notification.object().and_then(|o| o.downcast::<NSTextView>().ok()) else { return };
            let text = view.string().to_string();
            let r = view.selectedRange();
            data.push(TextEvent::Selection(utf16_to_bytes(&text, r.location, r.length)));
        }

        // Each editor keeps its own undo history, instead of sharing the
        // window's (where ⌘Z in one view could undo another view's edit).
        #[unsafe(method_id(undoManagerForTextView:))]
        fn undo_manager_for(&self, _view: &NSTextView) -> Option<Retained<NSUndoManager>> {
            self.data().map(|d| d.undo_manager())
        }
    }
);

impl Delegate {
    fn data(&self) -> Option<Rc<dyn Data>> {
        self.ivars().data.borrow().as_ref().and_then(Weak::upgrade)
    }
}

/// UTF-16 range → UTF-8 byte range in `text` (clamped).
pub(crate) fn utf16_to_bytes(text: &str, location: usize, length: usize) -> Range<usize> {
    let (mut units, mut start, mut end) = (0usize, None, None);
    for (byte, c) in text.char_indices() {
        if start.is_none() && units >= location {
            start = Some(byte);
        }
        if end.is_none() && units >= location + length {
            end = Some(byte);
        }
        units += c.len_utf16();
    }
    let start = start.unwrap_or(text.len());
    start..end.unwrap_or(text.len()).max(start)
}

/// UTF-8 byte range → UTF-16 range (location, length). Bytes inside a
/// character round down to its start.
pub(crate) fn bytes_to_utf16(text: &str, range: Range<usize>) -> (usize, usize) {
    let to_units = |byte: usize| -> usize {
        let byte = byte.min(text.len());
        let mut b = byte;
        while !text.is_char_boundary(b) {
            b -= 1;
        }
        text[..b].encode_utf16().count()
    };
    let (s, e) = (to_units(range.start), to_units(range.end.max(range.start)));
    (s, e - s)
}

fn utf16_slice(text: &str, location: usize, length: usize) -> String {
    text[utf16_to_bytes(text, location, length)].to_string()
}

struct Inner {
    id: ViewId,
    events: RefCell<VecDeque<TextEvent>>,
    quiet: Cell<bool>,
    mounted: Cell<bool>,
    undo: RefCell<Retained<NSUndoManager>>,
    /// Whether `undo` is someone else's (shared with other views).
    shared: Cell<bool>,
    scroll: Retained<NSScrollView>,
    view: Retained<HostTextView>,
    _delegate: Retained<Delegate>,
    /// The font unstyled text uses (`set_font`).
    font: RefCell<Retained<NSFont>>,
    highlighter: RefCell<Option<Rc<Highlighter>>>,
    /// Set while styles are applied (they re-enter the storage delegate).
    styling: Cell<bool>,
    /// The runs last applied and the text's length then, to restyle only
    /// what an edit changed.
    shown: RefCell<Option<(usize, Runs)>>,
}

type Highlighter = dyn Fn(&str) -> Runs;
type Runs = Vec<(Range<usize>, TextStyle)>;

/// A colour for styled text. The named ones are the system's colours: they
/// follow Dark Mode and Increase Contrast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextColor {
    /// The text colour (what unstyled text uses).
    Primary,
    /// Less prominent text.
    Secondary,
    /// Even less prominent text (comments, placeholders).
    Tertiary,
    /// Links.
    Link,
    /// System red.
    Red,
    /// System orange.
    Orange,
    /// System yellow.
    Yellow,
    /// System green.
    Green,
    /// System mint.
    Mint,
    /// System teal.
    Teal,
    /// System cyan.
    Cyan,
    /// System blue.
    Blue,
    /// System indigo.
    Indigo,
    /// System purple.
    Purple,
    /// System pink.
    Pink,
    /// System brown.
    Brown,
    /// System gray.
    Gray,
    /// The background of content (text, tables): white, or dark in Dark Mode.
    Background,
    /// Separator lines and grids.
    Separator,
    /// A fixed sRGB colour (components 0–1; the same in light and dark).
    Rgb(f64, f64, f64),
}

impl TextColor {
    const NAMED: [TextColor; 19] = [
        TextColor::Primary,
        TextColor::Secondary,
        TextColor::Tertiary,
        TextColor::Link,
        TextColor::Red,
        TextColor::Orange,
        TextColor::Yellow,
        TextColor::Green,
        TextColor::Mint,
        TextColor::Teal,
        TextColor::Cyan,
        TextColor::Blue,
        TextColor::Indigo,
        TextColor::Purple,
        TextColor::Pink,
        TextColor::Brown,
        TextColor::Gray,
        TextColor::Background,
        TextColor::Separator,
    ];

    /// The `NSColor` (dynamic for the named colours).
    pub fn ns(self) -> Retained<NSColor> {
        match self {
            TextColor::Primary => NSColor::textColor(),
            TextColor::Secondary => NSColor::secondaryLabelColor(),
            TextColor::Tertiary => NSColor::tertiaryLabelColor(),
            TextColor::Link => NSColor::linkColor(),
            TextColor::Red => NSColor::systemRedColor(),
            TextColor::Orange => NSColor::systemOrangeColor(),
            TextColor::Yellow => NSColor::systemYellowColor(),
            TextColor::Green => NSColor::systemGreenColor(),
            TextColor::Mint => NSColor::systemMintColor(),
            TextColor::Teal => NSColor::systemTealColor(),
            TextColor::Cyan => NSColor::systemCyanColor(),
            TextColor::Blue => NSColor::systemBlueColor(),
            TextColor::Indigo => NSColor::systemIndigoColor(),
            TextColor::Purple => NSColor::systemPurpleColor(),
            TextColor::Pink => NSColor::systemPinkColor(),
            TextColor::Brown => NSColor::systemBrownColor(),
            TextColor::Gray => NSColor::systemGrayColor(),
            TextColor::Background => NSColor::textBackgroundColor(),
            TextColor::Separator => NSColor::separatorColor(),
            TextColor::Rgb(r, g, b) => NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0),
        }
    }

    fn from_ns(color: &NSColor) -> Option<TextColor> {
        Self::NAMED.into_iter().find(|c| c.ns().isEqual(Some(color))).or_else(|| {
            let c = color.colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace())?;
            Some(TextColor::Rgb(c.redComponent(), c.greenComponent(), c.blueComponent()))
        })
    }
}

/// How a run of text looks. What it leaves unset stays as the editor's own
/// (its font and text colour).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextStyle {
    /// The text colour.
    pub color: Option<TextColor>,
    /// A background behind the run.
    pub background: Option<TextColor>,
    /// Bold (the font's bold face).
    pub bold: bool,
    /// Italic (the font's italic face, where it has one).
    pub italic: bool,
    /// Underlined.
    pub underline: bool,
}

impl TextStyle {
    /// A style that changes nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// With this text colour.
    pub fn color(mut self, color: TextColor) -> Self {
        self.color = Some(color);
        self
    }

    /// With this background.
    pub fn background(mut self, color: TextColor) -> Self {
        self.background = Some(color);
        self
    }

    /// Bold.
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// Italic.
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Underlined.
    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }
}

/// UTF-16 offsets of every byte offset of `text` (inside a character: that
/// character's start), plus the end.
fn utf16_map(text: &str) -> Vec<usize> {
    let mut map = Vec::with_capacity(text.len() + 1);
    let mut units = 0;
    for c in text.chars() {
        map.extend(std::iter::repeat_n(units, c.len_utf8()));
        units += c.len_utf16();
    }
    map.push(units);
    map
}

impl Inner {
    /// Styles the text with `runs`: what differs from the runs shown before
    /// (and the `edited` UTF-16 range) is reset to the editor's font and
    /// colour, then given its runs' styles. `None` restyles everything.
    fn apply_styles(&self, runs: Vec<(Range<usize>, TextStyle)>, edited: Option<NSRange>) {
        // SAFETY: the text view's own storage, on the main thread.
        let Some(storage) = (unsafe { self.view.textStorage() }) else { return };
        let text = storage.string().to_string();
        let map = utf16_map(&text);
        let len16 = *map.last().unwrap_or(&0);
        let shown = self.shown.borrow_mut().take();
        // The span whose runs changed: runs equal before it (a prefix) and,
        // shifted by the edit, after it (a suffix) keep their styles.
        let region = match (shown, edited) {
            (Some((old_len, old)), Some(edited)) => {
                let delta = text.len() as isize - old_len as isize;
                let shift = |r: &Range<usize>| (r.start as isize + delta) as usize..(r.end as isize + delta) as usize;
                let prefix = old.iter().zip(&runs).take_while(|(a, b)| a == b).count();
                let suffix = old[prefix..]
                    .iter()
                    .rev()
                    .zip(runs[prefix..].iter().rev())
                    .take_while(|(a, b)| a.1 == b.1 && a.0.start as isize + delta >= 0 && shift(&a.0) == b.0)
                    .count();
                let lo = if prefix > 0 { runs[prefix - 1].0.end.min(text.len()) } else { 0 };
                let hi = if suffix > 0 { runs[runs.len() - suffix].0.start.min(text.len()) } else { text.len() };
                let (lo, hi) = (map[lo].min(edited.location), map[hi.max(lo)].max(edited.location + edited.length).min(len16));
                lo..hi
            }
            _ => 0..len16,
        };
        let font = self.font.borrow().clone();
        self.styling.set(true);
        // SAFETY: attribute keys and values of the documented types (fonts,
        // colours, an NSNumber for the underline style), in range.
        unsafe {
            use objc2_app_kit::{NSBackgroundColorAttributeName, NSFontAttributeName, NSForegroundColorAttributeName, NSUnderlineStyleAttributeName};
            storage.beginEditing();
            let base = objc2_foundation::NSDictionary::from_retained_objects(
                &[NSFontAttributeName, NSForegroundColorAttributeName],
                &[Retained::into_super(Retained::into_super(font.clone())), Retained::into_super(Retained::into_super(NSColor::textColor()))],
            );
            storage.setAttributes_range(Some(&base), NSRange::new(region.start, region.end - region.start));
            let fonts = objc2_app_kit::NSFontManager::sharedFontManager(mtm());
            for (range, style) in &runs {
                let (start, end) = (map[range.start.min(text.len())].max(region.start), map[range.end.min(text.len())].min(region.end));
                if start >= end {
                    continue;
                }
                let r = NSRange::new(start, end - start);
                if let Some(c) = style.color {
                    storage.addAttribute_value_range(NSForegroundColorAttributeName, &c.ns(), r);
                }
                if let Some(c) = style.background {
                    storage.addAttribute_value_range(NSBackgroundColorAttributeName, &c.ns(), r);
                }
                if style.bold || style.italic {
                    let mut f = font.clone();
                    if style.bold {
                        f = fonts.convertFont_toHaveTrait(&f, objc2_app_kit::NSFontTraitMask::BoldFontMask);
                    }
                    if style.italic {
                        f = fonts.convertFont_toHaveTrait(&f, objc2_app_kit::NSFontTraitMask::ItalicFontMask);
                    }
                    storage.addAttribute_value_range(NSFontAttributeName, &f, r);
                }
                if style.underline {
                    storage.addAttribute_value_range(NSUnderlineStyleAttributeName, &objc2_foundation::NSNumber::new_isize(1), r);
                }
            }
            storage.endEditing();
        }
        self.styling.set(false);
        *self.shown.borrow_mut() = Some((text.len(), runs));
        // What's typed next starts plain (the highlighter restyles it).
        // SAFETY: a dictionary of standard attributes.
        unsafe {
            use objc2_app_kit::{NSFontAttributeName, NSForegroundColorAttributeName};
            let typing = objc2_foundation::NSDictionary::from_retained_objects(
                &[NSFontAttributeName, NSForegroundColorAttributeName],
                &[Retained::into_super(Retained::into_super(font)), Retained::into_super(Retained::into_super(NSColor::textColor()))],
            );
            self.view.setTypingAttributes(&typing);
        }
    }
}

impl Data for Inner {
    fn push(&self, event: TextEvent) {
        let mut events = self.events.borrow_mut();
        // `Changed` means "read the text again": one queued is enough (an
        // edit is reported by both the text storage and textDidChange:).
        if event == TextEvent::Changed && events.contains(&TextEvent::Changed) {
            return;
        }
        // Keep only the latest of a run of selection moves.
        if let (TextEvent::Selection(_), Some(TextEvent::Selection(_))) = (&event, events.back()) {
            events.pop_back();
        }
        events.push_back(event);
        drop(events);
        crate::request_redraw();
    }

    fn quiet(&self) -> bool {
        self.quiet.get()
    }

    fn composing(&self) -> bool {
        self.view.hasMarkedText()
    }

    fn undo_manager(&self) -> Retained<NSUndoManager> {
        self.undo.borrow().clone()
    }

    fn highlight(&self, edited: Option<NSRange>) {
        if self.styling.get() {
            return;
        }
        let Some(f) = self.highlighter.borrow().clone() else { return };
        let runs = f(&self.view.string().to_string());
        self.apply_styles(runs, edited);
    }
}

impl Drop for ViewIvars {
    fn drop(&mut self) {
        crate::testing::LIVE_OBJC.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for DelegateIvars {
    fn drop(&mut self) {
        crate::testing::LIVE_OBJC.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // The text view and its storage hold the delegate weakly: detach it first.
        self.view.setDelegate(None);
        if let Some(storage) = unsafe { self.view.textStorage() } {
            storage.setDelegate(None);
        }
        crate::testing::LIVE_MODELS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        crate::keys::set_accepts_tab(self.id, false);
        if self.mounted.get() {
            crate::unmount(self.id);
        }
    }
}

/// A native plain-text editor. Cheap to clone (a handle); main thread only.
#[derive(Clone)]
pub struct TextModel {
    inner: Rc<Inner>,
}

impl std::fmt::Debug for TextModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextModel({:?})", self.inner.id)
    }
}

impl TextModel {
    /// Creates the editor (added to the window the first time the host draws
    /// it). Defaults: plain text, system font 13 pt, continuous spell
    /// checking, find bar, undo, Tab inserts a tab (Ctrl-Tab moves focus on).
    pub fn new(id: ViewId, text: &str) -> Self {
        let mtm = mtm();
        let size = NSSize::new(400.0, 300.0);
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::new(NSPoint::ZERO, size));
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setBorderType(objc2_app_kit::NSBorderType::NoBorder);

        // The standard recipe for a text view in a scroll view: width tracks
        // the scroll view, height grows with the text.
        crate::testing::LIVE_OBJC.fetch_add(2, std::sync::atomic::Ordering::Relaxed);
        crate::testing::LIVE_MODELS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let view = HostTextView::alloc(mtm).set_ivars(ViewIvars { data: RefCell::new(None), composing: Cell::new(false) });
        let view: Retained<HostTextView> = unsafe { msg_send![super(view), initWithFrame: NSRect::new(NSPoint::ZERO, size)] };
        view.setMinSize(NSSize::new(0.0, size.height));
        view.setMaxSize(NSSize::new(f64::MAX, f64::MAX));
        view.setVerticallyResizable(true);
        view.setHorizontallyResizable(false);
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        // SAFETY: the text view's own container.
        if let Some(container) = unsafe { view.textContainer() } {
            container.setContainerSize(NSSize::new(size.width, f64::MAX));
            container.setWidthTracksTextView(true);
        }
        view.setRichText(false);
        view.setImportsGraphics(false);
        view.setAllowsUndo(true);
        view.setUsesFindBar(true);
        view.setContinuousSpellCheckingEnabled(true);
        view.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        view.setTextContainerInset(NSSize::new(4.0, 6.0));
        view.setString(&NSString::from_str(text));
        scroll.setDocumentView(Some(&view));

        let delegate = Delegate::alloc(mtm).set_ivars(DelegateIvars { data: RefCell::new(None) });
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };

        let inner = Rc::new(Inner {
            id,
            events: RefCell::new(VecDeque::new()),
            quiet: Cell::new(false),
            mounted: Cell::new(false),
            undo: RefCell::new(NSUndoManager::new(mtm)),
            shared: Cell::new(false),
            scroll,
            view: view.clone(),
            _delegate: delegate.clone(),
            font: RefCell::new(NSFont::systemFontOfSize(13.0)),
            highlighter: RefCell::new(None),
            styling: Cell::new(false),
            shown: RefCell::new(None),
        });
        let weak: Weak<dyn Data> = Rc::downgrade(&inner) as Weak<dyn Data>;
        *view.ivars().data.borrow_mut() = Some(weak.clone());
        *delegate.ivars().data.borrow_mut() = Some(weak);
        // SAFETY: the delegate lives as long as the model, which removes the view.
        view.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        if let Some(storage) = unsafe { view.textStorage() } {
            // Weak in AppKit, like the view's delegate; detached in Drop.
            storage.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }
        crate::keys::set_accepts_tab(id, true);
        Self { inner }
    }

    /// The view id the editor was created with.
    pub fn id(&self) -> ViewId {
        self.inner.id
    }

    /// Adds the editor to the attached window if it isn't yet (the host
    /// widget calls this before placing it).
    pub fn ensure_mounted(&self) {
        if !self.inner.mounted.get() && crate::is_attached() {
            crate::mount(self.inner.id, Retained::into_super(self.inner.scroll.clone()));
            self.inner.mounted.set(true);
        }
    }

    /// The whole text.
    pub fn text(&self) -> String {
        self.inner.view.string().to_string()
    }

    /// Replaces the whole text (e.g. loading a document). Clears the undo
    /// history and emits no event.
    pub fn set_text(&self, text: &str) {
        self.inner.quiet.set(true);
        self.inner.view.setString(&NSString::from_str(text));
        self.inner.quiet.set(false);
        self.clear_undo();
    }

    /// Removes this editor's edits from its undo history (only this
    /// editor's, when the history is shared).
    fn clear_undo(&self) {
        let undo = self.inner.undo.borrow();
        if self.inner.shared.get() {
            // AppKit registers typing with the text storage as the target.
            unsafe {
                undo.removeAllActionsWithTarget(&self.inner.view);
                if let Some(storage) = self.inner.view.textStorage() {
                    undo.removeAllActionsWithTarget(&storage);
                }
            }
        } else {
            undo.removeAllActions();
        }
    }

    /// Records edits in `manager` (e.g. the window's, shared with other
    /// views and app-level changes) instead of the editor's own history, or
    /// in a fresh history of its own again with `None`. Edits recorded so
    /// far are dropped.
    pub fn share_undo(&self, manager: Option<&NSUndoManager>) {
        self.clear_undo();
        let mtm = MainThreadMarker::from(&*self.inner.view);
        let next = match manager {
            Some(m) => m.retain(),
            None => NSUndoManager::new(mtm),
        };
        *self.inner.undo.borrow_mut() = next;
        self.inner.shared.set(manager.is_some());
    }

    /// The undo history the editor records in.
    pub fn undo_manager(&self) -> Retained<NSUndoManager> {
        self.inner.undo.borrow().clone()
    }

    /// Replaces the selection as if typed: undoable, emits `Changed`.
    pub fn replace_selection(&self, text: &str) {
        let view = &self.inner.view;
        let range = view.selectedRange();
        let string = NSString::from_str(text);
        if view.shouldChangeTextInRange_replacementString(range, Some(&string)) {
            view.replaceCharactersInRange_withString(range, &string);
            view.didChangeText();
        }
    }

    /// Selected byte range (empty: the caret).
    pub fn selection(&self) -> Range<usize> {
        let r = self.inner.view.selectedRange();
        utf16_to_bytes(&self.text(), r.location, r.length)
    }

    /// Selects a byte range (emits no event).
    pub fn set_selection(&self, range: Range<usize>) {
        let (location, length) = bytes_to_utf16(&self.text(), range);
        self.inner.quiet.set(true);
        self.inner.view.setSelectedRange(NSRange::new(location, length));
        self.inner.view.scrollRangeToVisible(NSRange::new(location, length));
        self.inner.quiet.set(false);
    }

    /// Whether an input method is composing, and its marked text.
    pub fn composition(&self) -> Option<String> {
        let view = &self.inner.view;
        view.hasMarkedText().then(|| {
            let r = view.markedRange();
            utf16_slice(&view.string().to_string(), r.location, r.length)
        })
    }

    /// Read-only (`false`) or editable. Read-only text can still be selected and copied.
    pub fn set_editable(&self, editable: bool) {
        self.inner.view.setEditable(editable);
    }

    /// Whether the user can edit the text.
    pub fn is_editable(&self) -> bool {
        self.inner.view.isEditable()
    }

    /// Font: `None` for the system font, else a family or PostScript name.
    pub fn set_font(&self, name: Option<&str>, size: f64) {
        let font = match name {
            Some(name) => NSFont::fontWithName_size(&NSString::from_str(name), size),
            None => Some(NSFont::systemFontOfSize(size)),
        };
        if let Some(font) = font {
            self.inner.view.setFont(Some(&font));
            *self.inner.font.borrow_mut() = font;
            self.inner.highlight(None);
        }
    }

    /// Styles the text with `f`: given the text, it returns runs (UTF-8
    /// byte ranges) and their styles; the rest is plain. It runs now and
    /// after every edit (typing, paste, undo, `set_text`), before the edit
    /// is drawn, so keep it fast: it sees the whole text each time. `None`
    /// makes the text plain again.
    pub fn set_highlighter(&self, f: Option<impl Fn(&str) -> Vec<(Range<usize>, TextStyle)> + 'static>) {
        *self.inner.highlighter.borrow_mut() = f.map(|f| Rc::new(f) as Rc<Highlighter>);
        self.restyle();
    }

    /// Runs the highlighter again, for when what it depends on changed
    /// other than the text (a search term, a theme).
    pub fn restyle(&self) {
        if self.inner.highlighter.borrow().is_some() {
            self.inner.highlight(None);
        } else {
            self.inner.apply_styles(Vec::new(), None);
        }
    }

    /// The style of the character at byte `offset`, as shown (for checks).
    pub fn style_at(&self, offset: usize) -> TextStyle {
        // SAFETY: the text view's own storage, on the main thread.
        let Some(storage) = (unsafe { self.inner.view.textStorage() }) else { return TextStyle::default() };
        let text = storage.string().to_string();
        let Some(&index) = utf16_map(&text).get(offset).filter(|_| offset < text.len()) else { return TextStyle::default() };
        let mut style = TextStyle::default();
        // SAFETY: reading standard attributes at an index inside the text.
        unsafe {
            use objc2_app_kit::{NSBackgroundColorAttributeName, NSFontAttributeName, NSForegroundColorAttributeName, NSUnderlineStyleAttributeName};
            let at = |key| storage.attribute_atIndex_effectiveRange(key, index, std::ptr::null_mut());
            style.color = at(NSForegroundColorAttributeName)
                .and_then(|c| c.downcast::<NSColor>().ok())
                .and_then(|c| TextColor::from_ns(&c))
                .filter(|c| *c != TextColor::Primary);
            style.background = at(NSBackgroundColorAttributeName).and_then(|c| c.downcast::<NSColor>().ok()).and_then(|c| TextColor::from_ns(&c));
            if let Some(font) = at(NSFontAttributeName).and_then(|f| f.downcast::<NSFont>().ok()) {
                let traits = objc2_app_kit::NSFontManager::sharedFontManager(mtm()).traitsOfFont(&font);
                style.bold = traits.contains(objc2_app_kit::NSFontTraitMask::BoldFontMask);
                style.italic = traits.contains(objc2_app_kit::NSFontTraitMask::ItalicFontMask);
            }
            style.underline = at(NSUnderlineStyleAttributeName)
                .and_then(|n| n.downcast::<objc2_foundation::NSNumber>().ok())
                .is_some_and(|n| n.integerValue() != 0);
        }
        style
    }

    /// The current font size in points.
    pub fn font_size(&self) -> Option<f64> {
        self.inner.view.font().map(|f| f.pointSize())
    }

    /// Space between the text and the view's edges, in points.
    pub fn set_insets(&self, horizontal: f64, vertical: f64) {
        self.inner.view.setTextContainerInset(NSSize::new(horizontal, vertical));
    }

    /// Continuous spell checking (on by default).
    pub fn set_spell_checking(&self, on: bool) {
        self.inner.view.setContinuousSpellCheckingEnabled(on);
    }

    /// Grammar checking (off by default).
    pub fn set_grammar_checking(&self, on: bool) {
        self.inner.view.setGrammarCheckingEnabled(on);
    }

    /// Whether plain Tab inserts a tab (default) or moves focus on.
    pub fn set_accepts_tab(&self, accepts: bool) {
        crate::keys::set_accepts_tab(self.inner.id, accepts);
    }

    /// Shows the find bar (as ⌘F does).
    pub fn show_find(&self) {
        // NSTextFinderActionShowFindInterface = 1, passed as the sender's tag.
        let item = objc2_app_kit::NSMenuItem::new(mtm());
        item.setTag(1);
        let () = unsafe { msg_send![&*self.inner.view, performTextFinderAction: &*item] };
    }

    /// Whether the find bar is showing.
    pub fn is_find_visible(&self) -> bool {
        unsafe { msg_send![&*self.inner.scroll, isFindBarVisible] }
    }

    /// Whether there is something to undo.
    pub fn can_undo(&self) -> bool {
        self.inner.undo.borrow().canUndo()
    }

    /// Undoes the last edit (as ⌘Z does).
    pub fn undo(&self) {
        let undo = self.undo_manager();
        if undo.canUndo() {
            undo.undo();
        }
    }

    /// Takes the queued user events (for the host widget).
    pub fn drain_events(&self) -> Vec<TextEvent> {
        self.inner.events.borrow_mut().drain(..).collect()
    }

    /// The `NSScrollView` holding the text view, for hosts that place it
    /// themselves (no [`crate::attach`]ed window).
    pub fn scroll_view(&self) -> Retained<NSScrollView> {
        self.inner.scroll.clone()
    }

    /// The underlying `NSTextView` (escape hatch).
    pub fn text_view(&self) -> Retained<NSTextView> {
        Retained::into_super(self.inner.view.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{bytes_to_utf16, utf16_to_bytes};

    #[test]
    fn offsets_round_trip() {
        let t = "a東京😀b";
        // a=1 unit/1 byte, 東,京=1 unit/3 bytes, 😀=2 units/4 bytes.
        assert_eq!(utf16_to_bytes(t, 1, 2), 1..7);
        assert_eq!(utf16_to_bytes(t, 3, 2), 7..11);
        assert_eq!(bytes_to_utf16(t, 7..11), (3, 2));
        assert_eq!(bytes_to_utf16(t, 0..t.len()), (0, 6));
        assert_eq!(utf16_to_bytes(t, 6, 0), 12..12);
        assert_eq!(utf16_to_bytes(t, 99, 1), 12..12);
    }
}
