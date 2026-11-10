//! Updates through [Sparkle 2](https://sparkle-project.org): when the app's
//! bundle holds `Sparkle.framework` and names a feed (`cargo ferrokit
//! bundle` puts both in from `[package.metadata.ferrokit.updates]`), the app
//! starts Sparkle's updater at launch and gets **Check for Updates…** in its
//! app menu. Sparkle checks the feed on its schedule (it asks the user
//! first), shows what's new, downloads, checks the update's signature,
//! installs and relaunches.
//!
//! Run unbundled (`cargo run`) or built for the Mac App Store, the app has
//! no Sparkle, and these functions do nothing ([`available`] is `false`).
//! The guide's *Testing and shipping* chapter has the whole recipe: keys,
//! bundling, the feed (`cargo ferrokit appcast`) and publishing.

use ferrokit_core::kept::{UiCell, UiLocal};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSBundle, NSDate, NSError, NSString};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

/// What Sparkle's updater did (see [`on_event`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateEvent {
    /// The feed has a newer version than this one.
    Found {
        /// Its version, as the user sees it.
        version: String,
    },
    /// The check found nothing newer.
    UpToDate,
    /// The update is downloaded and checked, and installs when the app
    /// quits (Sparkle's automatic updates, or the user chose Install on
    /// Quit).
    ReadyOnQuit {
        /// Its version, as the user sees it.
        version: String,
    },
    /// A check or an update failed: the feed couldn't be read, the
    /// download or its signature was bad, the install failed.
    Failed {
        /// Sparkle's description of what went wrong.
        message: String,
    },
}

type Handler = Rc<dyn Fn(&UpdateEvent)>;

struct Sparkle {
    controller: Retained<NSObject>,
    updater: Retained<NSObject>,
    _delegate: Retained<Delegate>,
}

thread_local! {
    static SPARKLE: UiCell<Option<Sparkle>> = const { UiCell::new(None) };
    static HANDLERS: UiCell<Vec<Handler>> = const { UiCell::new(Vec::new()) };
    static STARTUP_ERROR: UiCell<Option<String>> = const { UiCell::new(None) };
}

define_class!(
    // Sparkle's updater delegate: turns its callbacks into `UpdateEvent`s.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitUpdatesDelegate"]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(updater:didFindValidUpdate:))]
        fn found(&self, _updater: &AnyObject, item: &AnyObject) {
            crate::failure::guard("the updater (found)", (), || {
                emit(UpdateEvent::Found { version: version_of(item) });
            })
        }

        #[unsafe(method(updaterDidNotFindUpdate:))]
        fn up_to_date(&self, _updater: &AnyObject) {
            crate::failure::guard("the updater (up_to_date)", (), || {
                emit(UpdateEvent::UpToDate);
            })
        }

        #[unsafe(method(updater:didAbortWithError:))]
        fn aborted(&self, _updater: &AnyObject, error: &NSError) {
            crate::failure::guard("the updater (aborted)", (), || {
                // Sparkle reports "no update" as an error here too (after
                // `updaterDidNotFindUpdate:`).
                if error.code() != NO_UPDATE {
                    emit(UpdateEvent::Failed { message: error.localizedDescription().to_string() });
                }
            })
        }

        #[unsafe(method(updater:willInstallUpdateOnQuit:immediateInstallationBlock:))]
        fn ready(&self, _updater: &AnyObject, item: &AnyObject, _install_now: *mut std::ffi::c_void) -> Bool {
            emit(UpdateEvent::ReadyOnQuit { version: version_of(item) });
            // Sparkle's own handling: install when the app quits.
            Bool::NO
        }
    }
);

/// `SUNoUpdateError` in Sparkle's error domain.
const NO_UPDATE: isize = 1001;

fn version_of(item: &AnyObject) -> String {
    // SAFETY: an SUAppcastItem's version for display (never nil).
    let v: Option<Retained<NSString>> = unsafe { msg_send![item, displayVersionString] };
    v.map(|v| v.to_string()).unwrap_or_default()
}

