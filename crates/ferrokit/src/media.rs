//! Web pages, PDFs and video: WebKit's `WKWebView`, PDFKit's `PDFView` and
//! AVKit's `AVPlayerView`, each bound to signals.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSView, NSWorkspace};
use objc2_av_foundation::{AVPlayer, AVPlayerItem, AVPlayerItemDidPlayToEndTimeNotification};
use objc2_av_kit::{AVPlayerView, AVPlayerViewControlsStyle};
use objc2_core_media::CMTime;
use objc2_foundation::{NSData, NSError, NSObjectNSKeyValueObserverRegistration, NSNotification, NSNotificationCenter, NSOperationQueue, NSRect, NSString, NSStringCompareOptions, NSURL, NSURLRequest};
use objc2_pdf_kit::{PDFDisplayMode, PDFDocument, PDFSelection, PDFView, PDFViewPageChangedNotification};
use objc2_web_kit::{WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationType, WKWebView, WKWebViewConfiguration};

use crate::signal::{Effect, Signal, batch, untrack};
use crate::view::{Cx, Node, Size, View};

fn as_view<T: objc2::ClassType>(v: &Retained<T>) -> Retained<NSView> {
    // SAFETY: only called with NSView subclasses.
    unsafe { Retained::cast_unchecked(v.clone()) }
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

/// A block-based notification observer, removed when dropped.
pub(crate) struct Observer(Retained<ProtocolObject<dyn NSObjectProtocol>>);

impl Observer {
    pub(crate) fn new(name: &objc2_foundation::NSNotificationName, object: Option<&AnyObject>, f: impl Fn() + 'static) -> Observer {
        let block = RcBlock::new(move |_: NonNull<NSNotification>| crate::failure::entry("a notification", (), &f));
        // SAFETY: delivered on the main queue, where the block's state lives.
        let token = unsafe { NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(Some(name), object, Some(&NSOperationQueue::mainQueue()), &block) };
        Observer(token)
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        // SAFETY: removing a token this module registered.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(self.0.as_ref()) };
    }
}

// ---- web ----

enum Load {
    Url(String),
    Html(String, Option<String>),
    File(PathBuf),
}

struct PageState {
    view: RefCell<Option<objc2::rc::Weak<WKWebView>>>,
    pending: RefCell<Option<Load>>,
    title: Signal<String>,
    url: Signal<String>,
    loading: Signal<bool>,
    back: Signal<bool>,
    forward: Signal<bool>,
}

/// A web page shown by a [`web_view`]: what it loads, and its title,
/// address and loading state as signals. It's SwiftUI's `WebPage`.
///
/// ```no_run
/// use ferrokit::*;
///
/// let page = WebPage::new();
/// page.load("https://www.rust-lang.org");
/// let browser = vstack((
///     hstack((
///         button("Back", { let p = page.clone(); move || p.back() }),
///         text({ let p = page.clone(); move || p.title() }),
///     )),
///     web_view(&page),
/// ));
/// # let _ = browser;
/// ```
#[derive(Clone)]
pub struct WebPage(Rc<PageState>);

impl Default for WebPage {
    fn default() -> Self {
        Self::new()
    }
}

impl WebPage {
    /// An empty page; load something with [`load`](Self::load),
    /// [`load_html`](Self::load_html) or [`load_file`](Self::load_file).
    pub fn new() -> Self {
        WebPage(Rc::new(PageState {
            view: RefCell::new(None),
            pending: RefCell::new(None),
            title: Signal::new(String::new()),
            url: Signal::new(String::new()),
            loading: Signal::new(false),
            back: Signal::new(false),
            forward: Signal::new(false),
        }))
    }

    fn view(&self) -> Option<Retained<WKWebView>> {
        self.0.view.borrow().as_ref().and_then(|w| w.load())
    }

    fn start(&self, load: Load) {
        match self.view() {
            Some(v) => run_load(&v, load),
            None => *self.0.pending.borrow_mut() = Some(load),
        }
    }

    /// Loads a web address (`https://…`) or a `file://` URL.
    pub fn load(&self, url: &str) {
        self.start(Load::Url(url.into()));
    }

    /// Shows `html`. Relative links and images resolve against `base_url`
    /// (a web address, or a `file://` folder the page may read).
    pub fn load_html(&self, html: &str, base_url: Option<&str>) {
        self.start(Load::Html(html.into(), base_url.map(Into::into)));
    }

    /// Shows a local HTML file, allowed to read the files in its folder.
    pub fn load_file(&self, path: impl AsRef<Path>) {
        self.start(Load::File(path.as_ref().to_path_buf()));
    }

    /// The page's title (a signal read: views showing it follow it).
    pub fn title(&self) -> String {
        self.0.title.get()
    }

    /// The page's address (a signal read).
    pub fn url(&self) -> String {
        self.0.url.get()
    }

    /// Whether the page is loading (a signal read).
    pub fn is_loading(&self) -> bool {
        self.0.loading.get()
    }

    /// Whether there's a page to go back to (a signal read).
    pub fn can_go_back(&self) -> bool {
        self.0.back.get()
    }

    /// Whether there's a page to go forward to (a signal read).
    pub fn can_go_forward(&self) -> bool {
        self.0.forward.get()
    }

    /// Goes back a page.
    pub fn back(&self) {
        // SAFETY: plain WebKit navigation on the main thread.
        self.view().map(|v| unsafe { v.goBack() });
    }

    /// Goes forward a page.
    pub fn forward(&self) {
        // SAFETY: as above.
        self.view().map(|v| unsafe { v.goForward() });
    }

    /// Loads the page again.
    pub fn reload(&self) {
        // SAFETY: as above.
        self.view().map(|v| unsafe { v.reload() });
    }

    /// Runs `script` in the page, then calls `then` with its result as text
    /// (a string's value, a number's digits) or the error's description.
    pub fn evaluate(&self, script: &str, then: impl FnOnce(Result<String, String>) + 'static) {
        let Some(v) = self.view() else {
            then(Err("the web view isn't showing".into()));
            return;
        };
        let then = Cell::new(Some(then));
        let block = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
            let Some(then) = then.take() else { return };
            // SAFETY: WebKit passes valid objects or null.
            let out = unsafe {
                match (result.as_ref(), error.as_ref()) {
                    (_, Some(e)) => Err(e.localizedDescription().to_string()),
                    (Some(r), None) => Ok(match r.downcast_ref::<NSString>() {
                        Some(s) => s.to_string(),
                        None => {
                            let d: Retained<NSString> = msg_send![r, description];
                            d.to_string()
                        }
                    }),
                    (None, None) => Ok(String::new()),
                }
            };
            crate::failure::entry("a web page's script result", (), || then(out));
        });
        // SAFETY: WebKit calls the block on the main thread.
        unsafe { v.evaluateJavaScript_completionHandler(&NSString::from_str(script), Some(&block)) };
    }

    /// Reads the web view's state into the signals.
    fn sync(&self, v: &WKWebView) {
        // SAFETY: plain WebKit properties on the main thread.
        let (title, url, loading, back, forward) = unsafe { (v.title(), v.URL(), v.isLoading(), v.canGoBack(), v.canGoForward()) };
        let s = &self.0;
        batch(|| {
            s.title.set_if_changed(title.map(|t| t.to_string()).unwrap_or_default());
            s.url.set_if_changed(url.and_then(|u| u.absoluteString()).map(|u| u.to_string()).unwrap_or_default());
            s.loading.set_if_changed(loading);
            s.back.set_if_changed(back);
            s.forward.set_if_changed(forward);
        });
    }
}

