// In a subfolder, and called from Greeting.swift: one module.
func greeting(_ name: String) -> String { "Hello, \(name)!" }

@_cdecl("module_name_length")
public func moduleNameLength() -> Int {
    String(reflecting: Marker.self).split(separator: ".").first.map(\.count) ?? 0
}

struct Marker {}
