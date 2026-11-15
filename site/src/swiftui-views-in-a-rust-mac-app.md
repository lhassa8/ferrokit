# SwiftUI views in a Rust Mac app (Swift Charts)

Some of Apple's views exist only in SwiftUI: Swift Charts, `Gauge`'s styles, MapKit's SwiftUI `Map`, and many third-party packages. A Ferrokit app can still show them. SwiftUI's `NSHostingView` is an `NSView`, so [`native(..)`](wrap-any-nsview.md) wraps it like any other. You write the SwiftUI view in Swift, and the [`ferrokit-build`](https://docs.rs/ferrokit-build) crate compiles it into your app from `build.rs`.

**1. Add the build helper and the objc2 crates to `Cargo.toml`:**

```toml
[dependencies]
ferrokit = "0.9"
objc2 = "0.6"
objc2-app-kit = { version = "0.3", features = ["NSResponder", "NSView"] }

[build-dependencies]
ferrokit-build = "0.9"
```

```rust,ignore
// build.rs
fn main() {
    ferrokit_build::swift("swift").compile();
}
```

Every `.swift` file under `swift/` becomes one Swift module, linked into the app. The Swift runtime and the frameworks your Swift imports (SwiftUI, Charts…) come with macOS, so nothing is copied into the app.

**2. Write the view in Swift,** and export C functions: one that returns the hosting view, retained, and one per value the app sets.

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

**3. Wrap it in Rust** and set its values from a signal:

```rust,ignore
// Not compiled on this site: it needs the Swift file above to link.
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

When `sales` changes, the `update` runs again and Swift Charts redraws. The chart is laid out with the rest of the window, and SwiftUI's view is released when the Ferrokit view leaves the tree.

For values coming back from SwiftUI, such as a slider, pass a C callback and a context pointer (a boxed signal, freed in `.on_remove`) to the Swift function.

## Things to know

- **Toolchain:** `swiftc` comes with the Command Line Tools (`xcode-select --install`), and that's enough for Swift like the example above. Since the macOS 27 SDK, SwiftUI's `@State`, `@Entry` and `#Preview` are macros, and only Xcode has the plugin that expands them. When an Xcode is in `/Applications`, `ferrokit-build` uses it for them automatically, with a warning; without one, it stops the build and says to install Xcode. No Xcode project is needed either way.
- **Builds:** `ferrokit-build` follows the target architecture (Apple silicon, Intel and universal builds), the Cargo profile, and `MACOSX_DEPLOYMENT_TARGET` (15.0 by default).
- **Accessibility:** SwiftUI's views are in the window's accessibility tree, for VoiceOver and for Ferrokit's `testing::ax_elements` and `ax_audit`, which find a SwiftUI control without a name as they do Ferrokit's own.
- **Size:** the view's ideal size is the hosting view's `fittingSize`, which `native` already uses; by default it fills the room it gets.
- **The glue is by hand for now.** A smaller bridge, with SwiftUI views registered by name and `.prop`, `.binding` and `.on` on the Rust side, is being designed in [issue #7](https://github.com/lhassa8/ferrokit/issues/7): say there what you'd use it for.

More in the guide: [SwiftUI views](guide/14-custom-native-views.md#swiftui-views), and `ferrokit-build`'s options (module name, minimum macOS, Swift 6 mode) in [its README](https://github.com/lhassa8/ferrokit/tree/main/crates/ferrokit-build).
