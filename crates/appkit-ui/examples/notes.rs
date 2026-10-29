//! G1 exit check: a plain-text document app. New, Open…, Open Recent,
//! Save, Save As… and Revert to Saved; the edited dot; a save prompt when
//! closing or quitting with changes. Each window is one file.
//! `cargo run --release -p appkit-ui --example notes`; `CHECK=all` runs its
//! self-checks.

use appkit_ui::*;

fn text_document() -> DocumentType<String> {
    DocumentType::new(&["txt", "md"], String::new, |doc: &Document<String>| {
        let model = TextModel::new(ViewId(1), &doc.content().get_untracked());
        let words = doc.content().clone();
        vstack((
            text_editor(&model).bind(doc.content()).accessibility_label("Text"),
            label(move || format!("{} words", words.with(|t| t.split_whitespace().count()))).padding(6.0),
        ))
    })
    .read(|path| std::fs::read_to_string(path))
    .write(|text, path| std::fs::write(path, text))
    .size(600.0, 420.0)
}

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|c| c == "all");
    App::new("Notes")
        .documents(text_document())
        .on_launch(move || {
            if checks {
                self_checks::run();
            }
        })
        .run();
}

// ---- self-checks (not part of the app code count) ----

mod self_checks {
    use std::cell::Cell;
    use std::path::{Path, PathBuf};

