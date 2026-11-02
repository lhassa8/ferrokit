//! Forms: labelled rows in sections ([`form`]), titled boxes
//! ([`group_box`]) and disclosure groups ([`disclosure`]), laid out as
//! System Settings and Mac settings windows lay them out.

use objc2::rc::Retained;
use objc2_app_kit::{NSAccessibility, NSBezelStyle, NSButton, NSButtonType, NSView};
use objc2_foundation::NSString;

use crate::appkit_views::{divider, text};
use crate::signal::Signal;
use crate::style::Font;
use crate::view::{Align, AnyView, Cx, Node, View, ViewExt, dynamic, grid, hstack, vstack};
use ferrokit_core::text::Color;

/// How a [`form`] lays out its rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormStyle {
    /// As System Settings: each section a rounded box of rows, labels on
    /// the left and controls on the right, lines between rows.
    #[default]
    Grouped,
    /// As classic settings windows: labels right-aligned in one column
    /// ending in a colon, controls in the next, aligned across sections.
    Columns,
}

/// A row of a [`section`]: a label and its control ([`form_row`]), or
/// content across the row ([`form_content`]).
pub struct FormRow {
    label: Option<String>,
    content: AnyView,
    help: Option<String>,
}

impl FormRow {
    /// A line of smaller text under the row, saying more about it.
    pub fn help(mut self, text: &str) -> Self {
        self.help = Some(text.into());
        self
    }
}

/// A row with `label` and its control. VoiceOver reads the label as the
/// control's name (unless it has a title of its own, as a checkbox does).
pub fn form_row(label: &str, control: impl View) -> FormRow {
    FormRow { label: Some(label.into()), content: AnyView::new(control), help: None }
}

/// A row of content without a label (a checkbox, a note, a button).
pub fn form_content(content: impl View) -> FormRow {
    FormRow { label: None, content: AnyView::new(content), help: None }
}

/// A group of rows in a [`form`], with a title (`""` for none).
pub struct Section {
    title: String,
    footer: Option<String>,
    rows: Vec<FormRow>,
}

impl Section {
    /// Smaller text under the section: what its settings do.
    pub fn footer(mut self, text: &str) -> Self {
        self.footer = Some(text.into());
        self
    }
}

/// A section of a [`form`]: a title and rows.
pub fn section(title: &str, rows: Vec<FormRow>) -> Section {
    Section { title: title.into(), footer: None, rows }
}

/// Settings and inspectors as Mac apps lay them out (see [`FormStyle`]).
///
/// ```no_run
/// use ferrokit::*;
///
/// let (name, sync, size) = (Signal::new(String::new()), Signal::new(true), Signal::new(12.0));
/// let settings = form(vec![
///     section("Account", vec![
///         form_row("Name", text_field(&name)),
///         form_row("Sync", toggle("", &sync)).help("Keeps your library the same on every Mac."),
///     ]),
///     section("Appearance", vec![form_row("Text size", slider(&size, 9.0, 24.0))]).footer("Applies to new windows."),
/// ]);
/// # let _ = settings;
/// ```
pub struct Form {
    sections: Vec<Section>,
    style: FormStyle,
}

impl Form {
    /// How the rows are laid out ([`FormStyle::Grouped`] unless set).
    pub fn style(mut self, style: FormStyle) -> Self {
        self.style = style;
        self
    }
}

/// A form of `sections` (see [`Form`]).
pub fn form(sections: Vec<Section>) -> Form {
    Form { sections, style: FormStyle::Grouped }
}

/// A node built already, to place among views.
struct Built(Node);

impl View for Built {
    fn build(self, _cx: &mut Cx) -> Node {
        self.0
    }
}

fn caption(s: &str) -> AnyView {
    AnyView::new(text(s).font(Font::caption()).color(Color::Secondary))
}

/// Builds a row's label and control, naming the control by the label for
/// VoiceOver when it has no name of its own.
fn labelled(cx: &mut Cx, label: &str, content: AnyView) -> (Node, Node) {
    let name = label.trim_end_matches(':').to_string();
    let label = text(label).build(&mut cx.child());
    let content = content.build_node(&mut cx.child());
    let mut c = Vec::new();
    content.leaf_views(&mut c);
    // The row's first control (not an icon before it) that VoiceOver has
    // no name for: a field, a slider, a stepper.
    let named = |v: &NSView| v.accessibilityLabel().is_some_and(|s| s.length() > 0) || v.accessibilityTitle().is_some_and(|s| s.length() > 0);
    let control = c.iter().find(|v| v.downcast_ref::<objc2_app_kit::NSControl>().is_some() && v.downcast_ref::<objc2_app_kit::NSImageView>().is_none());
    if let Some(c) = control.filter(|c| !named(c)) {
        // (On the cell too: macOS 27 shows VoiceOver a cell-based control's cell.)
        <crate::platform::Native as crate::platform::Platform>::set_accessibility_label(c, &name);
    }
    (label, content)
}

