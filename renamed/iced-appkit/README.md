# iced-appkit is now `ferrokit-iced`

`iced-appkit` was renamed to [`ferrokit-iced`](https://crates.io/crates/ferrokit-iced) when the
project took one name, [Ferrokit](https://github.com/lhassa8/ferrokit).
It puts real AppKit views (tables, text editors) inside Iced windows.

To move over, change the dependency and the paths:

```toml
[dependencies]
ferrokit-iced = "0.4"
```

```rust,ignore
use ferrokit_iced::*;
```

This release (0.3.0) only re-exports `ferrokit-iced` 0.4, so code written for it
keeps building. Earlier versions of `iced-appkit` stay available unchanged.