    use appkit_ui::testing::{ax_audit, menu_key};
    use appkit_ui::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplication, NSButton, NSEventModifierFlags, NSTextView, NSView};
    use objc2_foundation::NSString;

    thread_local! {
        static FAILED: Cell<u32> = const { Cell::new(0) };
    }

    fn check(name: &str, pass: bool, note: String) {
        eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
        if !pass {
            FAILED.with(|f| f.set(f.get() + 1));
        }
    }

    fn views(v: &NSView, out: &mut Vec<Retained<NSView>>) {
        for s in v.subviews().iter() {
            out.push(s.clone());
            views(&s, out);
        }
    }

    fn text_view(w: &NSWindow) -> Retained<NSTextView> {
        let mut all = Vec::new();
        views(&w.contentView().unwrap(), &mut all);
        all.into_iter().find_map(|v| v.downcast::<NSTextView>().ok()).unwrap()
    }

    /// Types `text` at the end, as one undoable step.
    fn type_text(w: &NSWindow, text: &str) {
        let tv = text_view(w);
        let um = tv.undoManager().unwrap();
        um.setGroupsByEvent(false);
        um.beginUndoGrouping();
        let end = tv.string().length();
        unsafe { objc2::msg_send![&*tv, insertText: &*NSString::from_str(text), replacementRange: objc2_foundation::NSRange::new(end, 0)] }
        um.endUndoGrouping();
    }

    /// The sheet's button titles, sorted (the views' order isn't the order
    /// they were added in).
    fn sheet_titles(w: &NSWindow) -> Vec<String> {
        let mut t: Vec<String> = sheet_buttons(w).iter().map(|b| b.title().to_string()).collect();
        t.sort();
        t
    }

    fn sheet_buttons(w: &NSWindow) -> Vec<Retained<NSButton>> {
        let Some(sheet) = w.attachedSheet() else { return Vec::new() };
        let mut all = Vec::new();
        views(&sheet.contentView().unwrap(), &mut all);
        all.into_iter().filter_map(|v| v.downcast::<NSButton>().ok()).filter(|b| !b.title().to_string().is_empty()).collect()
    }

    fn press(w: &NSWindow, title: &str) -> bool {
        match sheet_buttons(w).into_iter().find(|b| b.title().to_string() == title) {
            Some(b) => {
                unsafe { b.performClick(None) };
                true
            }
            None => false,
        }
    }

    fn titles() -> Vec<String> {
        windows().iter().map(|w| w.title().to_string()).collect()
    }

    fn find(title: &str) -> Option<Retained<NSWindow>> {
        windows().into_iter().find(|w| w.title().to_string() == title)
    }

    /// Runs `f` after the run loop has handled pending work (model events,
    /// teardown, sheet completions).
    fn then(f: impl FnOnce() + 'static) {
        spawn(|| std::thread::sleep(std::time::Duration::from_millis(150)), move |()| f());
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap_or_default()
    }

    pub fn run() {
        let dir = std::env::temp_dir().join(format!("appkit-ui-notes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let w1 = windows()[0].clone();
        check(
            "untitled-at-launch",
            titles() == ["Untitled"] && !document_edited(&w1),
            format!("windows {:?}; edited {}", titles(), document_edited(&w1)),
        );
        let menu = menu_titles();
        check(
            "file-menu",
            // AppKit adds Close All (the ⌥ alternate) after Close.
            menu == ["New", "Open…", "Open Recent", "", "Close", "Close All", "Save", "Save As…", "Revert to Saved"].map(String::from),
            format!("{menu:?}"),
        );
        let fired = menu_key("n", NSEventModifierFlags::Command);
        check("new-document", fired && titles() == ["Untitled", "Untitled 2"], format!("⌘N → {:?}", titles()));
        let problems = ax_audit(&w1);
        check("accessibility-audit", problems.is_empty(), format!("{problems:?}"));

        type_text(&w1, "Hello there");
        then(move || {
            let edited = document_edited(&w1) && w1.isDocumentEdited();
            text_view(&w1).undoManager().unwrap().undo();
            then(move || {
                let back = !document_edited(&w1) && !w1.isDocumentEdited() && text_view(&w1).string().length() == 0;
                check("edited-and-undo", edited && back, format!("typing marks it edited (and the dot): {edited}; undoing back to the saved text clears both: {back}"));
                save_checks(w1, dir);
            });
        });
    }

    fn menu_titles() -> Vec<String> {
        let app = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
        let file = app.mainMenu().unwrap().itemWithTitle(&NSString::from_str("File")).unwrap().submenu().unwrap();
        (0..file.numberOfItems()).map(|i| file.itemAtIndex(i).unwrap().title().to_string()).collect()
    }

    fn save_checks(w1: Retained<NSWindow>, dir: PathBuf) {
        let a = dir.join("a.txt");
        type_text(&w1, "First draft");
        then(move || {
            let saved = save_document_as(&w1, &a).is_ok();
            then(move || {
                let url = w1.representedURL().and_then(|u| u.path()).map(|p| p.to_string());
                check(
                    "save-as",
                    saved && read(&a) == "First draft" && w1.title().to_string() == "a.txt" && !document_edited(&w1) && url.is_some_and(|u| u.ends_with("a.txt")),
                    format!("file {:?}; title {:?}; edited {}; proxy icon for a.txt: {}", read(&a), w1.title(), document_edited(&w1), w1.representedURL().is_some()),
                );
                type_text(&w1, ", revised");
                then(move || {
                    let edited = document_edited(&w1);
                    let ok = save_document(&w1).is_ok();
                    check("save", edited && ok && read(&a) == "First draft, revised" && !document_edited(&w1), format!("edited {edited} → saved: file {:?}", read(&a)));
                    type_text(&w1, " THROW AWAY");
                    then(move || {
                        let ok = revert_document(&w1).is_ok();
                        then(move || {
                            let text = text_view(&w1).string().to_string();
                            check("revert", ok && text == "First draft, revised" && !document_edited(&w1), format!("after revert the editor shows {text:?}; edited {}", document_edited(&w1)));
                            open_checks(w1, dir, a);
                        });
                    });
                });
            });
        });
    }

    fn open_checks(w1: Retained<NSWindow>, dir: PathBuf, a: PathBuf) {
        let before = windows().len();
        let again = open_document(&a).map(|w| Retained::as_ptr(&w) == Retained::as_ptr(&w1)).unwrap_or(false);
        check("open-already-open", again && windows().len() == before, format!("opening a.txt again brings its window: {again}; windows {} → {}", before, windows().len()));
        let b = dir.join("b.md");
        std::fs::write(&b, "# Notes\nfrom elsewhere").unwrap();
        let opened = open_document(&b);
        let shows = opened.as_ref().map(|w| text_view(w).string().to_string()).unwrap_or_default();
        check("open-file", opened.is_ok() && shows == "# Notes\nfrom elsewhere" && find("b.md").is_some(), format!("b.md opens in a window showing {shows:?}"));
        let bad = dir.join("binary.txt");
        std::fs::write(&bad, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        let err = open_document(&bad).err().map(|e| e.to_string());
        check("open-unreadable", err.is_some() && find("binary.txt").is_none(), format!("not UTF-8 → {err:?}, no window"));

        // Open Recent: rebuilt when the menu opens.
        let recent = recent_documents();
        let app = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
        let file = app.mainMenu().unwrap().itemWithTitle(&NSString::from_str("File")).unwrap().submenu().unwrap();
        let sub = file.itemWithTitle(&NSString::from_str("Open Recent")).unwrap().submenu().unwrap();
        if let Some(d) = sub.delegate() {
            let () = unsafe { objc2::msg_send![&*d, menuNeedsUpdate: &*sub] };
        }
        let items: Vec<String> = (0..sub.numberOfItems()).map(|i| sub.itemAtIndex(i).unwrap().title().to_string()).collect();
        let canon = |p: &Path| p.canonicalize().unwrap();
        check(
            "open-recent",
            recent.first() == Some(&canon(&b)) && recent.get(1) == Some(&canon(&a)) && items.starts_with(&["b.md".to_string(), "a.txt".to_string()]) && items.last().map(String::as_str) == Some("Clear Menu"),
            format!("recents {:?}; menu {items:?}", recent.iter().take(2).map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>()),
        );
        close_checks(w1, a);
    }

    fn close_checks(w1: Retained<NSWindow>, a: PathBuf) {
        let wb = find("b.md").unwrap();
        type_text(&wb, "\nmore");
        then(move || {
            wb.performClose(None);
            let buttons = sheet_titles(&wb);
            let cancelled = press(&wb, "Cancel");
            then(move || {
                let still = find("b.md").is_some() && document_edited(&wb);
                check(
                    "close-asks-cancel",
                    buttons == ["Cancel", "Don't Save", "Save"] && cancelled && still,
                    format!("close with changes → a sheet with {buttons:?}; Cancel keeps it open and edited: {still}"),
                );
                wb.performClose(None);
                let pressed = press(&wb, "Don't Save");
                then(move || {
                    let gone = find("b.md").is_none() && !wb.isVisible();
                    let file = std::fs::read_to_string(wb_path()).unwrap_or_default();
                    check("close-dont-save", pressed && gone && file == "# Notes\nfrom elsewhere", format!("Don't Save closes it: {gone}; the file is unchanged: {:?}", file));
                    type_text(&w1, "!");
                    then(move || {
                        w1.performClose(None);
                        let pressed = press(&w1, "Save");
                        then(move || {
                            let gone = find("a.txt").is_none();
                            check("close-save", pressed && gone && read(&a) == "First draft, revised!", format!("Save writes and closes: file {:?}, closed {gone}", read(&a)));
                            quit_check();
                        });
                    });
                });
            });
        });
    }

    fn wb_path() -> PathBuf {
        std::env::temp_dir().join(format!("appkit-ui-notes-{}", std::process::id())).join("b.md")
    }

    /// Quitting with an edited document asks; Cancel keeps the app running.
    fn quit_check() {
        let w2 = find("Untitled 2").unwrap();
        type_text(&w2, "unsaved");
        then(move || {
            // quit() starts on the next pass; AppKit then waits for the
            // review's answers in a nested run loop, where this runs.
            quit();
            then(move || {
                let buttons = sheet_titles(&w2);
                let cancelled = press(&w2, "Cancel");
                then(move || {
                    let alive = find("Untitled 2").is_some() && document_edited(&w2);
                    check(
                        "quit-review-cancel",
                        buttons == ["Cancel", "Don't Save", "Save"] && cancelled && alive,
                        format!("quitting with changes → {buttons:?}; Cancel: still running with the document edited: {alive}"),
                    );
                    let _ = std::fs::remove_dir_all(wb_path().parent().unwrap());
                    let failed = FAILED.with(Cell::get);
                    eprintln!("[check] done, {failed} failed");
                    std::process::exit(if failed == 0 { 0 } else { 1 });
                });
            });
        });
    }
}
