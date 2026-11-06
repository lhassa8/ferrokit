# Custom native views

Ferrokit wraps a lot of AppKit, but not all of it, and not the frameworks
beside it: MapKit's map, SceneKit's 3D view, a third-party chart, or a view
your app draws itself. [`native`](crate::native) wraps any `NSView` as a
Ferrokit view. It takes part in layout, identity, signals, focus and
teardown like the built-in ones. The `native_views` example wraps
`MKMapView` and a clock dial the app draws:

```bash
git clone https://github.com/lhassa8/ferrokit && cd ferrokit
cargo run -p ferrokit --example native_views
```

You'll use the `objc2` crates directly here (`objc2`, `objc2-app-kit`, and
the framework's own crate, such as `objc2-map-kit`), with the features for
the classes you call.

## Wrapping a view

`native(make)` calls `make(mtm)` once, when the view is built, and shows the
view it returns. `update(f)` changes it in place: `f` runs as an effect, now
and again whenever a signal it read changes, as text and controls follow
their signals.

```rust,no_run
use ferrokit::*;
use objc2_app_kit::NSLevelIndicator;

let level = Signal::new(3.0);
let meter = native(|mtm| {
    let m = NSLevelIndicator::new(mtm);
    m.setMaxValue(5.0);
    m
})
.update(move |m| m.setDoubleValue(level.get()))
.measure(|_| Size { w: 120.0, h: 18.0 })
.flexible(false, false)
.accessibility_label("Signal strength");

let view = vstack((meter, slider(&level, 0.0, 5.0)));
# let _ = view;
```

Make the view in `make`, not before: the closure runs on the main thread
when the window is built, and the view belongs to that one place in the
tree. Don't rebuild the view to change it. Each `update` is its own effect,
so give unrelated properties their own `update`s and a change to one
doesn't re-apply the others.

## Size and layout

A wrapped view is a leaf in Ferrokit's layout, placed by setting its frame:

- **Its ideal size** is `measure`'s answer, asked when it's built and again
  after each `update`, so a change that resizes it moves its neighbours.
  Without `measure` it's the view's `fittingSize`, which is right for views
  with an intrinsic size (AppKit's controls) and zero for a plain `NSView`.
- **It stretches** across and down by default (a map or a drawing fills
  the room it's given). `.flexible(false, false)` keeps it at its measured
  size, like a control.
- **The usual modifiers** apply on top: `.width`, `.height`, `.min_width`,
  `.min_height`, `.fill_width`, `.padding`, `.hidden`, `.opacity`,
  backgrounds and borders.

The layout doesn't offer a width and ask for a height, so a view whose
height depends on its width (wrapping text of its own) needs a fixed width
or a `measure` that assumes one.

## Delegates and teardown

AppKit holds delegates and targets weakly, so something has to keep them.
`.keep(object)` keeps an object as long as the view. Make it when you
describe the view, set it in `make`, and keep it:

```rust,no_run
use ferrokit::*;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSTextField, NSTextFieldDelegate, NSControlTextEditingDelegate};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GuideEdits"]
    struct Edits;

    unsafe impl NSObjectProtocol for Edits {}
    unsafe impl NSControlTextEditingDelegate for Edits {}
    unsafe impl NSTextFieldDelegate for Edits {
        #[unsafe(method(controlTextDidEndEditing:))]
        fn ended(&self, _note: &NSNotification) {
            println!("editing ended");
        }
    }
);

let mtm = MainThreadMarker::new().unwrap();
// SAFETY: NSObject's designated initialiser.
let edits: Retained<Edits> = unsafe { msg_send![super(Edits::alloc(mtm).set_ivars(())), init] };
let field = native({
    let edits = edits.clone();
    move |mtm| {
        let f = NSTextField::new(mtm);
        // SAFETY: the delegate is kept as long as the field (below).
        unsafe { f.setDelegate(Some(ProtocolObject::from_ref(&*edits))) };
        f
    }
})
.keep(edits)
.on_remove(|f| unsafe { f.setDelegate(None) })
.flexible(true, false);
# let _ = field;
```

When the view leaves the tree (a `dynamic` switches it out, a `for_each`
row goes, its window closes), its updates stop, it leaves the window,
`on_remove` runs, and then the view, its kept objects and its closures are
released. Use `on_remove` for what AppKit won't undo for you: a timer, a
notification observer, key-value observation, a delegate that might call
back late.

## Drawing your own view

A view of your own is an `NSView` subclass made with `define_class!`. It
keeps its state in its ivars, draws in `drawRect:`, and gets its state from
signals through `update`. Ferrokit marks the view for display after each
update, so `drawRect:` runs again with the new state:

```rust,no_run
use std::cell::Cell;
use ferrokit::*;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBezierPath, NSColor, NSView};
use objc2_foundation::NSRect;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GuideBar"]
    #[ivars = Cell<f64>]
    struct Bar;

    impl Bar {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let mut b = self.bounds();
            b.size.width *= self.ivars().get();
            NSColor::controlAccentColor().setFill();
            NSBezierPath::fillRect(b);
        }
    }
);

impl Bar {
    fn new(mtm: MainThreadMarker) -> Retained<Bar> {
        let this = Bar::alloc(mtm).set_ivars(Cell::new(0.0));
        // SAFETY: NSView's designated initialiser.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }
}

let done = Signal::new(0.4);
let bar = native(Bar::new)
    .update(move |bar| bar.ivars().set(done.get()))
    .height(8.0)
    .accessibility_label("Progress");
# let _ = bar;
```

For anything drawn from data, Ferrokit's own [`canvas`](crate::canvas) may
be enough. A subclass is for drawing that needs AppKit's whole view API:
its own mouse tracking, a layer, or text input.

## Focus and accessibility

Ferrokit's windows use AppKit's key view loop, so Tab reaches a wrapped
view that accepts the focus (`acceptsFirstResponder`, or AppKit's controls
on their own) in the order its frame puts it.

Accessibility is the view's own. AppKit's views and the system frameworks'
views have theirs. A view of your own should say what it is: override
`accessibilityRole` (and `isAccessibilityElement`) in the subclass, and
give it a name with `.accessibility_label(..)` and, for a value,
`.accessibility_value(..)`. `testing::ax_audit` finds a wrapped view without
a name, as it does Ferrokit's own. It also reports a system view's own parts
that come without names: MapKit's zoom buttons, for one. That's Apple's to
fix, and the `native_views` example's check leaves out what's inside the
map.

## What `native` doesn't do

- **Implement `View` yourself.** `Cx` and `Node` stay internal; `native`
  covers any `NSView`.
- **View controllers.** Wrap the controller's view
  (`native(move |_| controller.view())`) and keep the controller with
  `.keep(controller)`. It isn't added as a child of the window's controller.
- **Ferrokit views inside a wrapped view.** A wrapped view is a leaf: its
  subviews are its own.
- **SwiftUI.** `NSHostingView` needs Swift.
