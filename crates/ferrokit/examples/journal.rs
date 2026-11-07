//! C12: a rich text journal. A document app saving RTF, as TextEdit does:
//! the format bar, the ruler and the Format menu (fonts, bold, colours,
//! alignment, lists), pictures pasted in, and Export as HTML.
//! `cargo run --release -p ferrokit --example journal`; `CHECK=all` runs
//! its self-checks.

use std::cell::RefCell;

use ferrokit::*;

thread_local! {
    /// The editors made, for the self-checks (only while they run).
    static EDITORS: RefCell<Vec<TextModel>> = const { RefCell::new(Vec::new()) };
}

fn journal() -> DocumentType<Vec<u8>> {
    DocumentType::new(&["rtf"], Vec::new, |doc: &Document<Vec<u8>>| {
        let model = TextModel::new(ViewId(1), "");
        if std::env::var("CHECK").is_ok() {
            EDITORS.with_borrow_mut(|e| e.push(model.clone()));
        }
        let (m1, m2) = (model.clone(), model.clone());
        let rtf = *doc.content();
        vstack((
            text_editor(&model).format_bar().bind_rtf(doc.content()).accessibility_label("Entry"),
            hstack((
                // Words in the entry, counted again as it changes.
                text(move || rtf.with(|_| format!("{} words", m1.text().split_whitespace().count()))).color(Color::Secondary),
                spacer(),
                button("Export as HTML…", move || export(&m2)),
            ))
            .padding_xy(10.0, 6.0),
        ))
        .spacing(0.0)
    })
    .read(|path| std::fs::read(path))
    // What's on disk becomes the saved state (`write_returning`): the
    // entry is edited again only once it differs from the file.
    .write_returning(|rtf, path| {
        std::fs::write(path, rtf)?;
        std::fs::read(path)
    })
    .size(640.0, 460.0)
}

/// Saves the entry as an HTML page, where the user chooses.
fn export(model: &TextModel) {
    let Some(w) = focused_window() else { return };
    let html = model.html();
    save_panel(&w, "Entry.html", None, move |path| {
        if let Some(path) = path
            && let Err(e) = std::fs::write(&path, html)
        {
            eprintln!("couldn't export: {e}");
        }
    });
}

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    App::new("Journal")
        .documents(journal())
        .menu("Format", format_menu())
        .on_launch(move || {
            if checks {
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(400)), |()| self_checks::run());
            }
        })
        .run();
}

// ---- self-checks ----

mod self_checks {
    use std::cell::Cell;
    use std::path::{Path, PathBuf};