fn run_load(v: &WKWebView, load: Load) {
    // SAFETY: WebKit's loading methods on the main thread.
    unsafe {
        match load {
            Load::Url(url) => {
                let Some(u) = NSURL::URLWithString(&NSString::from_str(&url)) else { return };
                if u.isFileURL() {
                    let dir = u.URLByDeletingLastPathComponent().unwrap_or_else(|| u.clone());
                    v.loadFileURL_allowingReadAccessToURL(&u, &dir);
                } else {
                    v.loadRequest(&NSURLRequest::requestWithURL(&u));
                }
            }
            Load::Html(html, base) => {
                let base = base.and_then(|b| NSURL::URLWithString(&NSString::from_str(&b)));
                v.loadHTMLString_baseURL(&NSString::from_str(&html), base.as_deref());
            }
            Load::File(path) => {
                let u = file_url(&path);
                let dir = file_url(path.parent().unwrap_or(Path::new("/")));
                v.loadFileURL_allowingReadAccessToURL(&u, &dir);
            }
        }
    }
}

struct WebState {
    page: Weak<PageState>,
    links_outside: bool,
}

define_class!(
    // Follows a web view's navigation into its page's signals, and sends
    // links out to the browser when asked.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitWebDelegate"]
    #[ivars = WebState]
    struct WebDelegate;

    unsafe impl NSObjectProtocol for WebDelegate {}

    impl WebDelegate {
        // The title and the rest change after navigation callbacks too
        // (a title arrives once the page has parsed it).
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(&self, _key: Option<&NSString>, object: Option<&AnyObject>, _change: Option<&AnyObject>, _context: *mut std::ffi::c_void) {
            crate::failure::guard("a web view (observe)", (), || {
                if let Some(view) = object.and_then(|o| o.downcast_ref::<WKWebView>()) {
                    self.sync(view);
                }
            })
        }
    }

    unsafe impl WKNavigationDelegate for WebDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide(&self, view: &WKWebView, action: &WKNavigationAction, handler: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>) {
            crate::failure::guard("a web view (decide)", (), || {
                // SAFETY: reading the action's properties on the main thread.
                let (kind, request, frame) = unsafe { (action.navigationType(), action.request(), action.targetFrame()) };
                let url = request.URL();
                let link = kind == WKNavigationType::LinkActivated || frame.is_none();
                if link && let Some(url) = url {
                    if self.ivars().links_outside && !url.isFileURL() {
                        NSWorkspace::sharedWorkspace().openURL(&url);
                        handler.call((WKNavigationActionPolicy::Cancel,));
                        return;
                    }
                    if frame.is_none() {
                        // A link to a new window (`target="_blank"`): opened here.
                        // SAFETY: as above.
                        unsafe { view.loadRequest(&request) };
                        handler.call((WKNavigationActionPolicy::Cancel,));
                        return;
                    }
                }
                handler.call((WKNavigationActionPolicy::Allow,));
            })
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn started(&self, view: &WKWebView, _n: Option<&WKNavigation>) {
            crate::failure::guard("a web view (started)", (), || {
                self.sync(view);
            })
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn committed(&self, view: &WKWebView, _n: Option<&WKNavigation>) {
            crate::failure::guard("a web view (committed)", (), || {
                self.sync(view);
            })
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn finished(&self, view: &WKWebView, _n: Option<&WKNavigation>) {
            crate::failure::guard("a web view (finished)", (), || {
                self.sync(view);
            })
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed(&self, view: &WKWebView, _n: Option<&WKNavigation>, _e: &NSError) {
            crate::failure::guard("a web view (failed)", (), || {
                self.sync(view);
            })
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn failed_early(&self, view: &WKWebView, _n: Option<&WKNavigation>, _e: &NSError) {
            crate::failure::guard("a web view (failed_early)", (), || {
                self.sync(view);
            })
        }
    }
);

impl WebDelegate {
    fn sync(&self, view: &WKWebView) {
        if let Some(page) = self.ivars().page.upgrade() {
            WebPage(page).sync(view);
        }
    }
}

const KEYS: [&str; 5] = ["title", "URL", "loading", "canGoBack", "canGoForward"];

/// Stops observing the web view when its node goes.
struct Observing {
    view: Retained<WKWebView>,
    delegate: Retained<WebDelegate>,
}

impl Drop for Observing {
    fn drop(&mut self) {
        for key in KEYS {
            // SAFETY: removing observations `build` added.
            unsafe { self.view.removeObserver_forKeyPath(&self.delegate, &NSString::from_str(key)) };
        }
    }
}

/// A web view (see [`web_view`]).
pub struct WebView {
    page: WebPage,
    links_outside: bool,
}

impl WebView {
    /// Opens links the user clicks in their web browser instead of in the
    /// view: for help pages, previews and HTML the app made itself.
    pub fn links_open_in_browser(mut self) -> Self {
        self.links_outside = true;
        self
    }
}

impl View for WebView {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        // SAFETY: WebKit's designated initialiser with a fresh configuration.
        let view = unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), NSRect::ZERO, &WKWebViewConfiguration::new(mtm)) };
        let delegate = WebDelegate::alloc(mtm).set_ivars(WebState { page: Rc::downgrade(&self.page.0), links_outside: self.links_outside });
        // SAFETY: NSObject's designated initialiser.
        let delegate: Retained<WebDelegate> = unsafe { msg_send![super(delegate), init] };
        // SAFETY: weak in WebKit; the node keeps the delegate.
        unsafe { view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        *self.page.0.view.borrow_mut() = Some(objc2::rc::Weak::from_retained(&view));
        if let Some(load) = self.page.0.pending.borrow_mut().take() {
            run_load(&view, load);
        }
        for key in KEYS {
            // SAFETY: the observer answers observeValueForKeyPath:; Observing
            // removes it before either goes.
            unsafe { view.addObserver_forKeyPath_options_context(&delegate, &NSString::from_str(key), objc2_foundation::NSKeyValueObservingOptions::empty(), std::ptr::null_mut()) };
        }
        let (mut node, _) = Node::leaf(cx, as_view(&view), (true, true), Size { w: 200.0, h: 150.0 });
        node.hold(Rc::new(Observing { view, delegate }));
        node
    }
}

/// A web view showing `page`: a real `WKWebView`, with the system's
/// scrolling, text selection, find and Look Up. Its size is flexible.
pub fn web_view(page: &WebPage) -> WebView {
    WebView { page: page.clone(), links_outside: false }
}

// ---- PDF ----

enum PdfSource {
    File(Box<dyn Fn() -> Option<PathBuf>>),
    Data(Box<dyn Fn() -> Vec<u8>>),
}

/// A PDF view (see [`pdf_view`]).
pub struct PdfView {
    source: PdfSource,
    page: Option<Signal<usize>>,
    pages: Option<Signal<usize>>,
    find: Option<Box<dyn Fn() -> String>>,
    mode: PDFDisplayMode,
}

impl PdfView {
    /// The page shown, from 0, bound two ways: scrolling changes it, and
    /// setting it goes to that page.
    pub fn page(mut self, page: &Signal<usize>) -> Self {
        self.page = Some(*page);
        self
    }

    /// Sets `count` to the document's number of pages.
    pub fn page_count(mut self, count: &Signal<usize>) -> Self {
        self.pages = Some(*count);
        self
    }

    /// Highlights every match of `query()` (ignoring case) and goes to the
    /// first, as Preview's search does.
    pub fn highlight(mut self, query: impl Fn() -> String + 'static) -> Self {
        self.find = Some(Box::new(query));
        self
    }

    /// One page at a time instead of a continuous scroll.
    pub fn single_page(mut self) -> Self {
        self.mode = PDFDisplayMode::SinglePage;
        self
    }

    /// Pages side by side, as a book's spread.
    pub fn two_up(mut self) -> Self {
        self.mode = PDFDisplayMode::TwoUpContinuous;
        self
    }
}

impl View for PdfView {
    fn build(self, cx: &mut Cx) -> Node {
        // SAFETY: NSView's designated initialiser.
        let view: Retained<PDFView> = unsafe { msg_send![PDFView::alloc(cx.mtm), initWithFrame: NSRect::ZERO] };
        // SAFETY: plain PDFView settings.
        unsafe {
            view.setAutoScales(true);
            view.setDisplayMode(self.mode);
        }
        let (mut node, _) = Node::leaf(cx, as_view(&view), (true, true), Size { w: 200.0, h: 200.0 });
        let (page, pages) = (self.page, self.pages);
        // The document, reloaded when the source's signals change.
        let v = view.clone();
        let source = self.source;
        node.own(Effect::new(move || {
            let doc = match &source {
                // SAFETY: PDFKit's initialisers; None when unreadable.
                PdfSource::File(f) => f().and_then(|p| unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &file_url(&p)) }),
                PdfSource::Data(f) => unsafe { PDFDocument::initWithData(PDFDocument::alloc(), &NSData::with_bytes(&f())) },
            };
            untrack(|| {
                // SAFETY: setting the view's document on the main thread.
                unsafe { v.setDocument(doc.as_deref()) };
                if let Some(c) = pages {
                    c.set_if_changed(doc.as_ref().map_or(0, |d| unsafe { d.pageCount() }));
                }
                if let (Some(p), Some(d)) = (page, &doc) {
                    let want = p.get().min(unsafe { d.pageCount() }.saturating_sub(1));
                    if let Some(pg) = unsafe { d.pageAtIndex(want) } {
                        unsafe { v.goToPage(&pg) };
                    }
                    p.set_if_changed(want);
                }
            });
        }));
        if let Some(p) = page {
            // Scrolling to another page sets the signal.
            let v = view.clone();
            // SAFETY: PDFKit's notification name.
            let observer = Observer::new(unsafe { PDFViewPageChangedNotification }, Some(&view), move || {
                // SAFETY: reading the view's state on the main thread.
                if let (Some(d), Some(cur)) = unsafe { (v.document(), v.currentPage()) } {
                    p.set_if_changed(unsafe { d.indexForPage(&cur) });
                }
            });
            node.hold(Rc::new(observer));
            let v = view.clone();
            node.own(Effect::new(move || {
                let want = p.get();
                // SAFETY: as above.
                unsafe {
                    let Some(d) = v.document() else { return };
                    let shown = v.currentPage().map(|c| d.indexForPage(&c));
                    if shown != Some(want)
                        && let Some(pg) = d.pageAtIndex(want.min(d.pageCount().saturating_sub(1)))
                    {
                        v.goToPage(&pg);
                    }
                }
            }));
        }
        if let Some(find) = self.find {
            let v = view.clone();
            node.own(Effect::new(move || {
                let q = find();
                // SAFETY: PDFKit's search on the main thread.
                unsafe {
                    let Some(d) = v.document() else { return };
                    if q.is_empty() {
                        v.setHighlightedSelections(None);
                        return;
                    }
                    let found = d.findString_withOptions(&NSString::from_str(&q), NSStringCompareOptions::CaseInsensitiveSearch);
                    v.setHighlightedSelections(Some(&found));
                    if let Some(first) = found.firstObject() {
                        go_to_match(&v, &first);
                    }
                }
            }));
        }
        node
    }
}

