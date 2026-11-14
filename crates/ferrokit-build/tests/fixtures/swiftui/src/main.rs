unsafe extern "C" {
    fn chart_width() -> f64;
    fn task_value() -> isize;
}

fn main() {
    // SAFETY: called on the main thread; functions without arguments.
    let (w, t) = unsafe { (chart_width(), task_value()) };
    println!("chart {w} task {t}");
}
