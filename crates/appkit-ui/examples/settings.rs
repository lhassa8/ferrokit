//! F1.8 exit check: a settings form written with appkit-ui only.
//! `cargo run -p appkit-ui --example settings`; `CHECK=all` runs its
//! self-checks; `SNAP=<dir>` also saves light and dark window images.

use appkit_ui::*;

#[derive(Clone)]
struct Settings {
    name: Signal<String>,
    email: Signal<String>,
    password: Signal<String>,
    theme: Signal<Appearance>,
    notify: Signal<bool>,
    sounds: Signal<bool>,
    volume: Signal<f64>,
    keep_days: Signal<f64>,
    syncing: Signal<bool>,
    status: Signal<String>,
}

impl Settings {
    fn new() -> Self {
        Settings {
            name: Signal::new("Ada Lovelace".into()),
            email: Signal::new("ada@example.com".into()),
            password: Signal::new(String::new()),
            theme: Signal::new(Appearance::System),
            notify: Signal::new(true),
            sounds: Signal::new(false),
            volume: Signal::new(0.6),
            keep_days: Signal::new(30.0),
            syncing: Signal::new(false),
            status: Signal::new("Not synced yet".into()),
        }
    }

    fn email_ok(&self) -> bool {
        self.email.with(|e| e.split_once('@').is_some_and(|(user, host)| !user.is_empty() && host.contains('.')))
    }

    fn sync(&self) {
        self.syncing.set(true);
        self.status.set("Syncing…".into());
        let s = self.clone();
        spawn(|| std::thread::sleep(std::time::Duration::from_millis(300)), move |_| {
            s.syncing.set(false);
            s.status.set(format!("Synced {} settings", 7));
        });
    }
}

fn account(s: &Settings) -> impl View {
    let s1 = s.clone();
    vstack((
        text("Account"),
        grid([
            (text("Name:"), text_field(&s.name).placeholder("Full name")),
            (text("Email:"), text_field(&s.email).placeholder("name@example.com")),
            (text("Password:"), secure_field(&s.password).placeholder("Required")),
        ])
        .align_column(0, Align::End),
        label(move || if s1.email_ok() { String::new() } else { "Enter a valid email address.".into() }),
    ))
}

fn appearance(s: &Settings) -> impl View {
    let theme = s.theme.clone();
    let options = vec![("System", Appearance::System), ("Light", Appearance::Light), ("Dark", Appearance::Dark)];
    vstack((
        text("Appearance"),
        grid([(text("Theme:"), picker(&s.theme, options))]).align_column(0, Align::End),
        watch(move || set_appearance(theme.get())),
    ))
}

fn notifications(s: &Settings) -> impl View {
    let days = s.keep_days.clone();
    vstack((
        text("Notifications"),
        toggle("Show notifications", &s.notify),
        checkbox("Play sounds", &s.sounds),
        grid([
            (text("Volume:"), hstack((image("speaker.fill").description("Volume"), slider(&s.volume, 0.0, 1.0)))),
            (text("Keep for:"), hstack((stepper(&s.keep_days, 1.0, 365.0, 1.0), label(move || format!("{} days", days.get()))))),
        ])
        .align_column(0, Align::End),
    ))
}

fn sync_row(s: &Settings) -> impl View {
    let (s1, s2, s3, s4) = (s.clone(), s.clone(), s.clone(), s.clone());
    hstack((
        button("Sync Now", move || s1.sync()).disabled(move || s2.syncing.get()),
        progress(move || if s3.syncing.get() { None } else { Some(1.0) }),
        label(move || s4.status.get()),
    ))
}

fn settings_view(s: &Settings) -> impl View {
    let s1 = s.clone();
    let can_save = move || s1.email_ok() && !s1.name.with(String::is_empty);
    vstack((
        account(s),
        divider(),
        appearance(s),
        divider(),
        notifications(s),
        divider(),
        sync_row(s),
        spacer(),
        hstack((spacer(), button("Save", || {}).disabled(move || !can_save()).default_action())),
    ))
    .padding(20.0)
}

fn main() {
    let s = Settings::new();
    let view = settings_view(&s);
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    App::new("Settings")
        .window(Window::new("Settings").size(460.0, 560.0).view(view))
        .on_launch(move || {
            if checks {
                self_checks::run(s);
            }
        })
        .run();
}

// ---- self-checks (not part of the app code count) ----

mod self_checks {
    use std::cell::Cell;

