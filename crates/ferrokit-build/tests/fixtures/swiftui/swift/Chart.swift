// SwiftUI and Swift Charts, linked without naming them in build.rs.
import AppKit
import Charts
import SwiftUI

struct Bars: View {
    var body: some View {
        Chart([3.0, 5.0, 2.0].enumerated().map { $0 }, id: \.offset) {
            BarMark(x: .value("Day", $0.offset), y: .value("Value", $0.element))
        }
        .frame(width: 200, height: 100)
    }
}

/// The hosting view's fitting width, made on the main thread.
@MainActor
@_cdecl("chart_width")
public func chartWidth() -> Double {
    _ = NSApplication.shared
    return NSHostingView(rootView: Bars()).fittingSize.width
}

/// Something from Swift Concurrency (linked by @rpath).
@_cdecl("task_value")
public func taskValue() -> Int {
    let s = DispatchSemaphore(value: 0)
    nonisolated(unsafe) var v = 0
    Task.detached { v = await compute(); s.signal() }
    s.wait()
    return v
}

func compute() async -> Int { 42 }