impl View for Form {
    fn build(self, cx: &mut Cx) -> Node {
        match self.style {
            FormStyle::Grouped => {
                let sections: Vec<AnyView> = self
                    .sections
                    .into_iter()
                    .map(|s| {
                        let mut rows: Vec<AnyView> = Vec::new();
                        let count = s.rows.len();
                        for (i, r) in s.rows.into_iter().enumerate() {
                            let line: AnyView = match r.label {
                                Some(label) => {
                                    let (l, c) = labelled(cx, &label, r.content);
                                    AnyView::new(hstack((Built(l).fill_width(), Built(c))).spacing(12.0))
                                }
                                None => r.content,
                            };
                            let row: AnyView = match r.help {
                                Some(h) => AnyView::new(vstack((line, caption(&h))).spacing(4.0).alignment(Align::Start)),
                                None => line,
                            };
                            rows.push(AnyView::new(row.padding_xy(12.0, 8.0)));
                            if i + 1 < count {
                                rows.push(AnyView::new(divider().padding_xy(12.0, 0.0)));
                            }
                        }
                        let mut parts: Vec<AnyView> = Vec::new();
                        if !s.title.is_empty() {
                            parts.push(AnyView::new(text(s.title).font(Font::headline()).padding_xy(4.0, 0.0)));
                        }
                        parts.push(AnyView::new(vstack(rows).spacing(0.0).alignment(Align::Start).fill_width().background(Color::QuaternaryFill).corner_radius(10.0)));
                        if let Some(f) = s.footer {
                            parts.push(AnyView::new(caption(&f).padding_xy(4.0, 0.0)));
                        }
                        AnyView::new(vstack(parts).spacing(6.0).alignment(Align::Start).fill_width())
                    })
                    .collect();
                vstack(sections).spacing(18.0).alignment(Align::Start).fill_width().build(cx)
            }
            FormStyle::Columns => {
                // One grid, so the labels line up across sections.
                let mut rows: Vec<Vec<AnyView>> = Vec::new();
                let blank = || AnyView::new(text(""));
                for (n, s) in self.sections.into_iter().enumerate() {
                    if n > 0 {
                        rows.push(vec![blank(), AnyView::new(divider())]);
                    }
                    if !s.title.is_empty() {
                        rows.push(vec![AnyView::new(text(s.title).font(Font::headline())), blank()]);
                    }
                    for r in s.rows {
                        let content = match r.help {
                            Some(h) => AnyView::new(vstack((r.content, caption(&h))).spacing(4.0).alignment(Align::Start)),
                            None => r.content,
                        };
                        match r.label {
                            Some(label) => {
                                let (l, c) = labelled(cx, &format!("{label}:"), content);
                                rows.push(vec![AnyView::new(Built(l)), AnyView::new(Built(c))]);
                            }
                            None => rows.push(vec![blank(), content]),
                        }
                    }
                    if let Some(f) = s.footer {
                        rows.push(vec![blank(), caption(&f)]);
                    }
                }
                grid(rows).align_column(0, Align::End).spacing(10.0, 10.0).build(cx)
            }
        }
    }
}

/// A titled box: `content` on the system's fill for grouped content,
/// rounded, its title above (SwiftUI's `GroupBox`).
pub fn group_box(title: &str, content: impl View) -> impl View {
    let mut parts: Vec<AnyView> = Vec::new();
    if !title.is_empty() {
        parts.push(AnyView::new(text(title).font(Font::headline()).padding_xy(4.0, 0.0)));
    }
    parts.push(AnyView::new(content.padding(12.0).background(Color::QuaternaryFill).corner_radius(10.0).fill_width()));
    vstack(parts).spacing(6.0).alignment(Align::Start)
}

/// AppKit's disclosure triangle, bound two ways to a signal.
struct Triangle(Signal<bool>, String);

impl View for Triangle {
    fn build(self, cx: &mut Cx) -> Node {
        let b = NSButton::new(cx.mtm);
        b.setButtonType(NSButtonType::PushOnPushOff);
        b.setBezelStyle(NSBezelStyle::Disclosure);
        b.setTitle(&NSString::from_str(""));
        b.setAccessibilityLabel(Some(&NSString::from_str(&self.1)));
        b.sizeToFit();
        let view: Retained<NSView> = Retained::into_super(Retained::into_super(b.clone()));
        crate::controls::bound_toggle(cx, view, b, self.0)
    }
}

/// A title with a disclosure triangle; clicking either shows or hides
/// `content`, built when it's shown (`expanded` says whether it is, both
/// ways). For advanced settings and details.
///
/// ```no_run
/// use ferrokit::*;
///
/// let open = Signal::new(false);
/// let retries = Signal::new(3.0);
/// let advanced = disclosure("Advanced", &open, move || form(vec![section("", vec![form_row("Retries", stepper(&retries, 0.0, 10.0, 1.0))])]));
/// # let _ = advanced;
/// ```
pub fn disclosure<V: View>(title: &str, expanded: &Signal<bool>, content: impl Fn() -> V + 'static) -> impl View {
    let expanded = *expanded;
    let header = hstack((Triangle(expanded, title.into()), text(title).font(Font::headline()).on_click(move || expanded.set(!expanded.get())))).spacing(4.0);
    let body = dynamic(move || if expanded.get() { AnyView::new(content().padding_xy(18.0, 0.0)) } else { AnyView::new(vstack(Vec::<AnyView>::new())) });
    vstack((header, body)).spacing(8.0).alignment(Align::Start)
}
