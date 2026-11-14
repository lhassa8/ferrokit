# The ferrokit guide

`ferrokit` builds Mac apps in Rust from real AppKit views: an actual
`NSTableView`, `NSTextView`, `NSSplitView` and so on. You describe the views
once, connect them to signals, and each native view updates in place when
the state it reads changes. There is no virtual DOM and no diffing.

Every Rust example in these chapters is compiled by `cargo test --doc`, so
the guide can't drift from the API.

AI coding agents: read `AGENTS.md` in the crate's folder first (the rules,
recipes and known limits on one page), then `API.md` beside it for every
public item. `cargo ferrokit agents` writes `AGENTS.md` into an app.

1. [Getting started](01-getting-started.md): an app, a window, a button.
2. [State](02-state.md): signals, memos, effects, batching, background work, async tasks.
3. [Views and layout](03-views-and-layout.md): stacks, grids, modifiers,
   lists and identity.
4. [Controls and forms](04-controls-and-forms.md): fields, toggles,
   pickers, sliders, and naming them for VoiceOver.
5. [Tables, outlines and editors](05-tables-outlines-editors.md): the deep
   widgets.
6. [The app shell](06-app-shell.md): menus, toolbars, split views, sheets,
   panels, Settings, restoration and undo.
7. [Live data](07-live-data.md): per-frame updates and growing tables.
8. [Testing and shipping](08-testing-and-shipping.md): self-checks,
   accessibility audits, packaging and notarisation.
9. [Styling](09-styling.md): text styles, colours, cards, shadows, symbols
   and pictures.
10. [Materials and windows](10-materials-and-windows.md): sidebar and panel
    materials, Liquid Glass, layered views, and title bar and toolbar styles.
11. [Motion](11-motion.md): animated changes, transitions, hover and click,
    and Reduce Motion.
12. [More controls and charts](12-more-controls-and-charts.md): tabs, radio
    groups, links, gauges, token fields, path controls, tooltips, and bar,
    stacked, line, area, scatter and pie charts with formats.
13. [Web, PDF, video and services](13-web-pdf-video-and-services.md): web
    pages, PDFs, video and sound, the Share menu, printing, notifications
    and menu-bar extras.
14. [Custom native views](14-custom-native-views.md): any `NSView` as a
    Ferrokit view, from MapKit's map to a view your app draws itself.

The example apps are the long form of these chapters. They're in
Ferrokit's repository (and in the published crate's `examples` folder):

| Example | Shows |
| --- | --- |
| `settings` | a settings form: grid layout, validation, background work |
| `styles` | a styled screen: type hierarchy, cards, symbols, pictures |
| `materials` | materials, Liquid Glass over a picture, full-size windows |
| `motion` | animated layout, fades and slides, hover highlights, clickable rows |
| `components` | tabs, radio groups, tokens, ratings, gauges, path control, every chart |
| `habits` | a habit tracker: cards, a progress ring, a history calendar, insights charts, a sheet, reminders, a menu-bar panel, a printed report |
| `media` | a web page, a printed report in a PDF view, video, Share, Print, notifications, a menu-bar extra |
| `native_csv` | a 200,000-row CSV browser: table, sidebar, search, sheet, notes |
| `log_viewer` | a log tail at display rate: `on_frame`, `append`, filtering |
| `hands_on` | drag and drop, the clipboard, panels, undo, input methods |
| `native_views` | `MKMapView` and a view the app draws itself, wrapped with `native` |
| `ui_checklist` | the framework's regression suite |

Run one from a clone of the repository:

```bash
git clone https://github.com/lhassa8/ferrokit && cd ferrokit
cargo run --release -p ferrokit --example settings
```