/// Scrolls `view` to a found match. A match PDFKit returns can lack its
/// position (bounds of infinity, seen on macOS 27 once an on-device model
/// was cached), and `goToSelection` then
/// stays where it is: the match is rebuilt from its text range, or placed
/// by its first character, or failing those (PDFKit sometimes has no
/// position for any of it) its page is shown.
fn go_to_match(view: &PDFView, found: &PDFSelection) {
    // SAFETY: PDFKit's selection and page API on the main thread.
    unsafe {
        let Some(page) = found.pages().firstObject() else { return };
        let placed = |s: &PDFSelection| finite(s.boundsForPage(&page));
        if placed(found) {
            return view.goToSelection(found);
        }
        if found.numberOfTextRangesOnPage(&page) > 0 {
            let range = found.rangeAtIndex_onPage(0, &page);
            if let Some(again) = page.selectionForRange(range)
                && placed(&again)
            {
                return view.goToSelection(&again);
            }
            let first = page.characterBoundsAtIndex(range.location as _);
            if finite(first) {
                return view.goToRect_onPage(first, &page);
            }
        }
        view.goToPage(&page);
    }
}

fn finite(r: objc2_foundation::NSRect) -> bool {
    [r.origin.x, r.origin.y, r.size.width, r.size.height].iter().all(|v| v.is_finite())
}

