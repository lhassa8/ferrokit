# Changelog

## 0.4.0 (unreleased): one name, Ferrokit

The crates take the project's name and share one version number, released together:

| Was | Now |
| --- | --- |
| `appkit-ui` 0.3 | **`ferrokit`** 0.4: the framework, what most apps depend on |
| `iced-appkit` 0.2 | **`ferrokit-iced`** 0.4: real AppKit views in Iced windows |
| `appkit-host` 0.2 | **`ferrokit-core`** 0.4: the native layer the other two share |
| `cargo-ferrokit` 0.1 | `cargo-ferrokit` 0.4: same name, new version line |

Moving over: change the dependency (`ferrokit = "0.4"`, `ferrokit-iced = "0.4"`) and the paths (`use ferrokit::*`, `ferrokit_iced::…`, `ferrokit_core::…`). Nothing else in the API changed with the rename. `cargo ferrokit new` makes apps on `ferrokit` 0.4; its `--appkit-ui-path` option is now `--ferrokit-path`.

The old names get one last release (`appkit-ui` 0.4.0, `iced-appkit` 0.3.0, `appkit-host` 0.3.0) that only re-exports the new crate, so an old dependency still builds and its crates.io page says where the project went. Their earlier versions stay as they were.

Also renamed, for anyone who relied on them: the Objective-C class prefixes (`AppKitUi…` → `Ferrokit…`, `AppKitHost…` → `FerrokitCore…`) and a few identifiers the framework stores, such as its recent-documents defaults key and the Settings window's autosave name, so recent documents and the Settings window's position start fresh.

### Styling (Phase 3, V1)

