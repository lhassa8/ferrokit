# Views and layout

A view is a value that builds native views once (`View::build`). Layout is
done in Rust: stacks and grids size their children from the controls'
natural sizes, and only the part of the tree that changed is laid out
again.

## Stacks, spacers and padding

```rust,no_run
use appkit_ui::*;

let toolbar = hstack((
    button("Back", || {}),
    spacer(),                      // takes the free space
    text("Page 1 of 3"),
    spacer(),
    button("Next", || {}),
))
.spacing(8.0);
let page = vstack((toolbar, divider(), text("Content").fill_width())).padding(16.0);
# let _ = page;
```

- `vstack` and `hstack` take a tuple of views (or a `Vec<AnyView>` for a
  computed list). `.alignment(Align::Center)` aligns across the stack;
  `.spacing(points)` sets the gap.
- `spacer()` takes the space left over. A fixed view keeps its natural size.
- Modifiers, on any view: `.padding(p)`, `.width(w)`, `.height(h)`,
  `.frame(w, h)`, `.fill_width()`, `.fill_height()`.

## Grids: forms

A grid sizes its columns to their widest cell. `align_column` right-aligns
the labels of a form.

```rust,no_run
use appkit_ui::*;

let (name, email) = (Signal::new(String::new()), Signal::new(String::new()));
let form = grid([
    (text("Name:"), text_field(&name).accessibility_label("Name")),
    (text("Email:"), text_field(&email).accessibility_label("Email")),
])
.align_column(0, Align::End)
.spacing(8.0, 10.0);
# let _ = form;
```

## Text that changes

`text("…")` is fixed. `label(|| …)` re-runs when the signals it reads
change, and updates only its own text field.

## Lists: `for_each` and identity

`for_each` builds a row per item and keeps rows by **identity**: when the
list changes, rows whose id is still there are kept (with their native
views and state), new ids are built, and gone ones are torn down.

```rust,no_run
use appkit_ui::*;

#[derive(Clone)]
struct Task {
    id: u32,
    title: String,
}

impl Identifiable for Task {
    type Id = u32;
    fn id(&self) -> u32 {
        self.id
    }
}

let tasks = Signal::new(vec![Task { id: 1, title: "Write docs".into() }]);
let t = tasks.clone();
let list = scroll(vstack((for_each(move || t.get(), |task: Task| text(task.title)),)));
tasks.update(|v| v.push(Task { id: 2, title: "Ship".into() })); // builds one row
# let _ = list;
```

## Switching subtrees: `dynamic`

`dynamic(|| …)` rebuilds its subtree when the signals it read change. Use
it for "show this or that"; use `.id(key)` on a view to keep it across
rebuilds while the key is the same.

```rust,no_run
use appkit_ui::*;

let signed_in = Signal::new(false);
let s = signed_in.clone();
let body = dynamic(move || if s.get() { AnyView::new(text("Welcome back")) } else { AnyView::new(button("Sign in", || {})) });
# let _ = body;
```

## Scrolling

`scroll(view)` puts a view in an `NSScrollView` and lays its content out
at the visible width.

## Teardown

When a view leaves the tree (a list row removed, a `dynamic` branch
switched, a window closed) its native views are removed, its effects stop
and its memory is freed. AppKit itself keeps about 0.6 KB per view ever
shown, so prefer updating views over replacing them in hot paths.
