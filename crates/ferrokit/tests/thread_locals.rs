//! Every thread-local in `ferrokit` and `ferrokit-core` stays reachable while
//! the process exits: it's a `UiCell` (no destructor), a `Cell` of plain
//! values or an atomic. A `RefCell` one is destroyed during
//! `std::process::exit`, and a destructor reaching it then aborts the app (it
//! happened twice).

use std::path::Path;

fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((path.display().to_string(), std::fs::read_to_string(&path).unwrap()));
        }
    }
}

#[test]
fn thread_locals_have_no_destructors() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    sources(&crates.join("ferrokit/src"), &mut files);
    sources(&crates.join("ferrokit-core/src"), &mut files);
    let mut wrong = Vec::new();
    let mut seen = 0;
    for (path, text) in &files {
        // Unit tests keep a `RefCell` on purpose, as an app's would be.
        let text = text.split("#[cfg(test)]").next().unwrap();
        for line in text.lines() {
            let line = line.trim().trim_start_matches("thread_local!(").trim_start_matches("pub(crate) ");
            // `static NAME: Type = …` (extern statics have no initialiser).
            let Some(decl) = line.strip_prefix("static ") else { continue };
            let Some((_, rest)) = decl.split_once(": ") else { continue };
            if !rest.contains(" = ") {
                continue;
            }
            seen += 1;
            let ok = ["UiCell<", "Cell<", "std::cell::Cell<", "Atomic"].iter().any(|t| rest.starts_with(t));
            if !ok {
                wrong.push(format!("{path}: {line}"));
            }
        }
    }
    assert!(seen > 80, "found only {seen} thread-locals: the scan is broken");
    assert!(wrong.is_empty(), "thread-locals with destructors (use ferrokit_core::kept::UiCell):\n{}", wrong.join("\n"));
}
