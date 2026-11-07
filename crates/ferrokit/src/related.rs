//! Files beside a document that the app opens with it, such as a SQLite
//! database's `-journal`, `-wal` and `-shm`. In the App Sandbox the user's
//! choice of a file gives the app that file alone; [`related_files`] asks
//! for the files beside it (through a file presenter per file, Apple's
//! "related items"), which the app must also declare in its Info.plist
//! (`related = [...]` in `cargo ferrokit`'s documents).

use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{NSFileCoordinator, NSFilePresenter, NSOperationQueue, NSURL};

struct Urls {
    primary: Retained<NSURL>,
    item: Retained<NSURL>,
    queue: Retained<NSOperationQueue>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "FerrokitRelatedFile"]
    #[ivars = Urls]
    struct Presenter;

    unsafe impl NSObjectProtocol for Presenter {}

    unsafe impl NSFilePresenter for Presenter {
        #[unsafe(method_id(presentedItemURL))]
        fn presented_item_url(&self) -> Option<Retained<NSURL>> {
            Some(self.ivars().item.clone())
        }

        #[unsafe(method_id(primaryPresentedItemURL))]
        fn primary_presented_item_url(&self) -> Option<Retained<NSURL>> {
            Some(self.ivars().primary.clone())
        }

        #[unsafe(method_id(presentedItemOperationQueue))]
        fn presented_item_operation_queue(&self) -> Retained<NSOperationQueue> {
            self.ivars().queue.clone()
        }
    }
);

/// Access to the files beside a document, held while this lives (see
/// [`related_files`]).
#[must_use = "the access ends when this is dropped"]
pub struct RelatedFiles {
    presenters: Vec<Retained<Presenter>>,
    paths: Vec<PathBuf>,
}

impl RelatedFiles {
    /// The related files' paths (whether or not they exist yet).
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
}

impl Drop for RelatedFiles {
    fn drop(&mut self) {
        for p in &self.presenters {
            NSFileCoordinator::removeFilePresenter(ProtocolObject::from_ref(&**p));
        }
    }
}

/// Lets the app create, read and write the files beside `document` named
/// by appending each of `suffixes` to its name (`"-wal"` gives
/// `data.sqlite-wal`), for as long as the returned value lives.
///
/// Outside the sandbox this changes nothing. In it, the app must declare
/// the related files' extensions with `NSIsRelatedItemType` (with
/// `cargo ferrokit`, `related = ["-wal", "-shm", "-journal"]` in the
/// document type), and the user must have given it `document` (Open…, a
/// drop, Open Recent). Claim them before opening the document: a SQLite
/// database needs its journal to be written to, and its WAL files to be
/// read at all.
pub fn related_files(document: &Path, suffixes: &[&str]) -> RelatedFiles {
    let primary = NSURL::from_file_path(document);
    let mut out = RelatedFiles { presenters: Vec::new(), paths: Vec::new() };
    let Some(primary) = primary else { return out };
    let queue = NSOperationQueue::new();
    for suffix in suffixes {
        let mut name = document.as_os_str().to_owned();
        name.push(suffix);
        let path = PathBuf::from(name);
        let Some(item) = NSURL::from_file_path(&path) else { continue };
        let this = Presenter::alloc().set_ivars(Urls { primary: primary.clone(), item, queue: queue.clone() });
        // SAFETY: NSObject's designated initialiser.
        let presenter: Retained<Presenter> = unsafe { msg_send![super(this), init] };
        NSFileCoordinator::addFilePresenter(ProtocolObject::from_ref(&*presenter));
        out.presenters.push(presenter);
        out.paths.push(path);
    }
    out
}
