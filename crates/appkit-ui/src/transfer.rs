//! The clipboard and dragging data out: text, PNG images, files, and files
//! written on demand where they're dropped (a CSV dragged to the Finder).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_app_kit::{
    NSFilePromiseProvider, NSFilePromiseProviderDelegate, NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypePNG,
    NSPasteboardTypeString, NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSError, NSOperationQueue, NSString, NSURL};

/// A pasteboard: [`Clipboard::general`] is the system clipboard (⌘C/⌘V);
/// [`Clipboard::private`] is one of the app's own (for tests, or data that
/// mustn't reach the clipboard).
pub struct Clipboard(Retained<NSPasteboard>, bool);

impl Clipboard {
    /// The system clipboard.
    pub fn general() -> Self {
        Clipboard(NSPasteboard::generalPasteboard(), false)
    }

    /// A new pasteboard of the app's own, released when dropped.
    pub fn private() -> Self {
        Clipboard(NSPasteboard::pasteboardWithUniqueName(), true)
    }

    /// The `NSPasteboard`.
    pub fn pasteboard(&self) -> &NSPasteboard {
        &self.0
    }

    /// Replaces the contents with `text`.
    pub fn set_text(&self, text: &str) {
        self.0.clearContents();
        // SAFETY: AppKit's pasteboard type constant.
        self.0.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
    }

    /// The text on it, if any.
    pub fn text(&self) -> Option<String> {
        // SAFETY: AppKit's pasteboard type constant.
        self.0.stringForType(unsafe { NSPasteboardTypeString }).map(|s| s.to_string())
    }

    /// Replaces the contents with a PNG image.
    pub fn set_png(&self, png: &[u8]) {
        self.0.clearContents();
        // SAFETY: AppKit's pasteboard type constant.
        self.0.setData_forType(Some(&NSData::with_bytes(png)), unsafe { NSPasteboardTypePNG });
    }

    /// The PNG image on it, if any.
    pub fn png(&self) -> Option<Vec<u8>> {
        // SAFETY: AppKit's pasteboard type constant.
        self.0.dataForType(unsafe { NSPasteboardTypePNG }).map(|d| d.to_vec())
    }

    /// Replaces the contents with files (as the Finder's ⌘C does).
    pub fn set_files(&self, paths: &[PathBuf]) {
        self.0.clearContents();
        let urls: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = paths.iter().map(|p| ProtocolObject::from_retained(file_url(p))).collect();
        self.0.writeObjects(&NSArray::from_retained_slice(&urls));
    }

    /// The files on it (copied in the Finder, or dragged).
    pub fn files(&self) -> Vec<PathBuf> {
        files_on(&self.0)
    }

    /// Empties it.
    pub fn clear(&self) {
        self.0.clearContents();
    }

    /// Puts `item` on it, as a drag would carry it. A promised file is
    /// written now, into the temporary folder, and put on as that file.
    pub fn set(&self, item: &DragItem) {
        match item {
            DragItem::Text(t) => self.set_text(t),
            DragItem::Png(png) => self.set_png(png),
            DragItem::File(p) => self.set_files(std::slice::from_ref(p)),
            DragItem::Promise { name, write } => {
                let dir = std::env::temp_dir().join(format!("appkit-ui-{}", std::process::id()));
                let _ = std::fs::create_dir_all(&dir);
                let path = dir.join(name);
                if write(&path).is_ok() {
                    self.set_files(&[path]);
                }
            }
        }
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        if self.1 {
            // SAFETY: frees a uniquely named pasteboard's resources.
            let () = unsafe { msg_send![&*self.0, releaseGlobally] };
        }
    }
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

/// The file URLs on `pasteboard`, as paths.
pub(crate) fn files_on(pasteboard: &NSPasteboard) -> Vec<PathBuf> {
    let Some(items) = pasteboard.pasteboardItems() else { return Vec::new() };
    items
        .iter()
        // SAFETY: AppKit's pasteboard type constant.
        .filter_map(|i| i.stringForType(unsafe { NSPasteboardTypeFileURL }))
        .filter_map(|s| NSURL::URLWithString(&s)?.path())
        .map(|p| PathBuf::from(p.to_string()))
        .collect()
}

/// A file's writer: called with where to write it.
pub type WriteFile = Arc<dyn Fn(&Path) -> std::io::Result<()> + Send + Sync>;

/// What a drag carries out of the app (see
/// [`Canvas::draggable`](crate::Canvas::draggable)).
#[derive(Clone)]
pub enum DragItem {
    /// Text.
    Text(String),
    /// A PNG image.
    Png(Vec<u8>),
    /// A file that exists.
    File(PathBuf),
    /// A file written where it's dropped (the Finder, Mail, a folder in
    /// another app), named `name`: `write` gets its path when the drop
    /// lands, so nothing is written for a cancelled drag.
    Promise {
        /// The file's name, with its extension (`results.csv`).
        name: String,
        /// Writes the file at the given path.
        write: WriteFile,
    },
}

impl DragItem {
    /// A promised file named `name`, written by `write` when dropped.
    pub fn promise(name: &str, write: impl Fn(&Path) -> std::io::Result<()> + Send + Sync + 'static) -> Self {
        DragItem::Promise { name: name.into(), write: Arc::new(write) }
    }

