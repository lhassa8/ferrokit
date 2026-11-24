# Security

Please report a security issue privately: use **Report a vulnerability** on the repository's [Security tab](https://github.com/lhassa8/ferrokit/security), not a public issue.

You'll get a reply within 3 days. A confirmed issue gets a patch release within 14 days, and the report is published as an advisory once the fix is out. The fix goes into the latest release (see [STABILITY.md](STABILITY.md#fixes)).

Ferrokit runs inside your app with its permissions, so the issues that matter most are ones that let a file, a pasteboard, a URL or a web view's content reach further than the app meant it to: for example, a sandbox bookmark that resolves somewhere else, a `myapp://` link that runs an action without the app's handler, or a web view that reaches native code.