/// A PDF view showing the file at `path()`, reloaded when the signals it
/// reads change (`None` shows nothing). It's Preview's view: zoom, text
/// selection, copy and find. Its size is flexible.
pub fn pdf_view(path: impl Fn() -> Option<PathBuf> + 'static) -> PdfView {
    PdfView { source: PdfSource::File(Box::new(path)), page: None, pages: None, find: None, mode: PDFDisplayMode::SinglePageContinuous }
}

/// A PDF view showing the document in `data()` (from [`pdf_data`](crate::pdf_data), a
/// download or a database).
pub fn pdf_view_data(data: impl Fn() -> Vec<u8> + 'static) -> PdfView {
    PdfView { source: PdfSource::Data(Box::new(data)), page: None, pages: None, find: None, mode: PDFDisplayMode::SinglePageContinuous }
}

// ---- video ----

/// The playback controls of a [`video_player`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum VideoControls {
    /// A bar along the bottom, shown while the pointer is over the video.
    #[default]
    Inline,
    /// A floating panel, as QuickTime Player shows.
    Floating,
    /// Only a play button, for short clips.
    Minimal,
    /// None: the app plays and pauses it through
    /// [`playing`](VideoPlayer::playing).
    Hidden,
}

/// A video player (see [`video_player`]).
pub struct VideoPlayer {
    source: Box<dyn Fn() -> Option<Retained<NSURL>>>,
    playing: Option<Signal<bool>>,
    time: Option<Signal<f64>>,
    duration: Option<Signal<f64>>,
    muted: Option<Box<dyn Fn() -> bool>>,
    loops: bool,
    controls: VideoControls,
}

