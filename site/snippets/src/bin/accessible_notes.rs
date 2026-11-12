use ferrokit::*;

fn main() {
    let title = Signal::new(String::new());
    let notes = TextModel::new(ViewId(1), "Spell-checked, with input methods, dictation and Find.\n");
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");

    App::new("Notes")
        .window(Window::new("Notes").size(520.0, 400.0).view(
            vstack((
                hstack((
                    text("Title:"),
                    // The text beside it is a separate view, so name the field.
                    text_field(&title).accessibility_label("Title"),
                )),
                // A real NSTextView: spell check, Japanese and Chinese input,
                // the emoji picker, undo, Edit ▸ Find, the Services menu.
                text_editor(&notes).accessibility_label("Notes"),
            ))
            .padding(12.0),
        ))
        .on_launch(move || {
            if checks {
                // What VoiceOver users would miss: unnamed controls.
                let problems = testing::ax_audit(&windows()[0]);
                testing::check("accessibility", problems.is_empty(), format!("{problems:?}"));
                testing::finish();
            }
        })
        .run();
}
