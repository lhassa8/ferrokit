//! {{display}}: a native Mac document app made with appkit-ui. Each window
//! is a file: New, Open…, Open Recent, Save, Save As…, Revert, the edited
//! dot, and a save prompt on close and quit come from the framework.
//!
//! `cargo ferrokit run` runs it as an app; `cargo ferrokit check` builds,
//! lints, tests and runs its self-checks (below, under `CHECK=all`);
//! `cargo ferrokit bundle` makes `{{display}}.app`, registered for the
//! document types in Cargo.toml.

use appkit_ui::*;

fn document() -> DocumentType<String> {
    DocumentType::new(&["txt", "md"], String::new, |doc: &Document<String>| {
        let model = TextModel::new(ViewId(1), &doc.content().get_untracked());
        model.set_font(Some("Menlo"), 13.0);
        let text = *doc.content();
        vstack((
            text_editor(&model).bind(doc.content()).syntax(Syntax::new().line_comment("#", TextStyle::new().color(TextColor::Gray))).accessibility_label("Text"),
            label(move || format!("{} words", text.with(|t| t.split_whitespace().count()))).padding(6.0),
        ))
    })
    .read(|path| std::fs::read_to_string(path))
    .write(|text, path| std::fs::write(path, text))
    .size(640.0, 480.0)
}

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    App::new("{{display}}")
        .documents(document())
        .on_launch(move || {
            if checks {
                self_checks::run();
            }
        })
        .run();
}

// ---- self-checks: `cargo ferrokit check` runs these (CHECK=all) ----

mod self_checks {
    use appkit_ui::testing::ax_audit;
    use appkit_ui::*;

    pub fn run() {
        let mut failed = 0;
        let mut check = |name: &str, pass: bool, note: String| {
            eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
            failed += u32::from(!pass);
        };
        let window = windows()[0].clone();
        // (t: "Untitled" in the user's language.)
        check("untitled-window", window.title().to_string() == t("Untitled"), format!("{:?}", window.title()));
        let problems = ax_audit(&window);
        check("accessibility", problems.is_empty(), format!("{problems:?}"));
        // Save and open a file through the document machinery.
        let path = std::env::temp_dir().join(format!("{}-check-{}.txt", env!("CARGO_PKG_NAME"), std::process::id()));
        std::fs::write(&path, "hello # a comment").unwrap();
        let opened = open_document(&path).map(|w| w.title().to_string());
        check("open-file", opened.as_deref().is_ok_and(|t| t.ends_with(".txt")), format!("{opened:?}"));
        let _ = std::fs::remove_file(&path);
        eprintln!("[check] done, {failed} failed");
        std::process::exit(if failed == 0 { 0 } else { 1 });
    }
}
