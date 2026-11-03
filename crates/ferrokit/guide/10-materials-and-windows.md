# Materials and windows

Mac apps feel native partly through what's behind their content: the
translucent sidebar that runs up behind the window buttons, glass over
photos, a title bar that blends into the window. This chapter covers those,
and layering one view over another.

The `materials` example builds a travel journal this way:

```bash
git clone https://github.com/lhassa8/ferrokit && cd ferrokit
cargo run -p ferrokit --example materials
```

## Materials

`.material(..)` puts one of the system's materials behind a view, its
padding included. Materials blur what's behind them, and they follow Dark
Mode, Reduce Transparency and whether the window is active, as the system's
own sidebars and panels do:

```rust,no_run
use ferrokit::*;

let sidebar = vstack((
    text("Places").font(Font::subheadline().weight(Weight::Semibold)).color(Color::Secondary),
    text("Lofoten"),
    text("Kyoto"),
    spacer(),
))
.alignment(Align::Start)
.padding(10.0)
.material(Material::Sidebar);
# let _ = sidebar;
```

| Material | For |
| --- | --- |
| `Material::Sidebar` | sidebars and source lists |
| `Material::Header`, `Titlebar` | headers over scrolling content, a title bar area |
| `Material::Popover`, `Menu`, `Sheet`, `ToolTip` | panels like the system's own |
| `Material::Hud` | dark floating panels (a player, an inspector over media) |
| `Material::Window`, `Content`, `UnderWindow`, `UnderPage` | backgrounds |
| `Material::Selection` | a selection highlight in the accent colour |

Sidebars, panels and backgrounds show the desktop through the window;
headers, title bars and content materials blend with the window's own
content. `.corner_radius(..)` rounds a material.

## Liquid Glass

`.glass(..)` puts Liquid Glass behind a view on macOS 26 and later. On
macOS 15, which has no Liquid Glass, a translucent popover material takes
its place, so the same code runs on both:

```rust,no_run
use ferrokit::*;

let caption = vstack((
    text("Lofoten").font(Font::title().weight(Weight::Bold)),
    text("Nine days above the Arctic Circle").font(Font::callout()).color(Color::Secondary),
))
.alignment(Align::Start)
.padding(14.0)
.glass(Glass::Regular)
.corner_radius(14.0);
# let _ = caption;
```

- **`Glass::Regular`:** frosted glass, for controls and panels.
- **`Glass::Clear`:** clearer glass, over rich backgrounds like photos.
- **`Glass::Tinted(color)`:** regular glass with a tint; give the colour
  some transparency (`Color::Blue.opacity(0.3)`).

`has_liquid_glass()` says whether this Mac has it, for the rare design that
should differ.

## Layers

`zstack` puts children on top of one another, the first at the back. It's
as large as its largest child. Children that fill take its whole size, and
the others keep their own size and are centred, or placed by
`.alignment(horizontal, vertical)`:

```rust,no_run
use ferrokit::*;

let hero = zstack((
    image_file("cover.jpg").fill().corner_radius(14.0).fill_width().fill_height(),
    text("Lofoten").font(Font::title()).padding(14.0).glass(Glass::Regular).corner_radius(14.0).padding(16.0),
))
.alignment(Align::Start, Align::End)
.height(280.0);
# let _ = hero;
```

Here the picture fills the stack, and the caption sits 16 points in from
its bottom-left corner. Glass and materials blur the views behind them in
the stack, so glass over a picture refracts the picture.

## Window chrome

A [`Window`](crate::Window) chooses how its title bar and toolbar look:

```rust,no_run
use ferrokit::*;

let window = Window::new("Journal")
    .full_size_content()
    .hide_title()
    .toolbar(vec![toolbar_space(), toolbar_button("share", "Share", "square.and.arrow.up", || {})])
    .view(split((text("Sidebar").material(Material::Sidebar), text("Content"))).position(210.0));
# let _ = window;
```

- **`.full_size_content()`:** content goes under the title bar and toolbar,
  which become transparent. Backgrounds and materials, and a split view's
  panes, reach the window's top edge, so a sidebar's material runs up
  behind the window buttons, as in Apple's apps. The content itself starts
  below the bar, with no padding to work out.
- **`.transparent_title_bar()`:** the title bar shows the window's
  background, so bar and content read as one surface.
- **`.hide_title()`:** no title in the bar, for windows whose content says
  what they are. The title still names the window in the Window menu and
  Mission Control.
- **`.toolbar_style(..)`:** `ToolbarStyle::Unified` (the default),
  `UnifiedCompact` for a shorter bar, `Expanded` with the title above the
  items, or `Preference` for Settings-style tabs.
- **`.background(color)`:** the window's own colour. Named colours follow
  Dark Mode; fixed ones (`Color::hex("#e9dcc4")`) stay as they are, so give
  a fixed background text colours that suit it.

## Panels

A panel is a secondary window serving the main ones: an inspector, a
palette, a heads-up display over media. `Window::panel` makes one:

```rust,no_run
use ferrokit::*;

let inspector = Window::new("Inspector").size(260.0, 320.0).panel(PanelStyle::Floating).view(text("Selection"));
let tools = Window::new("Tools").size(180.0, 240.0).panel(PanelStyle::Utility).view(text("Brushes"));
let hud = Window::new("Adjust").size(240.0, 120.0).panel(PanelStyle::Hud).view(text("Exposure"));
# let _ = (inspector, tools, hud);
```

Panels float above the app's other windows, hide while the app is in the
background, stay out of the Window menu's list, and take the keyboard only
when a field in them is clicked, so typing still goes to the main window.
`Utility` has a slimmer title bar; `Hud` is dark and translucent.
