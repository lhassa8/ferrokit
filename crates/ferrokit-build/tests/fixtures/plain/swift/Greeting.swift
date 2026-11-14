@_cdecl("greeting_length")
public func greetingLength(_ name: UnsafePointer<CChar>) -> Int {
    greeting(String(cString: name)).count
}
