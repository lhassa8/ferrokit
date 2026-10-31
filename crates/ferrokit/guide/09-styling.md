# Styling

Ferrokit's views are AppKit's own, so they already look like a Mac app.
Styling adds the rest of what makes an app look designed: a hierarchy of
type, colour, cards, and imagery. Everything here follows Dark Mode,
Increase Contrast and the user's accent colour by itself.

The `styles` example builds a whole screen this way:

```bash
cargo run -p ferrokit --example styles
```

## Text

`text` takes a font and a colour, each fixed or from signals.
`.font(..)` and `.color(..)` also work on any view (a row, a card) and
after other modifiers, for all the text inside:

```rust,no_run
use ferrokit::*;

let total = Signal::new(48_210.0);
let header = vstack((
    text("Good morning").font(Font::subheadline()).color(Color::Secondary),
    text("Your money this month").font(Font::large_title().weight(Weight::Bold)),
    text(move || format!("${}", format_number(total.get(), 0)))
        .font(Font::title().monospaced_digits())
        .color(move || if total.get() < 0.0 { Color::Red } else { Color::Primary }),
))
.alignment(Align::Start);
# let _ = header;
```

**The system's text styles**, sized as the Mac sizes them:

| Style | Size | For |
| --- | --- | --- |
| `Font::large_title()` | 26 pt | a screen's main title |
| `Font::title()`, `title2()`, `title3()` | 22, 17, 15 pt | section titles |
| `Font::headline()` | 13 pt bold | headings in lists |
| `Font::body()` | 13 pt | what labels use |
| `Font::callout()`, `subheadline()` | 12, 11 pt | text set apart |
| `Font::footnote()`, `caption()`, `caption2()` | 10 pt | small print |

**Custom fonts:**
- `Font::system(size)`, `Font::monospaced(size)` (SF Mono);
- `Font::named("Avenir Next", 15.0)` (the system font if it isn't installed).

**Font changes:**
- `.weight(Weight::Semibold)`, `.bold()`;
- `.size(points)`, `.italic()`;
- `.monospaced_digits()`, so changing numbers don't shift.

**Colours:**
- **Text:** `Color::Primary`, `Secondary`, `Tertiary` and `Quaternary`, the system's levels of emphasis.
- **Accent:** `Color::Accent`, the user's accent colour.
- **System colours:** `Red`, `Blue`, `Teal` and the rest.
- **Fixed colours:** `Color::rgb(r, g, b)`, `Color::rgba(..)` and `Color::hex("#3a7bd5")`. They stay the same in light and dark.
- **Opacity:** `.opacity(0.5)` gives a colour at half its own opacity.

## Cards and surfaces

Any view can have a background, rounded corners, a border and a shadow:

```rust,no_run
use ferrokit::*;

let card = vstack((
    text("Balance").font(Font::subheadline()).color(Color::Secondary),
    text("$48,210").font(Font::title().weight(Weight::Semibold)),
))
.alignment(Align::Start)
.padding(16.0)
.fill_width()
.background(Color::ControlBackground)
.corner_radius(12.0)
.border(Color::Separator, 0.5)
.shadow(Shadow::soft());
# let _ = card;
```

- **Colours:** `Color::QuaternaryFill` is the system's fill for grouped content: cards as SwiftUI's `GroupBox` and grouped forms draw them, a translucent grey that suits light and dark. `Fill`, `SecondaryFill` and `TertiaryFill` are stronger, for smaller shapes (a ring's track, a field). `Color::ControlBackground` is the colour for panels and lists, and `Color::WindowBackground` the window's.
- **Shadows:** `Shadow::soft()` suits cards on a background, `Shadow::floating()` something above the content, and `Shadow::new(radius, y)` sets your own.
- **Opacity:** `.opacity(0.4)` fades a whole view.

**Modifiers apply in the order written**, as in SwiftUI:
- **Padding then background:** `.padding(16.0).background(..)` puts the background under the padding.
- **Background then padding:** `.background(..).padding(16.0)` keeps it to the view.
- **Frame then padding:** `.frame(18.0, 18.0).padding(7.0)` is an 18-point view in a 32-point tile.

**Rows of cards** that all `.fill_width()` share a stack's width equally, whatever their content.

## Images

**SF Symbols:**

```rust,no_run
use ferrokit::*;

let icons = hstack((
    image("cart.fill").symbol_size(15.0).symbol_weight(Weight::Semibold).hierarchical(Color::Orange),
    image("cloud.sun.fill").multicolor(),
    image("bag.fill").color(Color::rgb(1.0, 1.0, 1.0)).frame(18.0, 18.0).padding(7.0).background(Color::Pink).corner_radius(8.0),
));
# let _ = icons;
```

**Symbol styling:**
- `.symbol_size(points)` and `.symbol_weight(..)`, to match the text beside it;
- `.color(..)` for one colour;
- `.hierarchical(..)` for shades of one colour;
- `.palette(&[..])` for a colour per layer;
- `.multicolor()` for the symbol's own colours.

**Pictures,** from a file or from bytes built into the app:

```rust,no_run
use ferrokit::*;

let avatar = image_file("avatar.jpg").fill().corner_radius(24.0).description("Ada's photo").frame(48.0, 48.0);
let logo = image_data(include_bytes!("../README.md")).frame(120.0, 40.0);
# let _ = (avatar, logo);
```

- **Fit:** pictures fit inside their frame by default, all of it shown.
- **Fill:** `.fill()` fills the frame and crops the edges, for covers and avatars.
- **Rounding:** an image's own `.corner_radius(..)` rounds the picture itself; use half its size for a circle.
- **VoiceOver:** give pictures that carry meaning a `.description(..)`. Without one, VoiceOver skips the image as decorative.

An image's own methods (`.fill()`, `.corner_radius(..)`, `.description(..)`, the symbol ones) come before the modifiers every view has (`.frame(..)`, `.padding(..)`).
