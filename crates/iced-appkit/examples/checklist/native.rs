//! Test scaffolding: plain AppKit views built directly with objc2 (the real
//! table and text editor widgets come in M2/M3).

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSButton, NSControlTextEditingDelegate, NSColor, NSScrollView, NSTableColumn, NSTableView, NSTableViewDataSource, NSTableViewDelegate,
    NSTableViewStyle, NSTextAlignment, NSTextField, NSTextView, NSUserInterfaceItemIdentification, NSView,
};
use objc2_foundation::{NSInteger, NSString};

use iced_appkit::{ViewId, host};

pub struct Rows(NSInteger);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ChecklistTableSource"]
    #[ivars = Rows]
    pub struct TableSource;

    unsafe impl NSObjectProtocol for TableSource {}
    unsafe impl NSControlTextEditingDelegate for TableSource {}

    unsafe impl NSTableViewDataSource for TableSource {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &NSTableView) -> NSInteger {
            self.ivars().0
        }
    }

    unsafe impl NSTableViewDelegate for TableSource {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for(&self, table: &NSTableView, column: Option<&NSTableColumn>, row: NSInteger) -> Option<Retained<NSView>> {
            cell(self, table, column, row)
        }
    }
);

fn cell(source: &TableSource, table: &NSTableView, column: Option<&NSTableColumn>, row: NSInteger) -> Option<Retained<NSView>> {
    let ident = column?.identifier();
    // SAFETY: reuse lookup by identifier; owner may be nil.
    let field = match unsafe { table.makeViewWithIdentifier_owner(&ident, None) } {
        Some(view) => view.downcast::<NSTextField>().ok()?,
        None => {
            let field = NSTextField::labelWithString(&NSString::new(), MainThreadMarker::from(source));
            field.setIdentifier(Some(&ident));
            field
        }
    };
    let text = if ident.to_string() == "row" { row.to_string() } else { format!("Item {row:06}") };
    field.setStringValue(&NSString::from_str(&text));
    Some(Retained::into_super(Retained::into_super(field)))
}

/// Keeps data sources alive (AppKit holds them weakly).
pub struct Keep(#[allow(dead_code)] Vec<Retained<NSObject>>);

pub fn keep() -> Keep {
    Keep(Vec::new())
}

pub fn mount_table(id: u64, rows: NSInteger, keep: &mut Keep) {
    let mtm = MainThreadMarker::new().unwrap();
    let source = TableSource::alloc(mtm).set_ivars(Rows(rows));
    // SAFETY: plain NSObject init.
    let source: Retained<TableSource> = unsafe { msg_send![super(source), init] };
    let table = NSTableView::new(mtm);
    table.setStyle(NSTableViewStyle::Plain);
    table.setUsesAlternatingRowBackgroundColors(true);
    table.setAllowsMultipleSelection(true);
    for (ident, title, width) in [("row", "#", 80.0), ("name", "Name", 200.0)] {
        let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str(ident));
        column.setTitle(&NSString::from_str(title));
        column.setWidth(width);
        table.addTableColumn(&column);
    }
    // SAFETY: the source is kept alive in `keep` for the app's lifetime.
    unsafe {
        table.setDataSource(Some(ProtocolObject::from_ref(&*source)));
        table.setDelegate(Some(ProtocolObject::from_ref(&*source)));
    }
    let scroll = NSScrollView::new(mtm);
    scroll.setHasVerticalScroller(true);
    scroll.setDocumentView(Some(&table));
    host::mount(ViewId(id), Retained::into_super(scroll));
    keep.0.push(Retained::into_super(source));
}

pub fn mount_button(id: u64, title: &str) {
    let mtm = MainThreadMarker::new().unwrap();
    // SAFETY: nil target and action; set_action sets both.
    let button = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, mtm) };
    host::set_action(&button, ViewId(id));
    host::mount(ViewId(id), Retained::into_super(Retained::into_super(button)));
}

pub fn mount_label(id: u64, title: &str, (r, g, b): (f32, f32, f32)) {
    let mtm = MainThreadMarker::new().unwrap();
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setDrawsBackground(true);
    label.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(r as f64, g as f64, b as f64, 1.0)));
    label.setTextColor(Some(&NSColor::whiteColor()));
    label.setAlignment(NSTextAlignment::Center);
    host::mount(ViewId(id), Retained::into_super(Retained::into_super(label)));
}