fn emit(event: UpdateEvent) {
    let handlers: Vec<Handler> = HANDLERS.with_borrow(Clone::clone);
    for h in handlers {
        crate::failure::entry("an update event's handler", (), || h(&event));
    }
}

/// Why updates are off in a bundle that's set up for them: printed, and
/// handed to [`on_event`]'s handlers as [`UpdateEvent::Failed`].
fn failed(message: String) {
    eprintln!("ferrokit: updates are off: {message}");
    STARTUP_ERROR.with_borrow_mut(|e| *e = Some(message));
}

/// Starts Sparkle's updater if the app's bundle has it; the controller, the
/// target of Check for Updates…. Called once, by `App::run`.
pub(crate) fn start() -> Option<Retained<NSObject>> {
    let main = NSBundle::mainBundle();
    let info = |key: &str| main.objectForInfoDictionaryKey(&NSString::from_str(key));
    let framework = main.privateFrameworksPath().map(|p| Path::new(&p.to_string()).join("Sparkle.framework"));
    let has_framework = framework.as_ref().is_some_and(|f| f.exists());
    let has_feed = info("SUFeedURL").is_some();
    match (has_framework, has_feed) {
        (false, false) => return None,
        (false, true) => {
            failed("the app names an update feed (SUFeedURL) but its bundle has no Sparkle.framework: bundle it with `cargo ferrokit bundle` and [package.metadata.ferrokit.updates]".into());
            return None;
        }
        (true, false) => {
            failed("the app's bundle has Sparkle.framework but no update feed (SUFeedURL in Info.plist)".into());
            return None;
        }
        (true, true) => {}
    }
    if info("SUPublicEDKey").is_none() {
        failed("the app's Info.plist has no SUPublicEDKey: Sparkle won't install updates it can't check".into());
        return None;
    }
    let framework = framework?;
    let path = NSString::from_str(&framework.to_string_lossy());
    // SAFETY: loading Sparkle's framework from the app's own bundle runs its
    // initialisers, which is what linking it would do.
    let loaded = match NSBundle::bundleWithPath(&path) {
        Some(b) => unsafe { b.loadAndReturnError() }.map_err(|e| {
            // The loader's own words (a signature it refused, say) are in the debug description.
            let debug = e.userInfo().objectForKey(&NSString::from_str("NSDebugDescription")).and_then(|d| d.downcast::<NSString>().ok()).map(|d| format!(" ({d})")).unwrap_or_default();
            format!("{}{debug}", e.localizedDescription())
        }),
        None => Err("not a bundle".into()),
    };
    let class = loaded.and_then(|()| AnyClass::get(c"SPUStandardUpdaterController").ok_or_else(|| "no SPUStandardUpdaterController".into()));
    let class = match class {
        Ok(c) => c,
        Err(e) => {
            failed(format!("couldn't load {}: {e}", framework.display()));
            return None;
        }
    };
    let delegate: Retained<Delegate> = {
        let d = Delegate::alloc(crate::mtm()).set_ivars(());
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(d), init] }
    };
    // SAFETY: Sparkle's documented initialiser: start the updater now, with
    // our delegate (Sparkle keeps it weakly; `SPARKLE` keeps it) and the
    // standard user interface.
    let controller: Option<Retained<NSObject>> = unsafe {
        let alloc: Allocated<NSObject> = msg_send![class, alloc];
        msg_send![alloc, initWithStartingUpdater: true, updaterDelegate: &*delegate, userDriverDelegate: Option::<&AnyObject>::None]
    };
    let Some(controller) = controller else {
        failed("Sparkle's updater controller didn't start".into());
        return None;
    };
    // SAFETY: the controller's updater (never nil).
    let updater: Retained<NSObject> = unsafe { msg_send![&controller, updater] };
    SPARKLE.with_borrow_mut(|s| *s = Some(Sparkle { controller: controller.clone(), updater, _delegate: delegate }));
    Some(controller)
}

// The objects are taken out before Sparkle is called: it can call back
// (the delegate, a handler) while it runs.
fn with_updater<R>(f: impl FnOnce(&NSObject) -> R) -> Option<R> {
    let updater = SPARKLE.with_borrow(|s| s.as_ref().map(|s| s.updater.clone()))?;
    Some(f(&updater))
}

