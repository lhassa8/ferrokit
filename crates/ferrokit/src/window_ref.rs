//! [`WindowRef`]: a handle to one of the app's windows.

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use objc2_foundation::NSString;

/// One of the app's windows: what [`open`](crate::open),
/// [`windows`](crate::windows) and [`focused_window`](crate::focused_window)
/// return, and what the dialogs, document functions and checks take.
///
/// Cheap to clone (it's a reference to the window, not a copy), and two
/// handles are equal when they name the same window. It keeps the window
/// object alive, not open: after [`close`](Self::close) it's a closed
/// window that [`is_open`](Self::is_open) says so of.
///
/// ```no_run
/// use ferrokit::*;
///
/// let w = open(Window::new("Notes").size(400.0, 300.0).view(text("Hello")));
/// w.set_title("Notes — Draft");
/// assert!(windows().contains(&w));
/// ```
///
/// [`ns`](Self::ns) reaches the `NSWindow` underneath, for what Ferrokit
/// doesn't cover. Code that uses it follows objc2-app-kit's versions, not
/// Ferrokit's.
#[derive(Clone)]
pub struct WindowRef(Retained<NSWindow>);

impl WindowRef {
    pub(crate) fn new(window: Retained<NSWindow>) -> Self {
        WindowRef(window)
    }

    /// A handle to an `NSWindow` from AppKit (one a native view is in, say).
    pub fn from_ns(window: &NSWindow) -> Self {
        WindowRef(objc2::Message::retain(window))
    }

    /// The `NSWindow` underneath, for AppKit calls Ferrokit doesn't wrap.
    pub fn ns(&self) -> &NSWindow {
        &self.0
    }

    /// The window's title.
    pub fn title(&self) -> String {
        self.0.title().to_string()
    }

    /// Sets the window's title (a document's name, a count).
    pub fn set_title(&self, title: &str) {
        self.0.setTitle(&NSString::from_str(title));
    }

    /// Closes the window: [`close_window`](crate::close_window).
    pub fn close(&self) {
        crate::close_window(self);
    }

    /// Whether the window is still open: shown, minimised or in another
    /// Space (false once it's closed).
    pub fn is_open(&self) -> bool {
        crate::windows().contains(self) || self.0.isVisible()
    }

    /// The size of the window's content (its views' area, without the
    /// title bar), in points.
    pub fn size(&self) -> (f64, f64) {
        let frame = self.0.contentRectForFrameRect(self.0.frame());
        (frame.size.width, frame.size.height)
    }

    /// Resizes the window's content to `width` × `height` points, keeping
    /// its top left corner where it is (as dragging its bottom right
    /// corner does).
    pub fn set_size(&self, width: f64, height: f64) {
        self.0.setContentSize(objc2_foundation::NSSize::new(width, height));
    }

    /// Whether it's the key window (the one that gets typing).
    pub fn is_key(&self) -> bool {
        self.0.isKeyWindow()
    }

    /// Brings the window to the front and makes it key (and reopens it
    /// if it was hidden with its close button but kept, as Settings is).
    pub fn bring_to_front(&self) {
        self.0.makeKeyAndOrderFront(None);
    }
}

impl PartialEq for WindowRef {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(&*self.0, &*other.0)
    }
}

impl Eq for WindowRef {}

impl std::hash::Hash for WindowRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::hash(&*self.0, state);
    }
}

impl std::fmt::Debug for WindowRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("WindowRef").field(&self.title()).finish()
    }
}
