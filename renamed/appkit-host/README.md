# appkit-host is now `ferrokit-core`

`appkit-host` was renamed to [`ferrokit-core`](https://crates.io/crates/ferrokit-core) when the
project took one name, [Ferrokit](https://github.com/lhassa8/ferrokit).
It's the shared native layer under `ferrokit` and `ferrokit-iced`; most apps want one of those instead.

To move over, change the dependency and the paths:

```toml
[dependencies]
ferrokit-core = "0.4"
```

```rust,ignore
use ferrokit_core::*;
```

This release (0.3.0) only re-exports `ferrokit-core` 0.4, so code written for it
keeps building. Earlier versions of `appkit-host` stay available unchanged.
