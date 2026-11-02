# Motion

Good Mac apps move a little. A panel fades in, the list below it slides
down to make room, and a finished row fades back. A row highlights under the
pointer and dims as it's clicked. Ferrokit animates these with Core
Animation, as AppKit's own views do, and respects Reduce Motion by itself.

The `motion` example is a packing list built this way:

```bash
cargo run -p ferrokit --example motion
```

## Animated changes

Wrap a change in `animated`, and whatever it changes moves instead of
jumping:

```rust,no_run
use ferrokit::*;

let details = Signal::new(false);
let toggle = button("Details", move || animated(|| details.set(!details.get())));
# let _ = toggle;
```

Inside `animated`:
- **Views that move or resize** because of the change slide to their new
  place. That covers the list pushed down by a panel that appears.
- **Opacity and colours** set by `.opacity(..)` and
  `.background(..)` fade to their new values.
- **Views with a transition** that are added or removed come and go as it
  says.

Outside `animated`, the same change lands at once. The default is a quarter
of a second, easing in and out. `animated_with(Animation::ease_out(0.4), ..)`
sets your own timing; there's also `ease_in`, `ease_in_out` and `linear`.

## Opacity and colours that follow state

```rust,no_run
use ferrokit::*;

let done = Signal::new(false);
let row = hstack((text("Boots"), spacer()))
    .padding(10.0)
    .background(move || if done.get() { Color::Green.opacity(0.18) } else { Color::Clear })
    .corner_radius(8.0)
    .opacity(move || if done.get() { 0.45 } else { 1.0 });
# let _ = row;
```

## Transitions

`.transition(..)` says how a view comes and goes when `dynamic` or
`for_each` adds or removes it inside `animated`:

- **`Transition::Fade`:** fades in and out.
- **`Transition::Slide(Edge::Leading)`:** fades while sliding a short way
  in from that edge (`Above`, `Below`, `Leading` or `Trailing`), and back
  out to it.

```rust,no_run
use ferrokit::*;

#[derive(Clone)]
struct Item { id: u32, name: String }
impl Identifiable for Item {
    type Id = u32;
    fn id(&self) -> u32 { self.id }
}

let items = Signal::new(vec![Item { id: 1, name: "Boots".into() }]);
let list = vstack((for_each(move || items.get(), |item: Item| {
    text(&item.name).padding(8.0).transition(Transition::Slide(Edge::Leading))
}),));
let add = button("Add", move || animated(|| items.update(|v| v.push(Item { id: 2, name: "Map".into() }))));
# let _ = (list, add);
```

A removed view stays on screen until its transition ends, then goes, and
the views around it close the gap as it leaves.

## Springs

`Animation::spring(response, damping)` moves like a spring, as SwiftUI's
do: `response` is how quickly (its period undamped, in seconds) and
`damping` how it settles, 1 without overshooting and less with a bounce.
`Animation::bouncy()`, `snappy()` and `smooth()` are ready-made ones. A
spring lasts until it's still, and a view moved again mid-flight goes on
from where it is.

```rust,no_run
use ferrokit::*;

let open = Signal::new(false);
let toggle = button("Details", move || animated_with(Animation::bouncy(), || open.set(!open.get())));
# let _ = toggle;
```

Core Animation's curves can't overshoot, so frames under a spring are set
a frame at a time instead; fades and colours in the same change ease out
alongside.

## Animating values and keyframes

`animate(&signal, to, animation)` takes an `f64` signal to `to` over time,
setting it each frame: anything that reads it follows (a canvas drawing,
an opacity, a size). `keyframes(&signal, frames)` takes it through several
values, each reached with its own timing, as SwiftUI's keyframe animator
does: a shake, a pulse, a count-up. `.repeating()` loops it until
`stop_animation(&signal)`.

```rust,no_run
use ferrokit::*;

let level = Signal::new(0.0);
let fill = button("Fill", move || animate(&level, 1.0, Animation::snappy()));

let nudge = Signal::new(0.0);
let shake = button("Shake", move || {
    keyframes(&nudge, Keyframes::new(0.0).to(8.0, 0.06).to(-6.0, 0.08).with(0.0, Animation::snappy()))
});
# let _ = (fill, shake);
```

## Matched geometry

`.matched_geometry("id")` pairs views across an animated change: when one
with that id goes and another with it comes (a thumbnail and its detail, a
marker under the selected tab), the new one starts where and as large as
the old one was, then moves to its place. The `motion` example's trail
name moves from the header into the details panel this way.

## Hover and click

Any view can respond to the pointer:

- **`.on_hover(|inside| ..)`** hears the pointer come and go.
- **`.hover_highlight(Color::Quaternary)`** shows a background while the
  pointer is over the view, fading in and out. It's the quiet highlight of
  the system's lists.
- **`.on_click(|| ..)`** makes the whole view a button. It dims while
  pressed, and VoiceOver reads it as a button named by its text (or by
  `.accessibility_label(..)`). Use it for rows, cards and tiles; controls
  inside it don't get clicks.

```rust,no_run
use ferrokit::*;

let selected = Signal::new(0);
let row = hstack((image("map.fill"), text("Lofoten"), spacer()))
    .padding(8.0)
    .hover_highlight(Color::Quaternary)
    .corner_radius(6.0)
    .on_click(move || animated(|| selected.set(1)));
# let _ = row;
```

## Reduce Motion

Springs and matched geometry don't move either, and `animate` and
`keyframes` set the end value at once.

With Reduce Motion on (System Settings › Accessibility › Display),
animated changes don't move anything: views land in their new places at
once, and slides become plain fades. Fades and colour changes still
animate, as Apple's guidelines suggest. `reduce_motion()` says whether it's
on, for the rare animation of your own.
