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

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ops::Range;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSFont, NSResponder, NSScrollView, NSText, NSTextDelegate, NSTextInputClient, NSTextStorage, NSTextStorageDelegate,
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
        fn did_process_editing(&self, _storage: &NSTextStorage, edited: NSTextStorageEditActions, _range: NSRange, _delta: isize) {
            if edited.contains(NSTextStorageEditActions::EditedCharacters)
                && let Some(data) = self.data().filter(|d| !d.quiet() && !d.composing())
            {
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
        }
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