fn document<T: objc2::DowncastTarget + 'static>(id: u64) -> Option<Retained<T>> {
    let view = host::view(ViewId(id))?;
    let scroll = view.downcast::<NSScrollView>().ok()?;
    scroll.documentView()?.downcast::<T>().ok()
}

/// Text and selected range length of a mounted text view.
pub fn text_state(id: u64) -> Option<(String, usize)> {
    let text = document::<NSTextView>(id)?;
    Some((text.string().to_string(), text.selectedRange().length))
}

pub fn table_selected(id: u64) -> Option<(isize, isize)> {
    let table = document::<NSTableView>(id)?;
    Some((table.numberOfSelectedRows(), table.numberOfRows()))
}

/// Adds the native Tests menu; each item fires `Event::Action(ViewId(1000 + n))`.
pub fn install_tests_menu(tests: &[(u8, &str)]) {
    use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
    let mtm = MainThreadMarker::new().unwrap();
    let Some(main_menu) = NSApplication::sharedApplication(mtm).mainMenu() else { return };
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Tests"));
    for &(n, title) in tests {
        let key = if n < 10 { n.to_string() } else { String::new() };
        // SAFETY: plain menu item construction.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(&format!("{n}. {title}")), None, &NSString::from_str(&key))
        };
        host::set_menu_action(&item, ViewId(1000 + n as u64));
        menu.addItem(&item);
    }
    let top = NSMenuItem::new(mtm);
    top.setSubmenu(Some(&menu));
    main_menu.addItem(&top);
}

/// Mounts and removes `n` buttons; returns (RSS growth in KB, subviews after).
pub fn teardown(n: usize) -> (i64, usize) {
    let before = rss_kb();
    for _ in 0..n {
        objc2::rc::autoreleasepool(|_| {
            mount_button(9999, "Teardown");
            host::place(ViewId(9999), iced_appkit::Rect::new(10.0, 10.0, 120.0, 30.0), iced_appkit::Rect::new(10.0, 10.0, 120.0, 30.0));
            host::unmount(ViewId(9999));
        });
    }
    (rss_kb() - before, host::testing::subview_count())
}

fn rss_kb() -> i64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

/// Scrolls a native table's scroll view by `dy` points (clamped). Returns
/// false at the end.
pub fn scroll_table(table: &NSTableView, dy: f64) -> bool {
    let Some(clip) = table.enclosingScrollView().map(|s| s.contentView()) else { return false };
    let mut origin = clip.bounds().origin;
    let max = (table.frame().size.height - clip.bounds().size.height).max(0.0);
    let y = (origin.y + dy).clamp(0.0, max);
    let moved = y != origin.y;
    origin.y = y;
    clip.scrollToPoint(origin);
    if let Some(scroll) = table.enclosingScrollView() {
        scroll.reflectScrolledClipView(&clip);
    }
    moved
}

/// What VoiceOver gets from a table, through the accessibility attribute API
/// it uses: (role, row count, column count, has header, values of row 0's
/// cells, label of row 0's checkbox).
pub fn table_accessibility(table: &NSTableView) -> (String, usize, usize, bool, Vec<String>, Option<String>) {
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSArray;
    fn attr(o: &AnyObject, name: &str) -> Option<Retained<AnyObject>> {
        objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
            msg_send![o, accessibilityAttributeValue: &*NSString::from_str(name)]
        }))
        .ok()
        .flatten()
    }
    fn text(o: Option<Retained<AnyObject>>) -> Option<String> {
        let d: Option<Retained<NSString>> = o.and_then(|v| unsafe { msg_send![&*v, description] });
        d.map(|d| d.to_string())
    }
    fn list(o: Option<Retained<AnyObject>>) -> Vec<Retained<AnyObject>> {
        o.and_then(|o| o.downcast::<NSArray<AnyObject>>().ok()).map(|a| a.to_vec()).unwrap_or_default()
    }
    let t: &AnyObject = table.as_ref();
    let role = text(attr(t, "AXRole")).unwrap_or_default();
    let rows = list(attr(t, "AXRows"));
    let columns = list(attr(t, "AXColumns")).len();
    let header = attr(t, "AXHeader").is_some();
    let mut values = Vec::new();
    let mut check_label = None;
    if let Some(row) = rows.first() {
        for cell in list(attr(row, "AXChildren")) {
            for child in list(attr(&cell, "AXChildren")) {
                match text(attr(&child, "AXRole")).as_deref() {
                    // VoiceOver reads the cell's description with the checkbox.
                    Some("AXCheckBox") => {
                        check_label = [attr(&child, "AXDescription"), attr(&cell, "AXDescription")]
                            .into_iter()
                            .filter_map(text)
                            .find(|t| !t.is_empty())
                    }
                    _ => values.extend(text(attr(&child, "AXValue"))),
                }
            }
        }
    }
    (role, rows.len(), columns, header, values, check_label)
}