    use ferrokit::testing::ax_audit;
    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplication, NSMenu, NSWindow};
    use objc2_foundation::NSString;

    use super::EDITORS;

    thread_local! {
        static FAILED: Cell<u32> = const { Cell::new(0) };
    }

    fn check(name: &str, pass: bool, note: String) {
        eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
        if !pass {
            FAILED.with(|f| f.set(f.get() + 1));
        }
    }

    fn then(f: impl FnOnce() + 'static) {
        spawn(|| std::thread::sleep(std::time::Duration::from_millis(150)), move |()| f());
    }

    /// The editor in `w`.
    fn editor(w: &NSWindow) -> TextModel {
        EDITORS.with_borrow(|e| e.iter().find(|m| m.text_view().window().is_some_and(|x| std::ptr::eq(&*x, w))).cloned()).unwrap()
    }

    /// Runs the Format menu's item `title` (in its submenu), as choosing it
    /// does: AppKit sends it to the focused editor (or the font manager).
    fn format(title: &str) -> bool {
        let app = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
        let Some(format) = app.mainMenu().and_then(|m| m.itemWithTitle(&NSString::from_str("Format"))).and_then(|i| i.submenu()) else { return false };
        fn find(menu: &NSMenu, title: &str) -> Option<(Retained<NSMenu>, isize)> {
            for i in 0..menu.numberOfItems() {
                let item = menu.itemAtIndex(i)?;
                if item.title().to_string() == title {
                    return Some((menu.retain(), i));
                }
                if let Some(sub) = item.submenu()
                    && let Some(found) = find(&sub, title)
                {
                    return Some(found);
                }
            }
            None
        }
        use objc2::Message;
        let Some((menu, i)) = find(&format, title) else { return false };
        menu.update();
        let enabled = menu.itemAtIndex(i).is_some_and(|m| m.isEnabled());
        menu.performActionForItemAtIndex(i);
        enabled
    }

    pub fn run() {
        // The Format menu acts in the key window: bring the app forward.
        let w = windows()[0].clone();
        ferrokit::testing::input::ready(&w);
        then(move || {
            let active = ferrokit::testing::input::ready(&w);
            checks(w, active);
        });
    }

    fn checks(w: Retained<NSWindow>, active: bool) {
        let dir = std::env::temp_dir().join(format!("ferrokit-journal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m = editor(&w);
        let tv = m.text_view();
        w.makeFirstResponder(Some(&tv));
        check(
            "rich-editor",
            m.is_rich() && m.has_format_bar() && !m.is_ruler_visible() && ax_audit(&w).is_empty(),
            format!("rich {}; format bar {}; ruler hidden {}; audit {:?}", m.is_rich(), m.has_format_bar(), !m.is_ruler_visible(), ax_audit(&w)),
        );
        // Type a line, select a word, make it bold from the Format menu.
        unsafe { objc2::msg_send![&*tv, insertText: &*NSString::from_str("Ferry to Reine at dawn"), replacementRange: objc2_foundation::NSRange::new(0, 0)] }
        m.set_selection(9..14);
        let (bold, center, ruler) = if active {
            (format("Bold"), format("Center"), format("Show Ruler"))
        } else {
            // Another app is in front (an alert): style the word directly,
            // so the checks after this one still run.
            if let Some(storage) = unsafe { tv.textStorage() } {
                unsafe { objc2::msg_send![&*storage, applyFontTraits: objc2_app_kit::NSFontTraitMask::BoldFontMask, range: objc2_foundation::NSRange::new(9, 5)] }
            }
            (false, false, false)
        };
        then(move || {
            let (word, plain) = (m.style_at(10), m.style_at(2));
            let rtf = String::from_utf8_lossy(&m.rtf()).to_string();
            let edited = document_edited(&w);
            if !active {
                eprintln!("[check] format-menu SKIP the app couldn't come to the front (another app's window is in front)");
                return save_checks(w, m, dir);
            }
            check(
                "format-menu",
                bold && center && ruler && word.bold && !plain.bold && rtf.contains("\\qc") && m.is_ruler_visible() && edited,
                format!("Bold, Center, Show Ruler enabled and run {bold}/{center}/{ruler}: \"Reine\" bold {}, \"Fe\" plain {}; centred in the RTF {}; ruler shown {}; edited {edited}", word.bold, !plain.bold, rtf.contains("\\qc"), m.is_ruler_visible()),
            );
            save_checks(w, m, dir);
        });
    }

    fn save_checks(w: Retained<NSWindow>, m: TextModel, dir: PathBuf) {
        let path = dir.join("Lofoten.rtf");
        let saved = save_document_as(&w, &path).is_ok();
        let file = std::fs::read(&path).unwrap_or_default();
        let head = String::from_utf8_lossy(&file[..file.len().min(6)]).to_string();
        let html = m.html();
        check(
            "save-rtf",
            saved && head == "{\\rtf1" && !document_edited(&w) && html.contains("Reine") && (html.contains("font-weight: bold") || html.contains("<b>")),
            format!("saved {saved}: the file starts {head:?}; HTML export has the bold word {}", html.contains("font-weight: bold") || html.contains("<b>")),
        );
        ferrokit::close_window(&w);
        then(move || {
            // Opened again: the styles came back from the file.
            let opened = open_document(&path);
            then(move || {
                let w2 = opened.ok();
                let back = w2.as_ref().map(|w| editor(w));
                let ok = back.as_ref().is_some_and(|m| m.text() == "Ferry to Reine at dawn" && m.style_at(10).bold && !m.style_at(2).bold);
                check("open-rtf", ok, format!("reopened: text {:?}, bold kept {}", back.as_ref().map(|m| m.text()), back.as_ref().is_some_and(|m| m.style_at(10).bold)));
                html_checks(back, &path);
            });
        });
    }

    fn html_checks(m: Option<TextModel>, path: &Path) {
        let _ = path;
        let Some(m) = m else { return finish() };
        let loaded = m.set_html("<h1>Day two</h1><p>Kayaks in <i>Reine</i>, then <b>Å</b>.</p>");
        let text = m.text();
        let at = |s: &str| text.find(s).unwrap_or(0);
        let ok = loaded && text.contains("Day two") && m.style_at(at("Reine")).italic && m.style_at(at("Å")).bold && !m.style_at(at("Kayaks")).bold;
        check("load-html", ok, format!("loaded {loaded}: {text:?}; italic Reine {}, bold Å {}", m.style_at(at("Reine")).italic, m.style_at(at("Å")).bold));
        let not_rtf = !m.set_rtf(b"not rtf") && m.text() == text;
        check("bad-rtf", not_rtf, "text that isn't RTF is refused, the entry kept".into());
        finish();
    }

    fn finish() {
        // Dropped now, not as the thread's storage goes at exit.
        EDITORS.with_borrow_mut(Vec::clear);
        let failed = FAILED.with(Cell::get);
        eprintln!("[check] done, {failed} failed");
        std::process::exit(if failed == 0 { 0 } else { 1 });
    }
}