- **Text styles:** `text(..)` and `label(..)` take `.font(..)` and `.color(..)` (`label` also `.color_when(..)`). `Font` has the system's text styles (`large_title`, `title`, `headline`, `body`, `caption`…), `system`, `monospaced` and `named` fonts, with `.weight(Weight::..)`, `.bold()`, `.size(..)`, `.italic()` and `.monospaced_digits()`.
- **Surfaces on any view:** `.background(..)`, `.corner_radius(..)`, `.border(..)`, `.shadow(Shadow::soft())` and `.opacity(..)`. Colours resolve again on appearance changes, so cards follow Dark Mode.
- **Pictures:** `image_file(..)` and `image_data(..)`, fitted or `.fill()`ed, with `.corner_radius(..)`. SF Symbols take `.symbol_size(..)`, `.symbol_weight(..)`, `.color(..)`, `.hierarchical(..)`, `.palette(..)` and `.multicolor()`.
- **`Color`** (was `TextColor`, and merges the colour well's `Color` struct): it adds `Quaternary`, the system fills (`Fill`, `SecondaryFill`, `TertiaryFill`, `QuaternaryFill`), `Accent`, `WindowBackground`, `ControlBackground`, `Clear`, `Rgba`, plus `Color::rgb`, `rgba`, `hex`, `.opacity(..)` and `.components()`.
- **Changed: modifiers apply in the order written,** as in SwiftUI. A modifier that has to go around what's already there (padding outside a frame or background, a second frame) adds a box: `.frame(18.0, 18.0).padding(7.0)` is an 18-point view with 7 points around it. Before, all of a view's modifiers applied together.
- **Changed: a stack's flexible children share its length equally.** A child whose own length is larger keeps it. So a row of cards that `fill_width()` come out the same width; before, each got its own width plus an equal share of the extra.
- A new guide chapter, Styling, and a new example, `styles`, with 12 self-checks, from fonts read back to colours, corners and shadows sampled from real window captures in light and dark.

### Materials and window chrome (Phase 3, V2)

- **Materials:** `.material(Material::Sidebar)` (and header, title bar, popover, menu, HUD, sheet, window and content backgrounds, selection, tooltip) puts the system's translucent material behind any view. It follows Dark Mode, Reduce Transparency and the window's active state; `.corner_radius(..)` rounds it.
- **Liquid Glass:** `.glass(Glass::Regular)` (or `Clear`, or `Tinted(color)`) on macOS 26 and later, with a translucent material in its place on macOS 15. `has_liquid_glass()` says which.
- **Layers:** `zstack(..)` puts views on top of one another (text or glass over a picture, a badge over an icon), with `.alignment(horizontal, vertical)`.
- **Window chrome:** `Window::full_size_content()`, `.transparent_title_bar()`, `.hide_title()`, `.toolbar_style(ToolbarStyle::UnifiedCompact)` (and `Unified`, `Expanded`, `Preference`) and `.background(color)`. With full-size content, backgrounds, materials and split panes reach the top of the window, so a sidebar runs up behind the window buttons, while the content starts below the title bar.
- A new guide chapter, Materials and windows, and a new example, `materials`, with 13 self-checks including captures in light and dark.

### Motion (Phase 3, V3)

- **Animated changes:** `animated(|| signal.set(..))` animates what the change causes: views moving or resizing, opacity and colours fading. `animated_with(Animation::ease_out(0.4), ..)` sets the timing.
- **State-driven looks:** `.opacity_when(..)` and `.background_when(..)` follow signals, and fade inside `animated`.
- **Transitions:** `.transition(Transition::Fade)` or `Transition::Slide(Edge::Leading)` for views that `dynamic` and `for_each` add and remove. A removed view stays until it has faded out.
- **Hover and click on any view:** `.on_hover(..)`, `.hover_highlight(Color::Quaternary)` and `.on_click(..)`. A clickable view dims while pressed, and VoiceOver reads it as a button named by its text.
- **Reduce Motion is respected:** nothing moves, slides become fades. `reduce_motion()` reads the setting.
- A new guide chapter, Motion, and a new example, `motion`, with 15 self-checks that read Core Animation's in-flight values.

### More controls and charts (Phase 3, V4)

- **Tabs:** `tabs(&page, pages)`, AppKit's tab view, or `.style(TabStyle::Segmented)` for a segmented control over borderless panes.
- **Controls:** `radio_group`, `link`, `gauge` (with warning and critical levels), `rating` (stars), `token_field` (with suggestions) and `path_control`, each bound two ways to a signal.
- **Tooltips:** `.tooltip(..)` on any view.
- **Charts:** `stacked_bar_chart` (or `.grouped()`), `area_chart`, `scatter_chart` and `pie_chart` (or `.donut()`) join `bar_chart` and `line_chart`. All of them have legends, colours per series (`.colors(..)`, `Series::color(..)`) and formats for values and the x axis: `Format::Currency("USD")`, `Percent`, `Decimals(n)`, `Date` and `DateTime`. Date axes tick on whole local days, weeks, months or years.
- **Changed:** `bar_chart` and `line_chart` return a `Chart` (with `min_size` and `exportable`, as before) instead of a `Canvas`.
- A new guide chapter, More controls and charts, and a new example, `components`, with 16 self-checks.

### Web, PDF, video and the system's services (Phase 3, V5)

- **Web pages:** `web_view(&page)` shows a `WebPage` in WebKit's own view. The page loads web addresses, HTML or local files, and reports its title, address, loading state and history as signals; `evaluate(..)` runs JavaScript. `.links_open_in_browser()` sends clicked links to the browser.
- **PDFs:** `pdf_view(|| path)` (or `pdf_view_data`) is Preview's view, with `.page(&signal)` both ways, `.page_count(..)`, `.highlight(|| query)` and single-page or two-up layouts.
- **Video and sound:** `video_player(|| path)` and `video_stream(|| url)` are AVKit's player, with `.playing(..)` and `.time(..)` both ways, `.duration(..)`, `.muted(..)`, `.loops()` and the system's control styles.
- **Sharing:** `share_button(|| items)` shows the system's Share menu for text, web addresses or files.
- **Printing:** `print(window, title, view)` prints any view laid out at the page's width over as many pages as it needs, with the Print sheet; `save_pdf(view, path)` and `pdf_data(view)` make the same pages with no panel. Backgrounds, borders, pictures and charts print, in the light appearance.
- **Notifications:** `notify(Notification::new(..).body(..).sound().on_click(..))` posts through Notification Center (from an app bundle), with banners while the app is in front and clicks routed to the app.
- **Menu bar:** `App::menu_bar_extra(MenuBarExtra::new(symbol).title(..).menu(..))`, or `.panel(view)` for a panel of the app's own views; `App::menu_bar_only()` for an app without a Dock icon.
- **Fixed: printed text.** AppKit's labels write their text into a printed PDF twice, a few points apart, so finding or copying text in the PDF went wrong. Ferrokit's `text` and `label` now print their text once.
- **Testing:** `testing::hold_notifications`, `sent_notifications`, `click_notification`, `shared_items` and `menu_bar_extras`.
- A new guide chapter, Web, PDF, video and services, and a new example, `media`, with 20 self-checks: the printed report's pages, text and card backgrounds, the web page's title and a script, PDF pages and search, playback of a sound made with `say`, sharing, notifications and their clicks, both kinds of menu-bar extra, and the Print sheet.

### The SQL browser, polished (Phase 3, S1)

The flagship example now uses the visual layer, and building it added to the framework:
- **`App::welcome(make)`:** a welcome window at launch instead of an untitled document, closed when a document opens, back with Window ▸ Welcome to … (⇧⌘1) or `show_welcome()`. `welcome_window()` returns it.
- **`DocumentType::window(|w| ..)`:** document windows take `Window`'s chrome (full-size content, toolbar styles).
- **`choose_documents()`:** the Open panel for the app's document types, from a button.
- **Under the title bar:** the columns of a top-level row with a material or background now reach up under the title bar, as split panes do. A stacked split's `.position(..)` counts from below the bar.
- **Fixed:**
  - a clickable view's `.accessibility_label` named every view inside it as well as its button, so VoiceOver read it twice; it now names the button only;
  - editors and mounted views dropped during exit could panic on torn-down thread-locals.

The app itself:
- **Welcome window:** the sample, Open…, a scratch database, and recent databases in a sidebar.
- **Main window:** the sidebar's material runs under the title bar.
- **Status line:** the row count as a headline and the timings in secondary text; an error in red with a symbol.
- **Inspector:** the table's name, "Row 2 of 3" and the fields in a grouped card (NULL in italic grey, numbers with even digits), with an empty state, fading between rows.
- **Chart:** up to four number columns as series (lines, or grouped bars), with a legend and number formats.
- **German:** translations for all of the new text.
- **Checks:** 5 new: the welcome window, its menu item, its recent list, the chart's legend, and the welcome window in German.

### Habits, a new showcase app (Phase 3, S2)

A habit tracker built to sit next to Apple's apps (`examples/habits.rs`), and what the framework gained on the way:
- **`Painter::stroke_arc`:** part of a circle with round ends, for progress rings and gauges.
- **Dates:**
  - `format_date(seconds, template)` writes dates as the user's locale does ("Saturday, 31 October");
  - `today()`, `local_day`, `local_day_start` and `weekday` count calendar days in the user's time zone, through daylight saving.
- **`.disabled(|| ..)` on any view:** before, only buttons could be disabled.
- **Daily notifications:** `Notification::daily_at(hour, minute)`, and `cancel_notification(id)`.
- **`testing::canvas_click`:** runs a canvas's click handler at a point.
- **Fixed:**
  - `.hover_highlight` hid a view's own background (a selected row lost its selection colour), and now draws only where that background is clear;
  - a canvas without a description was an unnamed image to VoiceOver, and is now skipped as decoration.

The app:
- **Window:** a sidebar in the system material.
- **Today:** a summary card with a progress ring, and a card per habit with its streak, the week's dots and a check; a click marks it done, animated.
- **Each habit:** stat cards, a six-month calendar you can click, a weekday chart, a daily reminder, Share and Delete.
- **Insights:** an area chart, weekly stacked bars by habit with a legend, and a donut.
- **Also:**
  - a New Habit sheet;
  - a menu-bar panel to check habits off;
  - a printed or PDF report;
  - storage in user defaults.
- **Checks:** 17 self-checks, and captures in light and dark.

### Side by side with Apple's rendering (Phase 3, S3)

Both showcase apps were compared with a SwiftUI reference of the same screens (`docs/S3_REVIEW.md`), and the framework now matches it:
- **Toolbars show symbols alone,** as Apple's apps do (the bar is 52 points, not 66). The Expanded and Preference styles keep labels.
- **Sidebars:**
  - rows follow the user's sidebar icon size, with icons and titles centred and long titles truncated;
  - section headings are small, bold and secondary;
  - `.icon_color(..)` gives each row's symbol its own colour.
- **`Color::Fill`, `SecondaryFill`, `TertiaryFill`, `QuaternaryFill`:** the system fills; `QuaternaryFill` is the grouped-card colour of `GroupBox` and grouped forms.
- **`.padding_xy(horizontal, vertical)`:** padding per axis.
- **The apps:**
  - Habits uses the real sidebar (arrow keys, type-select, the system's selection) and flat grouped cards;
  - the SQL browser's inspector is a grouped form: names on the left, values right-aligned in secondary text.

### Real-input testing (Phase 3, Q1)

- **`testing::input`:** clicks, double-clicks, typing and keys with modifiers, posted to the app's own event queue so AppKit dispatches them along the user's path (hit-testing, the first responder, the field editor, key equivalents). `ready(&window)` brings the app and window to the front first. Window-server input (`server_click`, `server_type`, through `CGEvent`s) runs when the process is trusted for Accessibility; `server_available()` says whether it is.
- **Fixed, found by real input:** a shortcut typed straight after text in an editor (⌘R after a query) ran on the old text, since the editor's changes reached its bound signal only on the next pass. Menu items, buttons and commands now deliver pending edits before they act.
- **Checks:**
  - a new `input` example: 10 checks, plus window-server input, skipped without trust;
  - real-input checks in `habits` (4) and `sql_browser` (2);
  - check-all reports skipped checks.

### Soak tests (Phase 3, Q2)

- **`testing::soak`:** an app's list of weighted user actions, run at random from a printed seed for `SOAK` minutes. A watchdog thread reports main-thread hangs over 2 s, and the footprint and live views, signals and windows are sampled every 30 s. Growth is reported per 1,000 actions, so time paused doesn't count. The run pauses while another app is in front.
- **`scripts/soak.sh <example> [minutes] [seed]`** runs a soak and summarises it: exit code, crash reports, exceptions and assertions in the log, hangs, and the footprint over time. `habits` and `sql_browser` have soaks.
- **Fixed: views that entered with a slide inside an animated change could stay unplaced.** After an animated switch to a page whose content lays out in a later pass (a scroll view's), its labels kept the frame they were made with: invisible, at the corner. The motion layer now knows which views layout has placed:
  - a view's first placement never animates;
  - slides only move views already placed.
- **Habits' demo:** `DEMO=1` runs a fast, scripted tour of everything, in a window sized to the screen.

### API review (Phase 3, Q3)

The review is in `docs/Q3_API_REVIEW.md`. Changes:
- **One name per modifier, taking a value or a closure** (`impl IntoValue<T>`; types `IntoValue` and `Value`):
  - `text(..)`, `.color(..)`, `.background(..)`, `.opacity(..)`, `.disabled(..)`;
  - `button(title, ..)`, `menu_item(title, ..)`, `menu_command(title, ..)`;
  - a menu item's `.disabled(..)` and `.checked(..)`, and a toolbar item's `.disabled(..)`.
- **Deprecated, still working for one release:** `label`, `background_when`, `opacity_when`, `color_when`, and a menu item's `enabled_when`, `title_when` and `checked_when`.
- **`.font(..)` and `.color(..)` on any view and after any modifier,** for the text and symbols inside.
- **One view alone as a stack's children:** `vstack(for_each(..))`.
- **`text(..).truncate()`:** a row short of room takes it from truncating text before overflowing.
- **`.min_width(..)`, `.min_height(..)`.**
- **`button(..).symbol(..)`:** a symbol on a button, from signals too.
- **`.accessibility_value(..)`:** what VoiceOver reads after a view's name; Habits' cards say whether they're done.
- **A new `values` suite** with 11 checks; the examples, guide and templates use the new names.

### New controls (the C queue, after the coverage review)

- **Button styles and sizes:** `.style(ButtonStyle::…)` (borderless, inline, recessed, circular, toolbar, Liquid Glass), `.control_size(ControlSize::…)` and `.destructive()`.
- **`menu_button`:** a pull-down button with a menu of actions, using the menu bar's `MenuItem`s; a title, a symbol or both.
- **`help_button`:** AppKit's round "?" button, named "Help" for VoiceOver.
- **Sliders:** `.ticks(n)`, `.snap_to_ticks()`, `.vertical()`, `.circular()` (a dial) and `.control_size(..)`.
- **Window tabs, full screen, the Dock:** `Window::tabbing(Tabbing::…, group)` and `open_tab`; `.no_full_screen()`, `toggle_full_screen`, `is_full_screen`; `set_dock_badge` and `App::dock_menu`.
- **Panels:** `Window::panel(PanelStyle::Floating | Utility | Hud)` opens an `NSPanel`: floating above the app's windows, hidden while the app is inactive, out of the Window menu, and key only when a field in it needs the keyboard.
- **SF Symbol effects:** `image(..).effect(SymbolEffect::Pulse, active)` runs one while a condition holds; `.effect_on(SymbolEffect::Bounce, trigger)` plays one each time a value changes (bounce, pulse, variable colour, scale, wiggle, rotate, breathe).
- **Toolbar groups, pickers and customisation:** `toolbar_group` (buttons in one capsule), `toolbar_picker` (a choice of one as segments, bound to a signal), `toolbar_fixed_space`; `Window::customizable_toolbar(name)` opens AppKit's customisation sheet from the toolbar's menu or `customize_toolbar_item()`, offers `.optional()` items, and remembers the user's layout. The SQL browser's toolbar is customisable, with Export and Copy offered.
- **Chart rules, histograms, heatmaps and sparklines:** `.rule(Rule::y(v))` and `Rule::x(x)` draw a labelled threshold, target or average across or down a chart (its range takes them in; VoiceOver reads them); `histogram(values)` counts values in round-number bins (`.bins(n)`); `heatmap(cells)` shades a grid of `(column, row, value)` with a scale above; `sparkline(values)` is a small line with its last value marked, at a fixed height.
- **`progress(..).spinner()`:** a spinner (spinning while the amount is unknown, a filling circle when known), with `.control_size(..)`.

### Fixed in the hands-on checks (Phase 3, Q4)

- **Typing in the middle of highlighted text moved the caret** to the end of the restyled span (in the SQL browser's query, typing after `SELECT ` jumped past the next word). An edit now styles only its own characters during the edit and the rest just after it.
- **Toolbar buttons with a `.disabled(..)` condition (all `toolbar_command`s) stayed disabled:** their state was computed once, before the window's content could answer. AppKit now validates them as events arrive, as for menu items.
- **Code editors kept the Mac's prose settings:** typing `"` in the SQL browser's query could give `“`, which SQL rejects. `.syntax(..)` editors, and the new `.code()`, now turn off smart quotes and dashes, text replacement, autocorrection and spell checking (`TextModel::set_code_input` in `ferrokit-core`).
- **The title starts beside the sidebar,** as in Mail and Notes: when a window's view is a side-by-side `split` and it has a toolbar, the toolbar starts with an empty navigational item as wide as the sidebar, which follows its divider and collapse. Before, the title began over the sidebar and ran across its edge. (Each window's toolbar has its own identifier now, so windows don't share items.)
- **A toolbar button acts in its own window,** not in whichever window is key (or in none, when the app is inactive, as when VoiceOver presses it).
- **A menu bar extra's panel fits what it shows now,** not what it showed at launch: it's measured again each time it opens (Habits' panel cut off habits added since).
- **New: `time_picker`,** a time of day (hour and minute) bound to a `Signal<(u32, u32)>`, an `NSDatePicker` showing only the time. Habits' reminder uses it, so a reminder can be at 7:45, not only on the hour (found in the hands-on checks: the time menu offered whole hours).
- **`testing::ax_audit` also flags controls read as an SF Symbol's name** (a button VoiceOver reads as "sun.max.fill"), not only images.

### ferrokit-core (was appkit-host)

- **Fixed: typing over a selection in a highlighted editor could crash the app.** Restyling set the editor's typing attributes in the middle of the edit, while the text view still held the old selection; AppKit read attributes past the end of the shorter text and threw, which left TextKit 2's layout behind the text, so the typed text didn't show and the next redraw aborted. Typing attributes are now left alone during an edit, and highlighting runs in `willProcessEditing` as part of the edit itself.

### ferrokit (was appkit-ui)

- **Fixed: an exported chart included the hover readout.** Copying, saving or dragging out a canvas while the pointer was over it captured the highlighted bar and its tooltip; images are now made as the canvas looks without the pointer over it, and the hover comes back on screen afterwards.
- **Fixed: a document app's untouched Untitled window stayed open when a file was opened from code or File › Open….** Only files opened from the Finder closed it; now every way of opening a file does, as in TextEdit (the SQL browser opened its sample next to an empty Untitled window).
- **Fixed: hiding a collapsible split pane could leave a line across the pane that took its room.** After the SQL browser's chart was shown and hidden, a 1-point line stayed across the results table where the chart's edge had been; the panes are now laid out again when one collapses or opens.
- **Fixed: canvas colours lost their own transparency.** Drawing replaced a colour's alpha with the painter's opacity instead of multiplying them, so partly transparent system colours such as `TextColor::Separator` drew solid (the SQL browser's `results.csv` chip was a black box).

## appkit-ui 0.3.0, appkit-host 0.2.2, cargo-ferrokit 0.1.0 (2026-10-30)

`appkit-ui` 0.3 makes signals `Copy`, adds async data, commands, styled text, popovers, inspectors, a canvas with charts, the clipboard and drag and drop, and localisation. Its flagship example is a SQLite browser. `cargo-ferrokit` is new: it makes, runs, checks, bundles and notarizes apps. `appkit-host` only gains API (a patch release). `iced-appkit` is unchanged and stays at 0.2.1, whose `appkit-host` requirement 0.2.2 satisfies.

### appkit-ui 0.3.0

Changed (breaking; `cargo semver-checks`: `Signal` and `Memo` now implement `Copy`):

- **`Copy` signals:** `Signal` and `Memo` are `Copy` handles into a per-thread arena, so closures capture them without `.clone()`. Existing code still compiles; clippy's `clone_on_copy` points out the clones left.
  - Each signal belongs to the owner current when it was made, and is freed with it: a window's view tree (including signals made by its buttons' actions), a `dynamic` branch, a `for_each` row, or a document. Signals made outside any owner (in `main`) live for the app.
  - Reading a signal after its owner is gone panics with a message saying so; setting it does nothing. New: `Signal::try_get`, `Signal::is_alive`, `live_signals()` for leak checks. `Signal` compares by identity.

Added:

- **Async data:** `Resource::new(source, fetch)` loads on a background thread from the signals `source` reads, and again when they change.
  - A new load cancels the running one (`Cancel::is_cancelled`), so stale results are never shown, and the last value stays while the next loads.
  - `get`, `get_untracked`, `loading`, `error`, `state` (`Load`), `refetch`. Closing its window cancels the load.
- **Commands and focus:** `Command::new(name)`, handled by views (`ViewExt::on_command`, `on_command_when`), windows (`Window::on_command`) and the app (`App::on_command`).
  - `menu_command` and `toolbar_command` run the handler nearest the focused view, and are enabled only while one answers.
  - `ViewExt::focused_value` / `focused_value::<T>()`; `perform`, `can_perform`, `perform_in`, `focused_window`; `MenuItem::title_when`.
- **Styled text and syntax highlighting:** `TextEditor::highlight(|text| runs)` (`TextStyle`: a `TextColor`, background, bold, italic, underline).
  - It's applied after every edit, before it's drawn, restyling only the span that changed, and again when signals it reads change.
  - `TextEditor::syntax(Syntax::sql() / Syntax::rust() / Syntax::new()…)`.
- **Window furniture:**
  - `Split::stacked()` and `Split::collapsible(index, &signal)` for sidebars and inspectors: they hide with a signal or a drag, keep their size, and reopen at it. `Split::trailing(points)`.
  - `ViewExt::popover(&shown, || content)` / `popover_on(Edge, …)`: a native popover; Escape or a click outside closes it, and its content is freed.
  - `DocumentType::toolbar`.
- **Drawing and charts:** `canvas(|p: &mut Painter| …)`, redrawn when the signals it reads change or it resizes; `on_click`, `on_hover`, a VoiceOver `description`.
  - `bar_chart` and `line_chart` (`Series`): round-number axes, values under the pointer, a VoiceOver summary, and decimation (a million points draw as a few thousand).
- **Clipboard, drag and drop:**
  - `Clipboard` (general or private): text, PNG, files.
  - `DragItem`: text, PNG, a file, or a promised file written where it's dropped. `Canvas::draggable`, `Canvas::exportable` (Copy Image, Save Image…, drag out a PNG).
  - `Window::on_drop_files(extensions, action)`; document apps open dropped files of their kinds.
- **Localisation:** `t` (English keys) and `App::translations(&[(lang, .strings)])`, in the user's first preferred language the app has.
  - `t_with` (named placeholders), `t_plural` (CLDR plural rules), `format_number`, `set_language`.
  - The framework's own strings in German, French and Japanese.
- **Test helpers:** `testing::canvas_drawing`, `canvas_png`, `canvas_drag`, `fulfil`, `drop_on_view`.
- **New example, `sql_browser`:** a SQLite document app.
  - Schema sidebar; a highlighted SQL editor over a million-row results table; queries that cancel stale ones.
  - Row inspector, query history, charts, CSV export (drag `results.csv` to the Finder), German.

Fixed:

- A collapsible pane no longer collapses itself at the first layout.
- Popovers reopen after an animated close in an app that isn't active.

Known issue (fixed in the repository, no crate change needed): the `sql_browser` example didn't link on macOS 15, whose system SQLite is too old for rusqlite 0.41. The example now compiles SQLite in.

### appkit-host 0.2.2 (additive; semver-checks: no update required)

- `TableModel::set_columns(columns)`: new columns in the same native table (the sort is cleared). Text cells set their alignment on reuse too.
- `TextModel::set_highlighter`, `restyle`, `style_at`, with `TextStyle` and `TextColor` (system colours that follow Dark Mode, including `Background` and `Separator`; `TextColor::ns()`). Styled runs over plain text are applied before an edit is drawn and kept out of undo.
- `testing::drop_on_view`.

### cargo-ferrokit 0.1.0 (new)

- `cargo ferrokit new <path> [--document]`: a working app (a to-do list, or a text document app) with self-checks, on `appkit-ui` 0.3 (or a local checkout: `--appkit-ui-path`).
- `run`; `check` (clippy with warnings as errors, tests, a bundle, the app's `CHECK=all` self-checks, the signature); `bundle` (signed `.app`, hardened runtime, icon from a PNG, document types, languages, `--universal`, `--zip`); `notarize` (Developer ID, notary service, stapling; `--dry-run`).
- Settings in `[package.metadata.ferrokit]`.

## appkit-host 0.2.1, iced-appkit 0.2.1, appkit-ui 0.2.0 (2026-10-29)

`appkit-host` and `iced-appkit` only gain API (patch releases); `iced-appkit` re-exports the new `CellImage`, so it needs `appkit-host` 0.2.1. `appkit-ui` moves to 0.2.0: besides new features, some behaviour changed (see below).

### appkit-host 0.2.1 (additive)

- Group rows: `TableModel::set_group_by(key)` puts a header before each group (full width, floating, not selectable; sorting sorts within groups); `groups()`, `shown_rows()`. `row_at` is `None` for a header; event indices stay model indices.
- `Column::progress` (a bar from 0 to 1), `Column::icon` (`CellImage::Symbol` or `FileIcon` before the text), `Column::fit` (width from the longest values).

### iced-appkit 0.2.1 (additive)

- Re-exports `CellImage` (for `Column::icon`); requires `appkit-host` 0.2.1.

### appkit-ui 0.2.0

Changed behaviour:

- `image(symbol)` is decorative (hidden from VoiceOver) unless given a `.description`; it used to read the symbol name.
- `quit()` starts on the next run-loop pass (it used to terminate inside the caller's batch).
- Windows closed by the close button or ⌘W are torn down (views, effects, undo history), like `close_window`; the Settings window is kept.
- `layout_times_us()` keeps the most recent 10,000–20,000 passes (it grew forever); `layout_passes()` gives the total.

Added:

- **Document apps:** `App::documents(DocumentType)`.
  - The File menu gets New, Open…, Open Recent, Save, Save As… and Revert to Saved.
  - Each window is a `Document<T>`: a content signal and a file. The title bar shows the edited dot and the file's proxy icon.
  - Closing or quitting with changes asks to save.
  - `new_document`, `open_document`, `save_document(_as)`, `revert_document`, `document_edited`, `recent_documents`.
- Document apps open files from the Finder (`application:openURLs:`): double-click, Open With, drops on the Dock icon; the untouched Untitled window from launch then closes. `package-app.sh --doc-type "Name:ext,ext"` declares the types.
- `dynamic_submenu(title, items)`: a submenu rebuilt each time it opens.
- `collection(items, describe)`: a grid of items (`NSCollectionView`), virtualised (cells reused), with a title, subtitle and an image (SF Symbol, file icon or colour) per item; two-way selection by id (single or multiple); double-click and Return activate; VoiceOver reads items by title. `collection_cells_made()` diagnostic.
- More controls, each bound two ways to a signal: `date_picker` (`SystemTime`; with time, calendar style, range), `color_well` (`Color`), `segmented`, `combo_box` (text plus suggestions), `search_field` (placeholder, submit, clear).
- `testing::ax_audit` also requires names on date pickers and colour wells, and skips the parts AppKit draws inside a named date picker.
- `quit()` now starts on the next run-loop pass, so prompts shown while quitting don't run inside the caller's batch.
- Fix: a scroll view re-fits its content whenever AppKit changes the visible width (classic scroll bars appearing while drawing).
- Fix: with classic (always-shown) scroll bars, as on a Mac with a mouse, a `scroll` view's content stayed wider than the view once the vertical bar appeared (a stray horizontal scroll bar, clipped content). The content now re-fits.
- `testing::heap_in_use_mb()`: live allocations, for leak checks (footprint lags while the allocator returns freed pages).
- `testing::capture_window` returns `None` for a capture without the window's content (no screen-recording permission).

## 0.2.0 (2026-10-29)

`appkit-host` 0.2.0 has breaking changes, and `iced-appkit` re-exports its types (`host`, `TableEvent`, `Column`…), so both move to 0.2.0. `appkit-ui` 0.1.0 is new.

### appkit-host 0.2.0 (breaking)

- `TableEvent` is now `#[non_exhaustive]` and has a new `Edit { row, column, value }` variant.
- `Column::editable(true)` makes a text or number column's cells editable. Finishing an edit emits `TableEvent::Edit`; apply it with `update_row`.
- `TableModel::scroll_view()` returns the table's scroll view, for hosts that place it themselves.
- `observe_appearance()` is public, for apps without an attached host window.
- `TextModel::scroll_view()`.
- `TableModel::append(rows)` adds rows for live data: when unsorted, its cost doesn't depend on the rows already there, and the selection is kept. `scroll_to_end()` and `is_at_end()` support following a tail.
- Tables copy: ⌘C copies the selected rows as tab-separated text (columns in their shown order). `TableModel::selected_text()`, `copy_to(&pasteboard)`.
- Opt-in table transfers, each reported as a new `TableEvent` that the app applies:
  - `set_row_drag(true)`: rows drag out as text.
  - `set_reorderable(true)`: `RowsMoved { rows, before }`; apply with the new `move_rows`. Refused while sorted.
  - `set_file_drop(Some(&["csv"]))`: `FilesDropped { paths, before }`.
  - `set_pastes(true)`: ⌘V gives `Paste(text)`; `paste_from(&pasteboard)`.
- `testing`: `drag_rows`, `drop_on_table`, `files_pasteboard`, and private-pasteboard helpers, for checking drags without a mouse or the clipboard.
- `TextModel::share_undo(Some(&manager))` records the editor's edits in another undo manager (a window's, shared with other views); `None` returns to a history of its own, still the default. `TextModel::undo_manager()` returns the one in use. `set_text` drops only this editor's steps from a shared history.
- Fix: `TextEvent::Changed` is now sent for undo and redo too. AppKit sends no `textDidChange:` for them, so the model also listens to its text storage; a queued `Changed` isn't repeated.

### iced-appkit 0.2.0

- `table(..).on_edit(..)` for editable columns.
- Depends on `appkit-host` 0.2 (its re-exported `TableEvent` gained variants and is now `#[non_exhaustive]`).

### appkit-ui 0.1.0 (new)

A native macOS UI framework on `appkit-host`: real AppKit views built once and updated in place by fine-grained signals.

- Signals, memos, effects, batching, background work (`spawn`), stored settings.
- Layout in Rust: stacks, grids, spacers, padding and sizes, keyed lists (`for_each`), `dynamic` subtrees, subtree-only relayout.
- Controls bound to signals: text and secure fields, toggles, checkboxes, pickers, sliders, steppers, progress, SF Symbols, buttons; `.accessibility_label`.
- Deep widgets: `table` (sorting, editing, copy, drag and drop), `outline` and `sidebar`, `text_editor`.
- App shell: menus with shortcuts and state, toolbars, split views, sheets, alerts, open and save panels, Settings, window and split restoration, per-window undo, `on_frame` for live data; windows torn down when closed.
- `testing`: self-check helpers, accessibility audits, window captures.
- A guide (`guide/`) whose examples compile as doc tests; example apps `settings`, `native_csv`, `log_viewer`, `hands_on`.

## 0.1.0 (2026-10-28)

First release of `iced-appkit` and `appkit-host`: real AppKit controls inside Iced 0.14 windows on macOS 15+.

- **Embedding core:**
  - `NativeSlot` places a native view in the Iced layout, clipped inside scrollables and snapped to the pixel grid.
  - `root()` keeps native views in step with Iced's frames, and swaps them for pictures of themselves while any Iced overlay (dropdown, tooltip, menu) covers them. `cover()` does the same for modal layers.
  - Native views join Iced's Tab order, wrapping at the ends. Clicks move focus across the boundary.
  - Edit shortcuts and an optional Edit menu go to the focused side.
  - `theme::system()` follows the system colours, accent colour and Dark Mode.
- **Table:** `TableModel<T>` + `table()`.
  - Lazy rows (1,000,000 rows at 60 fps with flat memory).
  - Text, number and checkbox columns; resizable and reorderable columns with saved state.
  - Natural-order sorting in Rust (100,000 rows in ~70 ms). The selection is kept across sorts, inserts and removes.
  - `on_select`, `on_activate` (double-click/Return), `on_sort`, `on_toggle`.
  - Type-select and full VoiceOver structure.
- **Text editor:** `TextModel` + `text_editor()`.
  - Plain text with IME composition events (`on_composition`), and `on_change` held back while composing.
  - Spell and grammar checking with suggestions, Look Up, Translate, find bar, fonts, insets, read-only mode.
  - Undo kept per editor. Tab inserts a tab; Ctrl-Tab moves on.
  - UTF-8 byte offsets in the API.
- **Examples:** `csv_browser` (demo) and `checklist` (regression suite, `CHECK=all`).

Known limits:
- One window per app.
- An overlay appears ~2 frames after opening, while the native picture is prepared.
- Native views are hidden from accessibility while an overlay covers them.
- Iced's own widgets aren't exposed to VoiceOver.
- No rich text or editable table cells yet.
