import AppKit
import Charts
import SwiftUI

/// A chart and a named slider: everything has a name.
struct Named: View {
    @State private var level = 40.0
    var body: some View {
        VStack(alignment: .leading) {
            Text("Visits (Swift Charts)").font(.headline)
            Chart([12.0, 30.0, 22.0, 48.0].enumerated().map { $0 }, id: \.offset) {
                BarMark(x: .value("Month", $0.offset), y: .value("Visits", $0.element))
            }
            Slider(value: $level, in: 0...100) { Text("Level") }
        }
        .padding()
    }
}

/// A slider with no label at all: the audit must find it.
struct Unnamed: View {
    @State private var level = 10.0
    var body: some View {
        Slider(value: $level, in: 0...100).labelsHidden().padding()
    }
}

@MainActor @_cdecl("named_panel")
public func namedPanel() -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(NSHostingView(rootView: Named())).toOpaque()
}

@MainActor @_cdecl("unnamed_panel")
public func unnamedPanel() -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(NSHostingView(rootView: Unnamed())).toOpaque()
}