/// Clicks the checkbox cell at (view row, column) the way a mouse click would.
pub fn click_check(table: &NSTableView, column: isize, view_row: isize) -> bool {
    let Some(cell) = table.viewAtColumn_row_makeIfNecessary(column, view_row, false) else { return false };
    let Ok(button) = cell.downcast::<NSButton>() else { return false };
    // SAFETY: nil sender, as a click sends.
    unsafe { button.performClick(None) };
    true
}

/// Selects a view row through AppKit itself (as a click would), so the
/// delegate's selection notification fires.
pub fn select_view_row(table: &NSTableView, view_row: usize) {
    table.selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(view_row), false);
}

/// Sets the table's sort descriptors directly: what a header click does.
pub fn click_header_sort(table: &NSTableView, column: &str, ascending: bool) {
    use objc2_foundation::{NSArray, NSSortDescriptor};
    let d = NSSortDescriptor::sortDescriptorWithKey_ascending(Some(&NSString::from_str(column)), ascending);
    table.setSortDescriptors(&NSArray::from_retained_slice(&[d]));
}

/// Text shown in the cell at (view row, column).
pub fn cell_text(table: &NSTableView, column: isize, view_row: isize) -> Option<String> {
    let cell = table.viewAtColumn_row_makeIfNecessary(column, view_row, true)?;
    cell.downcast::<NSTextField>().ok().map(|f| f.stringValue().to_string())
}

pub fn rss_mb() -> f64 {
    rss_kb() as f64 / 1024.0
}

/// Debug: prints the accessibility subtree of `object` (role, title/value, children).
pub fn ax_dump(object: &objc2::runtime::AnyObject, depth: usize, max_depth: usize, max_children: usize) {
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSArray;
    // SAFETY: NSAccessibility protocol methods; guarded with respondsToSelector.
    unsafe {
        let s = |sel: objc2::runtime::Sel| -> Option<String> {
            objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
                let v: Option<Retained<AnyObject>> = msg_send![object, accessibilityAttributeValue: &*NSString::from_str(sel.name().to_str().unwrap())];
                let d: Option<Retained<NSString>> = v.and_then(|v| msg_send![&*v, description]);
                d.map(|d| d.to_string().chars().take(40).collect::<String>())
            }))
            .unwrap_or_else(|_| Some("<exception>".into()))
        };
        eprintln!(
            "[ax] {}{} title={:?} value={:?} label={:?}",
            "  ".repeat(depth),
            s(objc2::sel!(AXRole)).unwrap_or_default(),
            s(objc2::sel!(AXTitle)),
            s(objc2::sel!(AXValue)),
            s(objc2::sel!(AXDescription)),
        );
        if depth >= max_depth {
            return;
        }
        let children: Option<Retained<NSArray<AnyObject>>> =
            objc2::exception::catch(std::panic::AssertUnwindSafe(|| msg_send![object, accessibilityAttributeValue: &*NSString::from_str("AXChildren")])).ok().flatten();
        if let Some(children) = children {
            for c in children.iter().take(max_children) {
                ax_dump(&c, depth + 1, max_depth, max_children);
            }
        }
    }
}

