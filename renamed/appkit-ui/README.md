# appkit-ui is now `ferrokit`

`appkit-ui` was renamed to [`ferrokit`](https://crates.io/crates/ferrokit) when the
project took one name, [Ferrokit](https://github.com/lhassa8/ferrokit).
It's the framework for building whole Mac apps from real AppKit views.

To move over, change the dependency and the paths:

```toml
[dependencies]
ferrokit = "0.4"
```

```rust,ignore
use ferrokit::*;
```

This release (0.4.0) only re-exports `ferrokit` 0.4, so code written for it
keeps building. Earlier versions of `appkit-ui` stay available unchanged.
