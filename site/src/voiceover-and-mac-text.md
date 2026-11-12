# VoiceOver, spell check and input methods in a Rust GUI

Most Rust GUI toolkits draw their own widgets, so they have to rebuild what the Mac already does: a text field that works with the Japanese input method's candidate window, spell checking with the red underline and the right-click suggestions, an accessibility tree VoiceOver can walk. Several are doing that work (egui and Slint support screen readers through AccessKit, and Iced's support is in progress), and how far each has got is on [the comparison page](compare.md).

Ferrokit takes the other route: its views *are* the system's controls. A `text_field` is an `NSTextField`, a `text_editor` is an `NSTextView`, a `table` is an `NSTableView`. So these come from macOS itself:

- **Input methods:** Japanese, Chinese and Korean composition with the candidate window at the caret; the emoji picker (⌃⌘Space); dictation.
- **Spelling and grammar:** the underline, suggestions on right-click, Edit ▸ Spelling and Grammar.
- **Text editing:** every Mac shortcut (⌥← by word, ⌘← to the line start, ⌃K and the other Emacs keys), Find and Replace (⌘F), undo per window, the Services menu, Look Up, Writing Tools where the Mac offers them.
- **VoiceOver:** AppKit's own accessibility tree: tables read by row and cell, sliders with their values, buttons with their titles. Full Keyboard Access and Tab order follow.
- **Right-to-left** layout for Arabic and Hebrew, and the user's own date and number formats.

## The one thing you do: name your controls

VoiceOver reads a control's name. A button is named by its title, but a text field after a `text("Title:")` label has no name of its own: the label is a separate view. Give it one with `.accessibility_label(..)`, and let a self-check catch the ones you forget:

```rust,no_run
{{#include ../snippets/src/bin/accessible_notes.rs}}
```

`testing::ax_audit(&window)` lists the controls VoiceOver couldn't name; `cargo ferrokit check` runs it with the rest of your self-checks. [Controls and forms](guide/04-controls-and-forms.md#accessibility-name-every-control) has the rules, and [Testing and shipping](guide/08-testing-and-shipping.md) the checks.

## What it costs

The same choice that gives you the Mac's own behaviour makes Ferrokit **Mac only**. If your app must also run on Windows and Linux, a cross-platform toolkit is the better fit, and you'll be testing its IME and screen-reader support on each platform. See [the comparison](compare.md).
