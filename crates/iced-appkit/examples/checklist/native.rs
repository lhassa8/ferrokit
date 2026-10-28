//! Test scaffolding: plain AppKit views built directly with objc2 (the real
//! table and text editor widgets come in M2/M3).

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSButton, NSControlTextEditingDelegate, NSColor, NSFont, NSScrollView, NSTableColumn, NSTableView, NSTableViewDataSource, NSTableViewDelegate,
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

pub fn mount_text(id: u64, initial: &str) {
    let mtm = MainThreadMarker::new().unwrap();
    let scroll = NSTextView::scrollableTextView(mtm);
    if let Some(text) = scroll.documentView().and_then(|v| v.downcast::<NSTextView>().ok()) {
        text.setRichText(false);
        text.setFont(Some(&NSFont::systemFontOfSize(14.0)));
        text.setString(&NSString::from_str(initial));
        text.setAllowsUndo(true);
    }
    host::mount(ViewId(id), Retained::into_super(scroll));
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