/// Drives the editor's NSTextInputClient exactly as an input method does:
/// compose `marked`, report where the candidate window would go, then
/// commit `committed`. Returns (candidate rect on screen, text view's screen rect).
pub fn ime_compose(view: &NSTextView, marked: &str) -> (objc2_foundation::NSRect, objc2_foundation::NSRect) {
    use objc2_foundation::{NSRange, NSRect};
    let not_found = NSRange::new(usize::MAX as i64 as usize, 0);
    let marked = NSString::from_str(marked);
    let length = marked.length();
    // SAFETY: NSTextInputClient methods with valid ranges.
    unsafe {
        let () = msg_send![view, setMarkedText: &*marked, selectedRange: NSRange::new(length, 0), replacementRange: not_found];
        let range: NSRange = msg_send![view, markedRange];
        let rect: NSRect = msg_send![view, firstRectForCharacterRange: NSRange::new(range.location + range.length, 0), actualRange: std::ptr::null_mut::<NSRange>()];
        let window = view.window().unwrap();
        let in_window = view.convertRect_toView(view.visibleRect(), None);
        (rect, window.convertRectToScreen(in_window))
    }
}

pub fn ime_commit(view: &NSTextView, committed: &str) {
    use objc2_foundation::NSRange;
    let not_found = NSRange::new(usize::MAX as i64 as usize, 0);
    // SAFETY: NSTextInputClient insertText with "no replacement range".
    unsafe {
        let () = msg_send![view, insertText: &*NSString::from_str(committed), replacementRange: not_found];
    }
}

/// The text view's context menu for a right-click on `word` (titles).
pub fn context_menu_for_word(view: &NSTextView, word: &str) -> Vec<String> {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{NSPoint, NSRange, NSRect};
    let Some(window) = view.window() else { return Vec::new() };
    let text = view.string().to_string();
    let Some(byte_index) = text.find(word) else { return Vec::new() };
    let location = text[..byte_index].encode_utf16().count();
    // SAFETY: NSTextInputClient geometry query.
    let screen: NSRect = unsafe {
        msg_send![view, firstRectForCharacterRange: NSRange::new(location + word.len() / 2, 1), actualRange: std::ptr::null_mut::<NSRange>()]
    };
    let w = window.convertRectFromScreen(screen);
    let point = NSPoint::new(w.origin.x + w.size.width / 2.0, w.origin.y + w.size.height / 2.0);
    let Some(event) = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::RightMouseDown, point, NSEventModifierFlags::empty(), 0.0, window.windowNumber(), None, 0, 1, 1.0,
    ) else {
        return Vec::new();
    };
    let Some(menu) = view.menuForEvent(&event) else { return Vec::new() };
    (0..menu.numberOfItems()).filter_map(|i| menu.itemAtIndex(i)).map(|item| item.title().to_string()).collect()
}

/// Whether the system spell checker flags `word` in English. (With
/// automatic language detection, one isolated word can be guessed as another
/// language and pass, so the language is given.)
pub fn is_misspelled(word: &str) -> bool {
    let checker = objc2_app_kit::NSSpellChecker::sharedSpellChecker();
    let mut count: isize = 0;
    // SAFETY: documented NSSpellChecker method; wordCount out-pointer is valid.
    let range: objc2_foundation::NSRange = unsafe {
        msg_send![&*checker, checkSpellingOfString: &*NSString::from_str(word), startingAt: 0isize,
            language: &*NSString::from_str("en"), wrap: false, inSpellDocumentWithTag: 0isize, wordCount: &mut count]
    };
    range.length > 0
}

/// Accessibility role and value of an editor's text view.
pub fn text_accessibility(view: &NSTextView) -> (String, String) {
    use objc2::runtime::AnyObject;
    let get = |name: &str| -> String {
        let v: Option<Retained<AnyObject>> = objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
            msg_send![view, accessibilityAttributeValue: &*NSString::from_str(name)]
        }))
        .ok()
        .flatten();
        let d: Option<Retained<NSString>> = v.and_then(|v| unsafe { msg_send![&*v, description] });
        d.map(|d| d.to_string()).unwrap_or_default()
    };
    (get("AXRole"), get("AXValue"))
}