impl VideoPlayer {
    /// Whether it's playing, bound two ways: the controls set it, and
    /// setting it plays or pauses.
    pub fn playing(mut self, playing: &Signal<bool>) -> Self {
        self.playing = Some(*playing);
        self
    }

    /// The time played, in seconds, bound two ways: it follows playback
    /// (four times a second), and setting it jumps there.
    pub fn time(mut self, seconds: &Signal<f64>) -> Self {
        self.time = Some(*seconds);
        self
    }

    /// Sets `seconds` to the length of the video once it's known.
    pub fn duration(mut self, seconds: &Signal<f64>) -> Self {
        self.duration = Some(*seconds);
        self
    }

    /// Mutes the sound while `muted()` is true.
    pub fn muted(mut self, muted: impl Fn() -> bool + 'static) -> Self {
        self.muted = Some(Box::new(muted));
        self
    }

    /// Starts again from the beginning at the end.
    pub fn loops(mut self) -> Self {
        self.loops = true;
        self
    }

    /// The playback controls (inline by default).
    pub fn controls(mut self, controls: VideoControls) -> Self {
        self.controls = controls;
        self
    }
}

/// Removes the time observer when the node goes.
struct Watch {
    player: Retained<AVPlayer>,
    token: Retained<AnyObject>,
    _end: RefCell<Option<Observer>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: removing the observer this module added; pausing so the
        // sound stops with the view.
        unsafe {
            self.player.removeTimeObserver(&self.token);
            self.player.pause();
        }
    }
}

