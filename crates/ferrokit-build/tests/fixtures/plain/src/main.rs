unsafe extern "C" {
    fn greeting_length(name: *const std::ffi::c_char) -> isize;
    fn module_name_length() -> isize;
}

fn main() {
    // SAFETY: a NUL-terminated string; a function without arguments.
    let (n, m) = unsafe { (greeting_length(c"Ferrokit".as_ptr()), module_name_length()) };
    println!("greeting {n} module {m}");
}
