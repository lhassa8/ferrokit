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
- **SwiftUI** needs Swift: see the next section.

## SwiftUI views

Some of Apple's views exist only in SwiftUI: Swift Charts, `Gauge`'s
styles, MapKit's SwiftUI `Map`, and many third-party packages. An
`NSHostingView` is an `NSView`, so `native` wraps it. The SwiftUI view is
written in Swift, and [`ferrokit-build`](https://docs.rs/ferrokit-build)
compiles it into the app from `build.rs`:

```toml
# Cargo.toml
[build-dependencies]
ferrokit-build = "0.11"
```

```rust,ignore
// build.rs
fn main() {
    ferrokit_build::swift("swift").compile();
}
```

The Swift side makes the view and exports C functions: one that returns
the hosting view, retained, and one per value the app sets:

```swift
// swift/Sales.swift
import Charts
import SwiftUI

final class SalesModel: ObservableObject {
    @Published var values: [Double] = []
}

struct SalesChart: View {
    @ObservedObject var model: SalesModel
    var body: some View {
        Chart(Array(model.values.enumerated()), id: \.offset) {
            BarMark(x: .value("Month", $0.offset), y: .value("Sales", $0.element))
        }
        .padding()
    }
}

@MainActor @_cdecl("sales_chart_new")
public func salesChartNew() -> UnsafeMutableRawPointer {
    let view = NSHostingView(rootView: SalesChart(model: SalesModel()))
    return Unmanaged.passRetained(view).toOpaque()
}

@MainActor @_cdecl("sales_chart_set")
public func salesChartSet(_ view: UnsafeMutableRawPointer, _ values: UnsafePointer<Double>, _ count: Int) {
    let host = Unmanaged<NSHostingView<SalesChart>>.fromOpaque(view).takeUnretainedValue()
    host.rootView.model.values = Array(UnsafeBufferPointer(start: values, count: count))
}
```

The Rust side wraps the hosting view and sets its values from a signal:

```rust,ignore
use ferrokit::*;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use std::ffi::c_void;

unsafe extern "C" {
    fn sales_chart_new() -> *mut c_void;
    fn sales_chart_set(view: *mut c_void, values: *const f64, count: isize);
}

let sales = Signal::new(vec![12.0, 30.0, 22.0, 48.0]);
let chart = native(|_| {
    // SAFETY: a +1 NSHostingView, owned from here on.
    unsafe { Retained::from_raw(sales_chart_new().cast::<NSView>()) }.expect("a view")
})
.update(move |view| {
    let v: *mut c_void = (view as *const NSView).cast_mut().cast();
    // SAFETY: the chart's own view; a buffer of `len` values.
    sales.with(|s| unsafe { sales_chart_set(v, s.as_ptr(), s.len() as isize) })
});
```

(These two blocks are `ignore`d here: they need the Swift file to link.)

For values coming back, such as a SwiftUI slider, pass a C callback and a
context pointer (a boxed signal, freed in `.on_remove`) to the Swift
function. SwiftUI's view is released when the Ferrokit view leaves the tree.
Its ideal size is the hosting view's `fittingSize`, which `native` already
uses; by default it fills the room it gets, like a map.

Things to know:

- **Swift runtime:** it comes with macOS, so nothing is added to the app.
- **Toolchain:** `swiftc` comes with the Command Line Tools. Since the
  macOS 27 SDK, SwiftUI's `@State`, `@Entry` and `#Preview` are macros,
  and only Xcode has the plugin that expands them. `ferrokit-build` uses an
  Xcode in `/Applications` for them, with a warning; without one, it stops
  the build and says to install Xcode. Swift without those macros (like
  the example above, which uses `ObservableObject`) needs only the Command
  Line Tools.
- **Accessibility:** SwiftUI's views are in the window's accessibility tree,
  for VoiceOver and for `testing::ax_elements` and `ax_audit`, which find a
  SwiftUI control without a name as they do Ferrokit's own.
- **The glue is by hand for now.** A smaller bridge, with named SwiftUI
  views and `.prop`, `.binding` and `.on`, is being designed in
  [issue #7](https://github.com/lhassa8/ferrokit/issues/7).
