//! Quick Look: the preview panel Finder opens with Space ([`quick_look`]),
//! and file thumbnails ([`thumbnail`]), as Finder draws them.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSImage, NSImageScaling, NSImageView, NSWorkspace};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSError, NSInteger, NSRect, NSSize, NSString, NSURL};

use crate::signal::{Effect, untrack};
use crate::view::{Cx, Node, Size, View};

#[link(name = "QuickLookUI", kind = "framework")]
unsafe extern "C" {
    #[link_name = "OBJC_CLASS_$_QLPreviewPanel"]
    static PREVIEW_PANEL: AnyClass;
}

#[link(name = "QuickLookThumbnailing", kind = "framework")]
unsafe extern "C" {
    #[link_name = "OBJC_CLASS_$_QLThumbnailGenerator"]
    static GENERATOR: AnyClass;
    #[link_name = "OBJC_CLASS_$_QLThumbnailGenerationRequest"]
    static REQUEST: AnyClass;
}

fn file_url(path: &std::path::Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

// ---- the preview panel ----

define_class!(
    // The preview panel's data source: the files being previewed.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitQuickLookSource"]
    #[ivars = RefCell<Vec<Retained<NSURL>>>]
    struct Source;

    unsafe impl NSObjectProtocol for Source {}

    impl Source {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn count(&self, _panel: &AnyObject) -> NSInteger {
            self.ivars().borrow().len() as NSInteger
        }

        // NSURL is a QLPreviewItem.
        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn item(&self, _panel: &AnyObject, index: NSInteger) -> Option<Retained<NSURL>> {
            usize::try_from(index).ok().and_then(|i| self.ivars().borrow().get(i).cloned())
        }
    }
);

thread_local! {
    static SOURCE: RefCell<Option<Retained<Source>>> = const { RefCell::new(None) };
}

fn panel() -> Retained<AnyObject> {
    // SAFETY: QuickLookUI's shared panel, made on the main thread.
    unsafe { msg_send![&PREVIEW_PANEL, sharedPreviewPanel] }
}

/// The data source, if [`quick_look`] has previewed something (for the app
/// delegate, which takes control of the panel when it opens).
pub(crate) fn source() -> Option<Retained<NSObject>> {
    SOURCE.with_borrow(|s| s.clone().map(Retained::into_super))
}

/// Opens the Quick Look panel on `paths`, at `paths[index]`, as Space does
/// in Finder (or shows them in the panel if it's open): the arrow keys move
/// between them, and the panel offers Open With and Share.
pub fn quick_look(paths: Vec<PathBuf>, index: usize) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    crate::documents::install_delegate();
    let urls: Vec<Retained<NSURL>> = paths.iter().map(|p| file_url(p)).collect();
    let source = SOURCE.with_borrow_mut(|s| {
        s.get_or_insert_with(|| {
            let this = Source::alloc(mtm).set_ivars(RefCell::default());
            // SAFETY: NSObject's designated initialiser.
            unsafe { msg_send![super(this), init] }
        })
        .clone()
    });
    source.ivars().replace(urls);
    let panel = panel();
    // SAFETY: QLPreviewPanel's methods, on the main thread; the source is
    // kept above (the panel holds it weakly).
    unsafe {
        let _: () = msg_send![&panel, updateController];
        let _: () = msg_send![&panel, setDataSource: &*source];
        let _: () = msg_send![&panel, reloadData];
        let _: () = msg_send![&panel, setCurrentPreviewItemIndex: index as NSInteger];
        let visible: bool = msg_send![&panel, isVisible];
        if !visible {
            let _: () = msg_send![&panel, makeKeyAndOrderFront: Option::<&AnyObject>::None];
        }
    }
}

/// Closes the Quick Look panel, if it's open.
pub fn close_quick_look() {
    if !is_quick_look_open() {
        return;
    }
    // SAFETY: as in `quick_look`.
    unsafe {
        let _: () = msg_send![&panel(), orderOut: Option::<&AnyObject>::None];
    }
}

/// Whether the Quick Look panel is open.
pub fn is_quick_look_open() -> bool {
    // SAFETY: as in `quick_look`.
    unsafe {
        let exists: bool = msg_send![&PREVIEW_PANEL, sharedPreviewPanelExists];
        exists && msg_send![&panel(), isVisible]
    }
}

/// What the Quick Look panel shows: its files and the one shown (for checks).
pub(crate) fn quick_look_state() -> (Vec<PathBuf>, Option<usize>) {
    let files = SOURCE.with_borrow(|s| s.as_ref().map(|s| s.ivars().borrow().iter().filter_map(|u| u.path()).map(|p| PathBuf::from(p.to_string())).collect()).unwrap_or_default());
    if !is_quick_look_open() {
        return (files, None);
    }
    // SAFETY: as in `quick_look`.
    let index: NSInteger = unsafe { msg_send![&panel(), currentPreviewItemIndex] };
    (files, usize::try_from(index).ok())
}

