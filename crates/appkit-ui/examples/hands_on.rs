//! The developer's hands-on checks in one window: mouse drag and drop, the
//! real clipboard, Open and Save panels, Japanese input, undo, live resize
//! and VoiceOver. Every result is shown in the window and printed as an
//! `[event]` line.
//! `cargo run --release -p appkit-ui --example hands_on`

use std::rc::Rc;

use appkit_ui::*;

#[derive(Clone, Debug)]
struct Row {
    name: String,
    n: u32,
}

fn rows() -> Vec<Row> {
    (1..=8).map(|n| Row { name: format!("Row {n}"), n }).collect()
}

/// Shows `what` in the window (the last five) and prints it.
fn record(log: &Signal<Vec<String>>, what: String) {
    eprintln!("[event] {what}");
    log.update(|l| {
        l.push(what);
        let excess = l.len().saturating_sub(5);
        l.drain(..excess);
    });
}

fn main() {
    let model = Rc::new(TableModel::new(
        ViewId(1),
        vec![
            Column::text("name", "Name", |r: &Row| r.name.clone()).width(160.0).sortable(false),
            Column::number("n", "Number", |r: &Row| r.n as f64).width(90.0).sortable(false),
        ],
    ));
    model.set_rows(rows());
    let log = Signal::new(vec!["Ready.".to_string()]);
    let name = Signal::new(String::new());
    let notes = TextModel::new(ViewId(2), "Click here and type. Try Japanese input: toukyou → 東京.\n");

    let (m1, l1, l2, l3, l4, l5, l6, l7) = (model.clone(), log.clone(), log.clone(), log.clone(), log.clone(), log.clone(), log.clone(), log.clone());
    let m2 = model.clone();
    let n1 = name.clone();
    let content = vstack((
        text("1–3: the table. Drag rows to reorder, drag a .csv or .txt file from the Finder onto it, drag a row into TextEdit, ⌘C a selection, ⌘V text."),
        table(&model)
            .row_drag()
            .reorderable(move |picked, before| {
                let names = |r: &[usize]| r.iter().map(|&i| m1.rows()[i].name.clone()).collect::<Vec<_>>();
                let moved = names(&picked);
                m1.move_rows(&picked, before);
                record(&l1, format!("moved {moved:?} (before row {before:?}); now {:?}", m1.rows().iter().map(|r| r.n).collect::<Vec<_>>()));
            })
            .on_files_dropped(&["csv", "txt"], move |paths, before| {
                let names: Vec<String> = paths.iter().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into()).collect();
                record(&l2, format!("files dropped {names:?} before row {before:?}"));
            })
            .on_paste(move |t| {
                let lines = t.lines().count();
                record(&l3, format!("pasted {lines} line(s): {:?}", t.chars().take(60).collect::<String>()));
            }),
        hstack((
            button("Open…", move || {
                let l = l4.clone();
                open_panel(&windows()[0], OpenOptions { extensions: vec![], multiple: false, directories: false }, move |p| {
                    record(&l, format!("Open panel → {p:?}"));
                })
            }),
            button("Save…", move || {
                let l = l5.clone();
                save_panel(&windows()[0], "Untitled.csv", None, move |p| record(&l, format!("Save panel → {p:?}")))
            }),
            button("Reset rows", move || {
                m2.set_rows(rows());
                record(&l6, "rows reset".into());
            }),
            spacer(),
        )),
        text("4: a field and an editor. Type (also Japanese), then ⌘Z and ⇧⌘Z."),
        hstack((text("Name:"), text_field(&name))),
        label(move || format!("The field's value: {:?}", n1.get())),
        text_editor(&notes),
        label(move || l7.get().join("  ·  ")),
    ))
    .padding(12.0);
    App::new("Hands-on").window(Window::new("Hands-on").size(820.0, 640.0).view(content)).run();
}