/// Whether the app has Sparkle (it was bundled with updates).
pub fn available() -> bool {
    SPARKLE.with_borrow(Option::is_some)
}

/// Checks for updates as **Check for Updates…** does: Sparkle shows its
/// progress and what it finds.
pub fn check() {
    if let Some(controller) = SPARKLE.with_borrow(|s| s.as_ref().map(|s| s.controller.clone())) {
        // SAFETY: the controller's action, with no sender.
        let _: () = unsafe { msg_send![&controller, checkForUpdates: Option::<&AnyObject>::None] };
    }
}

/// Checks without showing anything unless there's an update (as Sparkle's
/// scheduled checks do). With [automatic checks](set_automatic_checks) and
/// [downloads](set_automatic_downloads) both on, an update is downloaded
/// and installed when the app quits; otherwise Sparkle shows it.
pub fn check_in_background() {
    // SAFETY: SPUUpdater's method.
    with_updater(|u| unsafe { msg_send![u, checkForUpdatesInBackground] }).unwrap_or(());
}

/// Whether a check can start now (there's Sparkle and none is running).
pub fn can_check() -> bool {
    // SAFETY: SPUUpdater's property.
    with_updater(|u| unsafe { msg_send![u, canCheckForUpdates] }).unwrap_or(false)
}

/// Whether Sparkle checks on its schedule (daily by default). Sparkle asks
/// the user at the second launch; this is their answer, or what the app set.
pub fn automatic_checks() -> bool {
    // SAFETY: SPUUpdater's property.
    with_updater(|u| unsafe { msg_send![u, automaticallyChecksForUpdates] }).unwrap_or(false)
}

/// Turns scheduled checks on or off (a Settings checkbox, say). Kept in the
/// app's defaults.
pub fn set_automatic_checks(on: bool) {
    // SAFETY: SPUUpdater's property.
    with_updater(|u| unsafe { msg_send![u, setAutomaticallyChecksForUpdates: on] }).unwrap_or(());
}

/// Whether updates found by scheduled checks download on their own and
/// install when the app quits (Sparkle does this only while
/// [automatic checks](automatic_checks) are on too).
pub fn automatic_downloads() -> bool {
    // SAFETY: SPUUpdater's property.
    with_updater(|u| unsafe { msg_send![u, automaticallyDownloadsUpdates] }).unwrap_or(false)
}

/// Turns automatic downloads on or off. Kept in the app's defaults.
pub fn set_automatic_downloads(on: bool) {
    // SAFETY: SPUUpdater's property.
    with_updater(|u| unsafe { msg_send![u, setAutomaticallyDownloadsUpdates: on] }).unwrap_or(());
}

/// When Sparkle last checked, if it has.
pub fn last_check() -> Option<SystemTime> {
    // SAFETY: SPUUpdater's property (nil before the first check).
    let date: Option<Retained<NSDate>> = with_updater(|u| unsafe { msg_send![u, lastUpdateCheckDate] }).flatten();
    date.map(|d| SystemTime::UNIX_EPOCH + Duration::from_secs_f64(d.timeIntervalSince1970().max(0.0)))
}

/// Calls `f` with what the updater does: found an update, up to date,
/// ready to install on quit, failed. Sparkle shows its own windows for
/// checks the user starts; this is for the app's own status (a badge, a
/// line in Settings) and for checks.
///
/// If the app's bundle is set up for updates but Sparkle couldn't start (a
/// missing key, a framework that won't load), `f` hears that first, as
/// [`UpdateEvent::Failed`].
pub fn on_event(f: impl Fn(&UpdateEvent) + 'static) {
    let f: Handler = Rc::new(f);
    HANDLERS.with_borrow_mut(|h| h.push(f.clone()));
    if let Some(message) = STARTUP_ERROR.with_borrow(Clone::clone) {
        crate::later(move || {
            let event = UpdateEvent::Failed { message: message.clone() };
            crate::failure::entry("an update event's handler", (), || f(&event));
        });
    }
}
