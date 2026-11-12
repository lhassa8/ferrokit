use ferrokit::*;
use objc2_app_kit::NSLevelIndicator;

fn main() {
    let level = Signal::new(3.0);
    App::new("Meter")
        .window(Window::new("Meter").size(320.0, 120.0).view(
            vstack((
                // Any NSView, made once when the window is built…
                native(|mtm| {
                    let meter = NSLevelIndicator::new(mtm);
                    meter.setMaxValue(5.0);
                    meter
                })
                // …and changed in place whenever a signal it reads changes.
                .update(move |meter| meter.setDoubleValue(level.get()))
                .measure(|_| Size { w: 120.0, h: 18.0 })
                .flexible(false, false)
                .accessibility_label("Signal strength"),
                slider(&level, 0.0, 5.0).accessibility_label("Level"),
            ))
            .padding(20.0),
        ))
        .run();
}
