//! {{display}}: a native Mac app made with ferrokit.
//!
//! `cargo ferrokit run` runs it as an app; `cargo ferrokit check` builds,
//! lints, tests and runs its self-checks (below, under `CHECK=all`);
//! `cargo ferrokit bundle` makes `{{display}}.app`.

use ferrokit::*;

/// A to-do item.
#[derive(Clone, PartialEq)]
struct Task {
    id: u64,
    title: String,
    done: bool,
}

const ADD: Command = Command::new("add-task");

fn content(tasks: Signal<Vec<Task>>, draft: Signal<String>) -> impl View {
    let model = std::rc::Rc::new(TableModel::new(
        ViewId(1),
        vec![
            Column::check("done", "Done", |t: &Task| t.done).width(50.0),
            Column::text("title", "Task", |t: &Task| t.title.clone()).width(320.0),
        ],
    ));
    let m = model.clone();
    let add = move || {
        let title = draft.get_untracked();
        if title.trim().is_empty() {
            return;
        }
        tasks.update(|t| {
            let id = t.iter().map(|t| t.id).max().unwrap_or(0) + 1;
            t.push(Task { id, title: title.trim().into(), done: false });
        });
        draft.set(String::new());
    };
    vstack((
        hstack((
            text_field(&draft).placeholder("New task").on_submit(add).accessibility_label("New task"),
            button("Add", add),
        )),
        table(&model).on_toggle(move |row, _, done| tasks.update(|t| t[row].done = done)).accessibility_label("Tasks"),
        text(move || {
            let (all, done) = tasks.with(|t| (t.len(), t.iter().filter(|t| t.done).count()));
            format!("{done} of {all} done")
        }),
        watch(move || m.set_rows(tasks.get())),
    ))
    .padding(12.0)
    .on_command(ADD, add)
}

fn main() {
    let tasks = Signal::new(vec![
        Task { id: 1, title: "Read the guide".into(), done: false },
        Task { id: 2, title: "Make something".into(), done: false },
    ]);
    let draft = Signal::new(String::new());
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    App::new("{{display}}")
        .menu("Task", vec![menu_command("Add Task", ADD).key("t")])
        .window(Window::new("{{display}}").size(480.0, 360.0).remember_frame("main").view(content(tasks, draft)))
        .on_launch(move || {
            if checks {
                self_checks::run(tasks, draft);
            }
        })
        .run();
}

// ---- self-checks: `cargo ferrokit check` runs these (CHECK=all) ----

mod self_checks {
    use ferrokit::testing::{ax_audit, check, steps};
    use ferrokit::*;

    use super::{ADD, Task};

    /// Each step runs in a turn of its own: act in one, look in the next
    /// (views update after the step that changed a signal returns). After
    /// the last step the run ends and reports.
    pub fn run(tasks: Signal<Vec<Task>>, draft: Signal<String>) {
        let window = windows()[0].clone();
        let w = window.clone();
        steps(100, vec![
            Box::new(move || {
                let problems = ax_audit(&window);
                check("accessibility", problems.is_empty(), format!("{problems:?}"));
                draft.set("Ship it".into());
                perform_in(Some(&window), ADD);
            }),
            Box::new(move || {
                let titles: Vec<String> = tasks.with_untracked(|t| t.iter().map(|t| t.title.clone()).collect());
                check("add-task", titles.last().map(String::as_str) == Some("Ship it") && draft.get_untracked().is_empty(), format!("{titles:?}"));
                // The new row is on screen, not only in the signal.
                let shown = testing::ax_elements(&w).iter().any(|e| e.name.contains("Ship it"));
                check("add-task-shown", shown, "a view shows the new task");
            }),
        ]);
    }
}
