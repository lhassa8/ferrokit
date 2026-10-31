# Web, PDF, video and the system's services

This chapter covers the views that show other kinds of content (web
pages, PDFs, video) and the system services apps hand things to: the Share
menu, printing, notifications and the menu bar. The `media` example is a
trip briefing that uses all of them:

```bash
cargo run -p ferrokit --example media
```

## Web pages

`web_view` shows a [`WebPage`](crate::WebPage): WebKit's own view, with the
system's scrolling, text selection, find and Look Up. The page says what to
load and reports its title, address and loading state as signals, so views
showing them follow along, as with SwiftUI's `WebPage`:

```rust,no_run
use ferrokit::*;

let page = WebPage::new();
page.load("https://www.rust-lang.org");
let (p1, p2, p3) = (page.clone(), page.clone(), page.clone());
let browser = vstack((
    hstack((
        button("Back", move || p1.back()).disabled(move || !p2.can_go_back()),
        label(move || p3.title()).font(Font::headline()),
        spacer(),
    )),
    web_view(&page),
));
# let _ = browser;
```

- **Loading:** `load(url)` for a web address, `load_html(html, base_url)`
  for HTML the app makes, `load_file(path)` for a local file (allowed to
  read the files beside it).
- **State:** `title()`, `url()`, `is_loading()`, `can_go_back()` and
  `can_go_forward()` read signals; `back()`, `forward()` and `reload()`
  navigate.
- **Scripts:** `evaluate(script, |result| ..)` runs JavaScript in the page
  and hands back its result as text.
- **`.links_open_in_browser()`** sends the links the user clicks to their
  browser: for help pages and HTML the app made itself.

## PDFs

`pdf_view` is Preview's view: zoom, text selection, copy and find.

```rust,no_run
use ferrokit::*;

let page = Signal::new(0);
let count = Signal::new(0);
let query = Signal::new(String::new());
let viewer = pdf_view(|| Some("manual.pdf".into()))
    .page(&page)
    .page_count(&count)
    .highlight(move || query.get());
# let _ = viewer;
```

- **`.page(&signal)`** is the page shown, from 0, both ways: scrolling
  changes it, and setting it goes there.
- **`.page_count(&signal)`** reports the number of pages.
- **`.highlight(|| query)`** highlights every match and goes to the first.
- **`.single_page()`** and **`.two_up()`** change the layout from a
  continuous scroll.
- **`pdf_view_data(|| bytes)`** shows a PDF held in memory.

## Video and sound

`video_player` is AVKit's player, with the system's controls, Picture in
Picture and full screen. It plays sound files too.

```rust,no_run
use ferrokit::*;

let playing = Signal::new(false);
let time = Signal::new(0.0);
let length = Signal::new(0.0);
let player = video_player(|| Some("trailer.mov".into()))
    .playing(&playing)
    .time(&time)
    .duration(&length)
    .controls(VideoControls::Floating);
# let _ = player;
```

- **`.playing(&signal)`** is both ways: the controls set it, and setting it
  plays or pauses.
- **`.time(&signal)`** follows playback four times a second; setting it
  jumps there (a chapter list, a "back 10 seconds" button).
- **`.duration(&signal)`** reports the length once it's known.
- **`.muted(|| ..)`**, **`.loops()`**, and **`.controls(..)`** (`Inline`,
  `Floating`, `Minimal` or `Hidden`).
- **`video_stream(|| Some(url))`** plays from a web address (an HLS
  playlist or a video file on a server).

## Sharing

`share_button` shows the system's Share menu (Mail, Messages, AirDrop,
Notes…) for what its function returns when it's clicked:

```rust,no_run
use ferrokit::*;

let share = share_button(|| vec![ShareItem::File("report.pdf".into()), ShareItem::Text("Our trip".into())]);
# let _ = share;
```

Items are `ShareItem::Text`, `ShareItem::Url` or `ShareItem::File`.

## Printing and PDFs

`print(window, title, view)` prints any view: it's built again on its own,
laid out at the page's width, and split over as many pages as it needs,
with the Print sheet on `window` (where the user can also save a PDF).
`save_pdf(view, path)` and `pdf_data(view)` make the same pages with no
panel.

```rust,no_run
use ferrokit::*;

fn report() -> impl View {
    vstack((
        text("July report").font(Font::large_title().weight(Weight::Bold)),
        bar_chart(|| vec![("Lodging".to_string(), 1240.0), ("Food".to_string(), 520.0)]),
    ))
    .alignment(Align::Start)
}

let print_button = button("Print…", || {
    if let Some(w) = focused_window() {
        print(&w, "July report", report());
    }
});
save_pdf(report(), std::env::temp_dir().join("July report.pdf")).ok();
# let _ = print_button;
```

What prints: text, pictures, symbols, charts, backgrounds, borders and
rounded corners, always in the light appearance (paper is white). Shadows
and materials don't print. Pages get half-inch margins when the printer
gives none.

## Notifications

```rust,no_run
use ferrokit::*;

let tab = Signal::new(0);
notify(
    Notification::new("Ferry to Moskenes")
        .subtitle("Monday 6 July, 15:30")
        .body("Be at the terminal by 14:45.")
        .sound()
        .on_click(move || tab.set(1)),
);
```

The first notification makes macOS ask the user whether to allow the app's
notifications. They show as banners while the app is in front too, and
`.on_click(..)` runs when the user clicks one while the app is running.
`.id(..)` replaces an earlier notification with the same id.

Notifications need an app bundle with a bundle identifier
(`cargo ferrokit bundle`, or `cargo ferrokit run`); from a plain `cargo
run` binary, `notify` does nothing and `notifications_supported()` is
false.

## The menu bar

`App::menu_bar_extra` puts an item on the right side of the menu bar, with
a menu or a panel of the app's own views:

```rust,no_run
use ferrokit::*;

let days = Signal::new(3);
App::new("Countdown")
    .menu_bar_extra(
        MenuBarExtra::new("ferry")
            .title(move || format!("{} days", days.get()))
            .menu(vec![menu_item("Reset", move || days.set(0)), menu_separator(), menu_item("Quit", quit)]),
    )
    .run();
```

- **`MenuBarExtra::new(symbol)`** shows an SF Symbol, drawn to match the
  menu bar; `.title(..)` adds text beside it that follows its signals.
- **`.menu(items)`** shows a menu when it's clicked; **`.panel(view)`**
  shows a panel of views under it (as the Wi-Fi and Sound panels do),
  built once so it keeps its state.
- **`App::menu_bar_only()`** makes a menu-bar app: no Dock icon and no menu
  bar of its own.

## Testing

`testing::hold_notifications()` makes `notify` record notifications
instead of posting them (no permission prompt), `sent_notifications()`
reads them and `click_notification(id)` runs a click.
`testing::shared_items(button)` says what a share button would share, and
`testing::menu_bar_extras()` returns the status items and their panels'
popovers. The `media` example's `CHECK=all` uses all of them.