    /// The pasteboard writer AppKit's drag takes.
    pub(crate) fn writer(&self) -> Retained<ProtocolObject<dyn NSPasteboardWriting>> {
        match self {
            DragItem::Text(t) => ProtocolObject::from_retained(NSString::from_str(t)),
            DragItem::File(p) => ProtocolObject::from_retained(file_url(p)),
            DragItem::Png(png) => {
                let item = NSPasteboardItem::new();
                // SAFETY: AppKit's pasteboard type constant.
                item.setData_forType(&NSData::with_bytes(png), unsafe { NSPasteboardTypePNG });
                ProtocolObject::from_retained(item)
            }
            DragItem::Promise { name, write } => {
                let delegate = Promiser::alloc().set_ivars(Promise { name: name.clone(), write: write.clone() });
                // SAFETY: NSObject's designated initialiser.
                let delegate: Retained<Promiser> = unsafe { msg_send![super(delegate), init] };
                let kind = uti_for(name);
                // SAFETY: a provider for one file of type `kind`; it keeps its
                // delegate only weakly, so the provider's userInfo keeps it.
                let provider = unsafe {
                    let p = NSFilePromiseProvider::initWithFileType_delegate(NSFilePromiseProvider::alloc(), &NSString::from_str(&kind), ProtocolObject::from_ref(&*delegate));
                    p.setUserInfo(Some(&delegate));
                    p
                };
                ProtocolObject::from_retained(provider)
            }
        }
    }
}

/// The type identifier for a file name's extension.
fn uti_for(name: &str) -> String {
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    objc2_uniform_type_identifiers::UTType::typeWithFilenameExtension(&NSString::from_str(&ext))
        .map(|t| t.identifier().to_string())
        .unwrap_or_else(|| "public.data".into())
}

pub(crate) struct Promise {
    name: String,
    write: WriteFile,
}

define_class!(
    // Names and writes a promised file when the drop lands.
    #[unsafe(super(NSObject))]
    #[name = "AppKitUiFilePromise"]
    #[ivars = Promise]
    pub(crate) struct Promiser;

    unsafe impl NSObjectProtocol for Promiser {}

    unsafe impl NSFilePromiseProviderDelegate for Promiser {
        #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
        fn file_name(&self, _provider: &NSFilePromiseProvider, _kind: &NSString) -> Retained<NSString> {
            NSString::from_str(&self.ivars().name)
        }

        #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
        fn write_promise(&self, _provider: &NSFilePromiseProvider, url: &NSURL, done: &block2::DynBlock<dyn Fn(*mut NSError)>) {
            let path = url.path().map(|p| PathBuf::from(p.to_string()));
            let result = path.ok_or_else(|| std::io::Error::other("not a file URL")).and_then(|p| (self.ivars().write)(&p));
            match result {
                Ok(()) => done.call((std::ptr::null_mut(),)),
                Err(e) => {
                    let error = NSError::new(-1, &NSString::from_str(&format!("appkit-ui: {e}")));
                    done.call((Retained::as_ptr(&error) as *mut NSError,));
                }
            }
        }

        #[unsafe(method_id(operationQueueForFilePromiseProvider:))]
        fn queue(&self, _provider: &NSFilePromiseProvider) -> Retained<NSOperationQueue> {
            // Off the main thread: writing a large file mustn't stall the UI.
            NSOperationQueue::new()
        }
    }
);

/// Writes a promised item into `dir` as its drop would (for checks): the
/// path written, or the error.
pub(crate) fn fulfil(item: &DragItem, dir: &Path) -> std::io::Result<PathBuf> {
    match item {
        DragItem::Promise { name, write } => {
            let path = dir.join(name);
            write(&path)?;
            Ok(path)
        }
        _ => Err(std::io::Error::other("not a promised file")),
    }
}