fn seconds(t: CMTime) -> Option<f64> {
    // SAFETY: a plain CMTime conversion.
    let s = unsafe { t.seconds() };
    s.is_finite().then_some(s)
}

impl View for VideoPlayer {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        // SAFETY: NSView's designated initialiser.
        let view: Retained<AVPlayerView> = unsafe { msg_send![AVPlayerView::alloc(mtm), initWithFrame: NSRect::ZERO] };
        // SAFETY: AVFoundation's player, made and used on the main thread.
        let player = unsafe { AVPlayer::new(mtm) };
        // SAFETY: plain AVKit settings.
        unsafe {
            view.setPlayer(Some(&player));
            view.setControlsStyle(match self.controls {
                VideoControls::Inline => AVPlayerViewControlsStyle::Inline,
                VideoControls::Floating => AVPlayerViewControlsStyle::Floating,
                VideoControls::Minimal => AVPlayerViewControlsStyle::Minimal,
                VideoControls::Hidden => AVPlayerViewControlsStyle::None,
            });
            view.setShowsFullScreenToggleButton(true);
        }
        let (mut node, _) = Node::leaf(cx, as_view(&view), (true, true), Size { w: 160.0, h: 90.0 });
        let (playing, time, duration, loops) = (self.playing, self.time, self.duration, self.loops);

