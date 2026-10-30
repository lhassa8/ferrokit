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

    let (m1, l1, l2, l3, l4, l5, l6, l7) = (model.clone(), log, log, log, log, log, log, log);
    let (m2, m3) = (model.clone(), model.clone());
    let n1 = name;
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
                // Each file's lines (after a CSV's header) become rows, inserted where dropped.
                let mut added = Vec::new();
                for p in &paths {
                    let text = std::fs::read_to_string(p).unwrap_or_default();
                    let skip = usize::from(p.extension().is_some_and(|e| e == "csv"));
                    added.extend(text.lines().skip(skip).filter(|l| !l.trim().is_empty()).map(|l| l.replace(',', "  ·  ")));
                }
                let mut rows = m3.rows().clone();
                let next = rows.iter().map(|r| r.n).max().unwrap_or(0);
                let at = before.unwrap_or(rows.len()).min(rows.len());
                let count = added.len();
                rows.splice(at..at, added.into_iter().enumerate().map(|(i, name)| Row { name, n: next + 1 + i as u32 }));
                m3.set_rows(rows);
                let names: Vec<String> = paths.iter().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into()).collect();
                record(&l2, format!("files dropped {names:?}: {count} row(s) inserted at row {}", at + 1));
            })
            .on_paste(move |t| {
                let lines = t.lines().count();
                record(&l3, format!("pasted {lines} line(s): {:?}", t.chars().take(60).collect::<String>()));
            }),
        hstack((
            button("Open…", move || {
                let l = l4;
                open_panel(&windows()[0], OpenOptions { extensions: vec![], multiple: false, directories: false }, move |p| {
                    record(&l, format!("Open panel → {p:?}"));
                })
            }),
            button("Save…", move || {
                let l = l5;
                save_panel(&windows()[0], "Untitled.csv", None, move |p| record(&l, format!("Save panel → {p:?}")))
            }),
            button("Reset rows", move || {
                m2.set_rows(rows());
                record(&l6, "rows reset".into());
            }),
            spacer(),
        )),
        text("4: a field and an editor. Type (also Japanese), then ⌘Z and ⇧⌘Z."),
        hstack((text("Name:"), text_field(&name).accessibility_label("Name"))),
        label(move || format!("The field's value: {:?}", n1.get())),
        text_editor(&notes).accessibility_label("Notes"),
        label(move || l7.get().join("  ·  ")),
    ))
    .padding(12.0);
    let m4 = model.clone();
    App::new("Hands-on")
        .window(Window::new("Hands-on").size(820.0, 640.0).view(content))
        .on_launch(move || {
            // CHECK=drop: drops a CSV through the table's own drop path, then exits.
            if std::env::var("CHECK").is_ok_and(|c| c == "drop") {
                let path = std::env::temp_dir().join("hands-on-drop.csv");
                std::fs::write(&path, "name,city,total\nAda,London,12.5\nGrace,Arlington,7\n").unwrap();
                let pb = appkit_host::testing::files_pasteboard(&[path.as_path()]);
                let taken = appkit_host::testing::drop_on_table(&m4.table_view(), &pb, 2, false);
                appkit_host::testing::release_pasteboard(&pb);
                // The table's events are handled on the next pass.
                let m4 = m4.clone();
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(100)), move |()| {
                    let names: Vec<String> = m4.rows().iter().map(|r| r.name.clone()).collect();
                    let ok = taken && names.len() == 10 && names[2].starts_with("Ada") && names[3].starts_with("Grace") && names[4] == "Row 3";
                    eprintln!("[check] drop {} taken {taken}; rows {names:?}", if ok { "PASS" } else { "FAIL" });
                    std::process::exit(if ok { 0 } else { 1 });
                });
            }
        })
        .run();
}