    use appkit_ui::testing::{ax_tree, has_ax, post_edit, shortcut, snapshot_windows};
    use appkit_ui::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSAppearanceCustomization, NSButton, NSSecureTextField, NSTextField, NSView};
    use objc2_foundation::NSString;

    use super::Settings;

    thread_local! {
        static FAILED: Cell<u32> = const { Cell::new(0) };
    }

    fn check(name: &str, pass: bool, note: String) {
        eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
        if !pass {
            FAILED.with(|f| f.set(f.get() + 1));
        }
    }

    fn views() -> Vec<Retained<NSView>> {
        windows()[0].contentView().unwrap().subviews().to_vec()
    }

    fn fields() -> Vec<Retained<NSTextField>> {
        views().into_iter().filter_map(|v| v.downcast::<NSTextField>().ok()).filter(|f| f.isEditable()).collect()
    }

    fn button(title: &str) -> Retained<NSButton> {
        views().into_iter().filter_map(|v| v.downcast::<NSButton>().ok()).find(|b| b.title().to_string() == title).unwrap()
    }

    fn label_with(text: &str) -> Option<Retained<NSTextField>> {
        views().into_iter().filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.stringValue().to_string().contains(text))
    }

    pub fn run(s: Settings) {
        let w = windows()[0].clone();
        w.makeKeyAndOrderFront(None);

        // Form layout: labels right-aligned in one column, fields after it.
        let labels: Vec<_> = ["Name:", "Email:", "Password:"].iter().map(|t| label_with(t).unwrap().frame()).collect();
        let right = |r: &objc2_foundation::NSRect| r.origin.x + r.size.width;
        let f = fields();
        check(
            "form-layout",
            labels.iter().all(|r| (right(r) - right(&labels[0])).abs() < 0.5) && f.iter().take(3).all(|x| x.frame().origin.x == f[0].frame().origin.x),
            format!("label right edges {:?}; fields start at x {:.0}", labels.iter().map(right).collect::<Vec<_>>(), f[0].frame().origin.x),
        );

        // Validation: a bad email shows the hint and disables Save.
        let save = button("Save");
        let ok_before = save.isEnabled();
        post_edit(&f[1], "ada@");
        let hint = label_with("valid email").is_some();
        let disabled = !save.isEnabled();
        post_edit(&f[1], "ada@example.org");
        check(
            "validation",
            ok_before && hint && disabled && save.isEnabled() && label_with("valid email").is_none(),
            format!("Save enabled {ok_before} → {} (bad email, hint shown {hint}) → {}", !disabled, save.isEnabled()),
        );

        // Tab: Name → Email → Password.
        let secure = views().into_iter().find_map(|v| v.downcast::<NSSecureTextField>().ok()).unwrap();
        w.makeFirstResponder(Some(&f[0]));
        let tab = |field: &NSTextField| {
            if let Some(e) = field.currentEditor() {
                let _: () = unsafe { objc2::msg_send![&*e, insertTab: Option::<&objc2::runtime::AnyObject>::None] };
            }
        };
        tab(&f[0]);
        let at_email = f[1].currentEditor().is_some();
        tab(&f[1]);
        check("tab-order", at_email && secure.currentEditor().is_some(), format!("Name → Email: {at_email}; Email → Password: {}", secure.currentEditor().is_some()));

        // Shortcut through the menu: ⌘A in the name field.
        w.makeFirstResponder(Some(&f[0]));
        let all = shortcut(&w, "a", false) && f[0].currentEditor().is_some_and(|e| e.selectedRange().length == s.name.get_untracked().chars().count());
        check("select-all", all, "⌘A selects the whole name".into());

        // Accessibility: every control is in the tree with its role.
        let tree = ax_tree(&w);
        let roles = [
            ("AXTextField", ""),
            ("AXTextField", "AXSecureTextField"),
            ("AXPopUpButton", ""),
            ("AXCheckBox", ""),
            ("AXSlider", ""),
            ("AXIncrementor", ""),
            ("AXButton", ""),
            ("AXProgressIndicator", ""),
        ];
        let missing: Vec<_> = roles.iter().filter(|(r, sr)| !has_ax(&tree, r, sr, "")).collect();
        check(
            "accessibility-tree",
            missing.is_empty() && tree.iter().any(|(_, sr, _)| sr == "AXSwitch") && has_ax(&tree, "AXImage", "", "Volume"),
            format!("{} elements; missing {missing:?}", tree.len()),
        );

        // Sync: background work, progress indeterminate while it runs, then done.
        unsafe { button("Sync Now").performClick(None) };
        let busy = s.syncing.get_untracked() && !button("Sync Now").isEnabled();
        let s2 = s.clone();
        let poll = std::rc::Rc::new(Cell::new(0));
        fn wait(s: Settings, busy: bool, tries: std::rc::Rc<Cell<u32>>) {
            later(move || {
                if s.syncing.get_untracked() && tries.get() < 2000 {
                    tries.set(tries.get() + 1);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    wait(s.clone(), busy, tries.clone());
                    return;
                }
                check(
                    "sync",
                    busy && !s.syncing.get_untracked() && label_with("Synced 7").is_some() && button("Sync Now").isEnabled(),
                    format!("busy while running: {busy}; status {:?}", s.status.get_untracked()),
                );
                appearance(s.clone());
            });
        }
        wait(s2, busy, poll);
    }

    fn appearance(s: Settings) {
        // Theme picker → app appearance → every native view.
        let names = unsafe {
            objc2_foundation::NSArray::from_slice(&[objc2_app_kit::NSAppearanceNameAqua, objc2_app_kit::NSAppearanceNameDarkAqua])
        };
        let dark_count = || {
            views()
                .iter()
                .filter(|v| {
                    v.effectiveAppearance()
                        .bestMatchFromAppearancesWithNames(&names)
                        .is_some_and(|n: Retained<NSString>| n.to_string().contains("Dark"))
                })
                .count()
        };
        batch(|| s.theme.set(Appearance::Light));
        let snap = std::env::var("SNAP").ok();
        if let Some(d) = &snap {
            snapshot_windows(&format!("{d}/light"));
        }
        let light = dark_count();
        batch(|| s.theme.set(Appearance::Dark));
        if let Some(d) = &snap {
            snapshot_windows(&format!("{d}/dark"));
        }
        let dark = dark_count();
        let total = views().len();
        batch(|| s.theme.set(Appearance::System));
        check("theme-picker", light == 0 && dark == total, format!("Light: {light} of {total} views dark; Dark: {dark} of {total}"));
        let failed = FAILED.with(Cell::get);
        eprintln!("[check] done, {failed} failed");
        std::process::exit(if failed == 0 { 0 } else { 1 });
    }
}