        // Follows playback: time, duration and whether it's playing.
        let p = player.clone();
        let tick = RcBlock::new(move |now: CMTime| {
            // SAFETY: AVFoundation calls this on the main queue.
            let (rate, length) = unsafe { (p.rate(), p.currentItem().and_then(|i| seconds(i.duration()))) };
            crate::failure::entry("a video's time", (), || {
                if let (Some(t), Some(now)) = (time, seconds(now)) {
                    t.set_if_changed(now);
                }
                if let (Some(d), Some(length)) = (duration, length) {
                    d.set_if_changed(length);
                }
                if let Some(s) = playing {
                    s.set_if_changed(rate > 0.0);
                }
            });
        });
        // SAFETY: a main-queue observer (queue None), removed in Watch's drop.
        let token = unsafe { player.addPeriodicTimeObserverForInterval_queue_usingBlock(CMTime::with_seconds(0.25, 600), None, &tick) };
        let watch = Rc::new(Watch { player: player.clone(), token, _end: RefCell::new(None) });

        // The item, replaced when the source's signals change.
        let (p, w, source) = (player.clone(), Rc::downgrade(&watch), self.source);
        node.own(Effect::new(move || {
            let url = source();
            untrack(|| {
                // SAFETY: AVFoundation's player item on the main thread.
                let item = url.map(|u| unsafe { AVPlayerItem::playerItemWithURL(&u, mtm) });
                unsafe { p.replaceCurrentItemWithPlayerItem(item.as_deref()) };
                if let Some(d) = duration {
                    // A local file's length is known at once.
                    let length = item.as_ref().and_then(|i| unsafe { seconds(i.asset().duration()) });
                    d.set_if_changed(length.unwrap_or(0.0));
                }
                if let Some(w) = w.upgrade() {
                    // Plays again from the start at the end, or reports it stopped.
                    let p2 = p.clone();
                    *w._end.borrow_mut() = item.as_ref().map(|i| {
                        // SAFETY: AVFoundation's notification name.
                        Observer::new(unsafe { AVPlayerItemDidPlayToEndTimeNotification }, Some(i), move || unsafe {
                            if loops {
                                p2.seekToTime(CMTime::with_seconds(0.0, 600));
                                p2.play();
                            } else if let Some(s) = playing {
                                s.set_if_changed(false);
                            }
                        })
                    });
                }
                if playing.is_some_and(|s| s.get()) {
                    // SAFETY: as above.
                    unsafe { p.play() };
                }
            });
        }));
        if let Some(s) = playing {
            let p = player.clone();
            node.own(Effect::new(move || {
                let want = s.get();
                // SAFETY: as above.
                unsafe {
                    if want && p.rate() == 0.0 {
                        p.play();
                    } else if !want && p.rate() != 0.0 {
                        p.pause();
                    }
                }
            }));
        }
        if let Some(t) = time {
            let p = player.clone();
            node.own(Effect::new(move || {
                let want = t.get();
                // SAFETY: as above.
                let now = unsafe { seconds(p.currentTime()) }.unwrap_or(0.0);
                if (now - want).abs() > 0.5 {
                    unsafe { p.seekToTime(CMTime::with_seconds(want, 600)) };
                }
            }));
        }
        if let Some(m) = self.muted {
            let p = player.clone();
            // SAFETY: as above.
            node.own(Effect::new(move || unsafe { p.setMuted(m()) }));
        }
        node.hold(watch);
        node
    }
}

/// A video player for the file at `path()`, replaced when the signals it
/// reads change (`None` shows nothing): AVKit's player, with the system's
/// controls, Picture in Picture and full screen. Plays sound files too.
pub fn video_player(path: impl Fn() -> Option<PathBuf> + 'static) -> VideoPlayer {
    VideoPlayer {
        source: Box::new(move || path().map(|p| file_url(&p))),
        playing: None,
        time: None,
        duration: None,
        muted: None,
        loops: false,
        controls: VideoControls::Inline,
    }
}

/// A video player streaming from a web address (an HLS `.m3u8` playlist or
/// a plain video file on a server).
pub fn video_stream(url: impl Fn() -> Option<String> + 'static) -> VideoPlayer {
    let mut player = video_player(|| None);
    player.source = Box::new(move || url().and_then(|u| NSURL::URLWithString(&NSString::from_str(&u))));
    player
}