// ---- thumbnails ----

thread_local! {
    pub(crate) static THUMBNAILS: Cell<usize> = const { Cell::new(0) };
}

/// A thumbnail as a Core Graphics image, sent from Quick Look's queue.
struct Picture(objc2_core_foundation::CFRetained<CGImage>);
// SAFETY: CGImage is immutable and documented as safe to use from any thread.
unsafe impl Send for Picture {}

/// Asks Quick Look for a thumbnail of `path` at `size` points (`scale`
/// pixels per point), waiting up to 10 seconds; on a worker thread.
fn generate(path: PathBuf, size: (f64, f64), scale: f64) -> Option<Picture> {
    let (tx, rx) = std::sync::mpsc::channel::<Option<Picture>>();
    let block = RcBlock::new(move |rep: *mut AnyObject, _error: *mut NSError| {
        // SAFETY: a QLThumbnailRepresentation or nil; its CGImage lives as
        // long as it does, so it's retained before the block returns.
        let picture = unsafe { rep.as_ref() }.and_then(|rep| {
            let image: *mut CGImage = unsafe { msg_send![rep, CGImage] };
            std::ptr::NonNull::new(image).map(|i| Picture(unsafe { objc2_core_foundation::CFRetained::retain(i) }))
        });
        let _ = tx.send(picture);
    });
    // SAFETY: QuickLookThumbnailing's documented API; it's thread-safe.
    unsafe {
        let request: Retained<AnyObject> = msg_send![msg_send![&REQUEST, alloc], initWithFileAtURL: &*file_url(&path), size: NSSize::new(size.0, size.1), scale: scale, representationTypes: usize::MAX];
        let generator: Retained<AnyObject> = msg_send![&GENERATOR, sharedGenerator];
        let _: () = msg_send![&generator, generateBestRepresentationForRequest: &*request, completionHandler: &*block];
    }
    rx.recv_timeout(std::time::Duration::from_secs(10)).ok().flatten()
}

/// A file's thumbnail (see [`thumbnail`]).
pub struct Thumbnail {
    path: Box<dyn Fn() -> Option<PathBuf>>,
    size: (f64, f64),
}

impl Thumbnail {
    /// Its size in points (64 × 64 unless set); the picture keeps its
    /// proportions within it.
    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.size = (width, height);
        self
    }
}

impl View for Thumbnail {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        let (w, h) = self.size;
        let view = NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect::new(objc2_foundation::NSPoint::ZERO, NSSize::new(w, h)));
        view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        let (mut node, _) = Node::leaf(cx, Retained::into_super(Retained::into_super(view.clone())), (false, false), Size { w, h });
        // A newer path's thumbnail wins over an older one still coming.
        let made = Rc::new(Cell::new(0u64));
        let scale = view.window().map_or(2.0, |win| win.backingScaleFactor());
        let path = self.path;
        node.own(Effect::new(move || {
            let p = path();
            untrack(|| {
                let n = made.get() + 1;
                made.set(n);
                let Some(p) = p else {
                    view.setImage(None);
                    return;
                };
                // The file's icon at once, then its thumbnail when it's made.
                view.setImage(Some(&NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&p.to_string_lossy()))));
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                objc2_app_kit::NSAccessibility::setAccessibilityLabel(&*view, Some(&NSString::from_str(&name)));
                let (v, made) = (view.clone(), made.clone());
                crate::spawn(
                    move || generate(p, (w, h), scale),
                    move |picture| {
                        let Some(Picture(cg)) = picture.filter(|_| made.get() == n) else { return };
                        // Its own proportions (a page, a wide frame), in points.
                        let size = NSSize::new(CGImage::width(Some(&cg)) as f64 / scale, CGImage::height(Some(&cg)) as f64 / scale);
                        let image = NSImage::initWithCGImage_size(NSImage::alloc(), &cg, size);
                        v.setImage(Some(&image));
                        THUMBNAILS.with(|t| t.set(t.get() + 1));
                    },
                );
            });
        }));
        node
    }
}

/// A file's thumbnail, as Finder and the Open panel draw it: a page of a
/// PDF, a frame of a video, a picture, a document's first page. Its icon
/// shows until the thumbnail is made (on a worker thread); it follows
/// `path()`'s signals, and VoiceOver reads the file's name.
///
/// ```no_run
/// use ferrokit::*;
///
/// let file = Signal::new(Some(std::path::PathBuf::from("/Library/Desktop Pictures")));
/// let picture = thumbnail(move || file.get()).size(96.0, 96.0);
/// # let _ = picture;
/// ```
pub fn thumbnail(path: impl Fn() -> Option<PathBuf> + 'static) -> Thumbnail {
    Thumbnail { path: Box::new(path), size: (64.0, 64.0) }
}
