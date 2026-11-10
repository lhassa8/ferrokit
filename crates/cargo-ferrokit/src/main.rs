//! `cargo ferrokit`: make, run, bundle, check and notarize native Mac apps
//! built with ferrokit. Settings live in the app's Cargo.toml under
//! `[package.metadata.ferrokit]` (see the template `cargo ferrokit new`
//! writes).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

const USAGE: &str = "\
cargo ferrokit: native Mac apps with ferrokit

USAGE:
    cargo ferrokit new <path> [--document] [--name <app name>] [--id <bundle id>]
                              [--ferrokit-path <dir>]
        Makes a new app: a to-do list, or with --document a text document app.
        --ferrokit-path uses a local checkout of ferrokit instead of crates.io.

    cargo ferrokit run [--release] [--bin <name>] [-- <args>…]
        Builds, bundles (so the app has its name, icon and identity) and runs
        it, with its output in the terminal.

    cargo ferrokit preview [--release] [--bin <name>] [-- <args>…]
        Runs the app like `run`, then rebuilds and restarts it each time you
        save a file in the package. Windows open where you left them and the
        app doesn't take the focus from your editor; a build that fails
        leaves the running app as it was. ⌃C stops.

    cargo ferrokit bundle [--debug] [--bin <name>] [--universal] [--sign <identity>]
                          [--zip] [--out <dir>]
        Makes <name>.app (release by default), signed with the hardened
        runtime: ad hoc, or with `sign` from Cargo.toml or --sign.
        --universal builds for Apple silicon and Intel.

    cargo ferrokit bundle --app-store [--build <number>] [--dry-run] [--bin <name>]
        For the Mac App Store: a universal app with the App Sandbox, its
        provisioning profile embedded, signed with `app-store-sign`, in a .pkg
        signed with `installer-sign`, ready to upload (Transporter). --build
        sets the build number (CFBundleVersion; each upload needs a higher
        one). --dry-run signs ad hoc, makes the package unsigned, and prints
        the signing commands.

    cargo ferrokit appcast [--key-file <file>] [--app <path>] [--out <dir>] [--bin <name>]
        For updates through Sparkle: zips the bundled app as \"<name> <version>.zip\"
        in the updates folder (updates/ by default) and writes the feed there with
        Sparkle's generate_appcast, signing with the key from your keychain (or
        --key-file). Upload the folder's new files where `feed` points.

    cargo ferrokit check [--no-lint] [--timeout <seconds>] [--bin <name>]
        Builds, runs clippy (warnings are errors) and the tests, bundles, then
        runs the app's self-checks (the app with CHECK=all; see the template)
        and verifies the signature.

    cargo ferrokit notarize [--profile <notary profile>] [--sign <identity>]
                            [--universal] [--dry-run] [--bin <name>]
        Bundles with a Developer ID, sends it to Apple's notary service, waits,
        staples the ticket and zips the app for sharing. --dry-run does
        everything up to the upload and prints the commands it would run.

Settings, in the app's Cargo.toml:
    [package.metadata.ferrokit]
    name = \"My App\"            # shown in the menu bar, Finder and Dock
    id = \"com.example.myapp\"   # bundle identifier
    icon = \"icon.png\"          # square PNG (1024 px) or .icns
    min-macos = \"15.0\"
    category = \"public.app-category.productivity\"
    languages = [\"de\", \"fr\"]     # its translations (see ferrokit's App::translations)
    sign = \"Developer ID Application: …\"
    notary-profile = \"…\"       # xcrun notarytool store-credentials …
    sandbox = true              # the App Sandbox (needed for the Mac App Store):
                                # files the user chooses, bookmarks to reopen them
    network = true              # with the sandbox: outgoing connections
    app-store-sign = \"Apple Distribution: …\"           # bundle --app-store
    installer-sign = \"3rd Party Mac Developer Installer: …\"
    provisioning-profile = \"MyApp.provisionprofile\"   # from developer.apple.com
    [[package.metadata.ferrokit.documents]]
    name = \"Text\"
    extensions = [\"txt\", \"md\"]
    role = \"viewer\"          # only reads these (default: editor)
    related = [\"-journal\"]     # files beside a document it opens too (a
                                # database's journal), reached in the sandbox
                                # through ferrokit's related_files
    [package.metadata.ferrokit.updates]                # updates through Sparkle 2
    feed = \"https://example.com/myapp/appcast.xml\"
    public-key = \"…\"          # what Sparkle's bin/generate_keys prints
    sparkle = \"~/Developer/Sparkle-2.10.1\"   # Sparkle's release, unpacked
                                # (or FERROKIT_SPARKLE)";

type Result<T> = std::result::Result<T, String>;

/// A kind of file the app opens (`[[package.metadata.ferrokit.documents]]`).
struct DocType {
    name: String,
    extensions: Vec<String>,
    /// Suffixes of files beside a document that it opens too, such as a
    /// database's `-wal` and `-journal`.
    related: Vec<String>,
    /// "Editor" (the default) or "Viewer": an app that only reads these
    /// files (a data file opened as a table) says so.
    role: String,
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // As a cargo subcommand, cargo passes its name first.
    if args.first().map(String::as_str) == Some("ferrokit") {
        args.remove(0);
    }
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("new") => new(rest),
        Some("run") => run(rest),
        Some("preview") => preview(rest),
        Some("bundle") => bundle_command(rest),
        Some("check") => check(rest),
        Some("notarize") => notarize(rest),
        Some("appcast") => appcast(rest),
        Some("-V" | "--version") => {
            println!("cargo-ferrokit {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("-h" | "--help" | "help") | None => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("no command `{other}` (see cargo ferrokit --help)")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cargo ferrokit: {e}");
            ExitCode::FAILURE
        }
    }
}

// ---- arguments ----

/// Parsed arguments: flags, options with values, positionals, and what
/// follows `--`.
struct Args {
    flags: Vec<String>,
    values: Vec<(String, String)>,
    positional: Vec<String>,
    rest: Vec<String>,
}

impl Args {
    /// `valued` lists the options that take a value; `allowed` the flags.
    fn parse(args: &[String], valued: &[&str], allowed: &[&str]) -> Result<Args> {
        let mut parsed = Args { flags: Vec::new(), values: Vec::new(), positional: Vec::new(), rest: Vec::new() };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a == "--" {
                parsed.rest = it.cloned().collect();
                break;
            }
            if let Some((name, value)) = a.split_once('=').filter(|(n, _)| valued.contains(n)) {
                parsed.values.push((name.into(), value.into()));
            } else if valued.contains(&a.as_str()) {
                // Not the next option taken as its value (`--sign --zip`).
                let value = it.next().filter(|v| !v.starts_with("--")).ok_or_else(|| format!("{a} needs a value"))?;
                parsed.values.push((a.clone(), value.clone()));
            } else if allowed.contains(&a.as_str()) {
                parsed.flags.push(a.clone());
            } else if a.starts_with('-') {
                return Err(format!("unknown option {a} (see cargo ferrokit --help)"));
            } else {
                parsed.positional.push(a.clone());
            }
        }
        Ok(parsed)
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

// ---- running tools ----

/// Runs `program args…`, showing the command; an error if it fails.
fn sh(program: &str, args: &[&str], dir: Option<&Path>) -> Result<()> {
    let shown = Path::new(program).file_name().map_or(program.into(), |n| n.to_string_lossy());
    eprintln!("    \x1b[2m{shown} {}\x1b[0m", args.join(" "));
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let status = cmd.status().map_err(|e| format!("couldn't run {program}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{program} {} failed ({status})", args.first().copied().unwrap_or_default())) }
}

/// Runs `program args…` like [`sh`], its output shown with the commands'
/// (on stderr), so stdout keeps only what `cargo ferrokit` prints.
fn sh_quiet(program: &str, args: &[&str]) -> Result<()> {
    eprintln!("    \x1b[2m{program} {}\x1b[0m", args.join(" "));
    let out = output(program, args, None)?;
    if !out.trim().is_empty() {
        eprintln!("{}", out.trim_end());
    }
    Ok(())
}

/// Runs a command and returns its standard output.
fn output(program: &str, args: &[&str], dir: Option<&Path>) -> Result<String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let out = cmd.output().map_err(|e| format!("couldn't run {program}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{program} {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn step(title: &str) {
    eprintln!("\x1b[1m{title}\x1b[0m");
}

// ---- the project ----

/// The app being built: its package, binary and bundle settings.
struct Project {
    package: String,
    version: String,
    dir: PathBuf,
    target_dir: PathBuf,
    bin: String,
    display: String,
    id: String,
    icon: Option<PathBuf>,
    min_macos: String,
    category: Option<String>,
    documents: Vec<DocType>,
    /// Languages the app speaks besides English (its translations).
    languages: Vec<String>,
    sign: Option<String>,
    notary_profile: Option<String>,
    /// Signed for the App Sandbox (`sandbox = true`), and with outgoing
    /// network access (`network = true`).
    sandbox: bool,
    network: bool,
    /// For `bundle --app-store`: the identities and the profile.
    app_store_sign: Option<String>,
    installer_sign: Option<String>,
    provisioning_profile: Option<PathBuf>,
    /// Updates through Sparkle (`[package.metadata.ferrokit.updates]`).
    updates: Option<Updates>,
    /// More entitlements and Info.plist keys, as written under
    /// `[package.metadata.ferrokit.entitlements]` and `….plist]`.
    entitlements: Vec<(String, Extra)>,
    plist: Vec<(String, Extra)>,
}

/// A value of an extra entitlement or Info.plist key.
#[derive(Debug, Clone, PartialEq)]
enum Extra {
    Bool(bool),
    Text(String),
    List(Vec<String>),
}

impl Extra {
    /// The keys and values of the table `name` in the settings.
    fn table(m: &serde_json::Value, name: &str) -> Result<Vec<(String, Extra)>> {
        let Some(table) = m[name].as_object() else {
            return if m[name].is_null() { Ok(Vec::new()) } else { Err(format!("[package.metadata.ferrokit] `{name}` must be a table of keys and values")) };
        };
        table
            .iter()
            .map(|(k, v)| {
                let list = v.as_array().map(|a| a.iter().map(|e| e.as_str().map(String::from)).collect::<Option<Vec<_>>>());
                let value = match (v.as_bool(), v.as_str(), list) {
                    (Some(b), ..) => Extra::Bool(b),
                    (_, Some(s), _) => Extra::Text(s.into()),
                    (_, _, Some(Some(l))) => Extra::List(l),
                    _ => return Err(format!("[package.metadata.ferrokit.{name}] `{k}` must be true or false, a string, or a list of strings")),
                };
                Ok((k.clone(), value))
            })
            .collect()
    }

    /// As a property list's key and value.
    fn xml(key: &str, value: &Extra) -> String {
        let value = match value {
            Extra::Bool(b) => format!("<{b}/>"),
            Extra::Text(s) => format!("<string>{}</string>", xml(s)),
            Extra::List(l) => format!("<array>{}</array>", l.iter().map(|s| format!("<string>{}</string>", xml(s))).collect::<String>()),
        };
        format!("    <key>{}</key>\n    {value}\n", xml(key))
    }
}

/// Fails on a setting `cargo ferrokit` doesn't know or one of the wrong
/// type: a misspelt `sandbox` would otherwise ship an app without it.
fn check_settings(m: &serde_json::Value) -> Result<()> {
    let Some(table) = m.as_object() else { return Ok(()) };
    const TEXT: [&str; 10] = ["name", "id", "icon", "min-macos", "category", "sign", "notary-profile", "app-store-sign", "installer-sign", "provisioning-profile"];
    const BOOL: [&str; 2] = ["sandbox", "network"];
    const OTHER: [&str; 5] = ["documents", "languages", "updates", "entitlements", "plist"];
    let unknown = |section: &str, key: &str, known: &[&str]| format!("[package.metadata.ferrokit{section}] has no setting `{key}` (it has: {})", known.join(", "));
    for (key, value) in table {
        if TEXT.contains(&key.as_str()) {
            if !value.is_string() {
                return Err(format!("[package.metadata.ferrokit] `{key}` must be a string, in quotes"));
            }
        } else if BOOL.contains(&key.as_str()) {
            if !value.is_boolean() {
                return Err(format!("[package.metadata.ferrokit] `{key}` must be true or false, without quotes"));
            }
        } else if !OTHER.contains(&key.as_str()) {
            let known: Vec<&str> = TEXT.iter().chain(&BOOL).chain(&OTHER).copied().collect();
            return Err(unknown("", key, &known));
        }
    }
    for d in m["documents"].as_array().into_iter().flatten() {
        const KEYS: [&str; 4] = ["name", "extensions", "related", "role"];
        if let Some(key) = d.as_object().into_iter().flat_map(|o| o.keys()).find(|k| !KEYS.contains(&k.as_str())) {
            return Err(unknown(".documents", key, &KEYS));
        }
    }
    const UPDATES: [&str; 3] = ["feed", "public-key", "sparkle"];
    if let Some(key) = m["updates"].as_object().into_iter().flat_map(|o| o.keys()).find(|k| !UPDATES.contains(&k.as_str())) {
        return Err(unknown(".updates", key, &UPDATES));
    }
    Ok(())
}

/// Where the app's updates come from and the key that checks them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Updates {
    feed: String,
    public_key: String,
    /// Sparkle's unpacked release (`Sparkle.framework`, `bin/`).
    sparkle: Option<PathBuf>,
}

impl Updates {
    fn from_metadata(u: &serde_json::Value, dir: &Path) -> Result<Option<Updates>> {
        if u.is_null() {
            return Ok(None);
        }
        let text = |key: &str| u[key].as_str().map(String::from).filter(|s| !s.trim().is_empty());
        let feed = text("feed").ok_or("[package.metadata.ferrokit.updates] needs `feed`: the appcast's URL, as https://example.com/myapp/appcast.xml")?;
        if !feed.starts_with("https://") && !feed.starts_with("http://127.0.0.1") && !feed.starts_with("http://localhost") {
            return Err(format!("updates: the feed {feed} must be https:// (macOS refuses plain http)"));
        }
        let public_key = text("public-key").ok_or("[package.metadata.ferrokit.updates] needs `public-key`: run Sparkle's bin/generate_keys once and copy the key it prints")?;
        let sparkle = text("sparkle").map(|s| match s.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
            None => dir.join(s),
        });
        Ok(Some(Updates { feed, public_key, sparkle }))
    }

    /// Sparkle's release folder: `FERROKIT_SPARKLE`, else `sparkle`.
    fn sparkle_dir(&self) -> Result<PathBuf> {
        let dir = std::env::var_os("FERROKIT_SPARKLE").map(PathBuf::from).or_else(|| self.sparkle.clone()).ok_or(
            "updates need Sparkle 2: download it from https://github.com/sparkle-project/Sparkle/releases, unpack it, and set `sparkle = \"<folder>\"` under [package.metadata.ferrokit.updates] (or FERROKIT_SPARKLE)",
        )?;
        if !dir.join("Sparkle.framework").is_dir() {
            return Err(format!("{} has no Sparkle.framework: point `sparkle` at Sparkle's unpacked release", dir.display()));
        }
        Ok(dir)
    }

    /// Where the update files go: the feed's folder.
    fn url_prefix(&self) -> String {
        match self.feed.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/"),
            None => self.feed.clone(),
        }
    }

    fn feed_name(&self) -> &str {
        self.feed.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("appcast.xml")
    }
}

impl Project {
    /// The package in the current folder (or the only one in its workspace).
    fn load(bin: Option<&str>) -> Result<Project> {
        let json = output(&cargo(), &["metadata", "--no-deps", "--format-version", "1"], None)?;
        let meta: serde_json::Value = serde_json::from_str(&json).map_err(|e| format!("cargo metadata: {e}"))?;
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?.canonicalize().map_err(|e| e.to_string())?;
        let packages = meta["packages"].as_array().cloned().unwrap_or_default();
        let dir_of = |p: &serde_json::Value| PathBuf::from(p["manifest_path"].as_str().unwrap_or_default()).parent().map(Path::to_path_buf).unwrap_or_default();
        let package = packages
            .iter()
            .find(|p| dir_of(p).canonicalize().is_ok_and(|d| d == cwd))
            .or_else(|| (packages.len() == 1).then(|| &packages[0]))
            .ok_or("run this in an app's folder (where its Cargo.toml is)")?;
        let bins: Vec<String> = package["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| k == "bin")))
            .filter_map(|t| t["name"].as_str().map(String::from))
            .collect();
        let bin = match bin {
            Some(b) if bins.iter().any(|x| x == b) => b.to_string(),
            Some(b) => return Err(format!("no binary `{b}` (there's {})", bins.join(", "))),
            None if bins.len() == 1 => bins[0].clone(),
            None if bins.is_empty() => return Err("the package has no binary to bundle".into()),
            None => return Err(format!("several binaries ({}): choose one with --bin", bins.join(", "))),
        };
        let name = package["name"].as_str().unwrap_or_default().to_string();
        let m = &package["metadata"]["ferrokit"];
        check_settings(m)?;
        let text = |key: &str| m[key].as_str().map(String::from);
        let dir = dir_of(package);
        let documents = m["documents"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| {
                let list = |key: &str| d[key].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(String::from)).collect();
                DocType {
                    name: d["name"].as_str().unwrap_or("Document").to_string(),
                    extensions: list("extensions"),
                    related: list("related"),
                    role: if d["role"].as_str().is_some_and(|r| r.eq_ignore_ascii_case("viewer")) { "Viewer".into() } else { "Editor".into() },
                }
            })
            .collect();
        let updates = Updates::from_metadata(&m["updates"], &dir)?;
        let languages = m["languages"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)).filter(|l| l != "en").collect();
        Ok(Project {
            languages,
            version: package["version"].as_str().unwrap_or("0.1.0").to_string(),
            target_dir: PathBuf::from(meta["target_directory"].as_str().unwrap_or("target")),
            display: text("name").unwrap_or_else(|| bin.clone()),
            id: text("id").unwrap_or_else(|| format!("com.example.{}", name.replace(['_', ' '], "-"))),
            icon: text("icon").map(|i| dir.join(i)),
            min_macos: text("min-macos").unwrap_or_else(|| "15.0".into()),
            category: text("category"),
            sign: text("sign"),
            sandbox: m["sandbox"].as_bool().unwrap_or(false),
            network: m["network"].as_bool().unwrap_or(false),
            notary_profile: text("notary-profile"),
            app_store_sign: text("app-store-sign"),
            installer_sign: text("installer-sign"),
            provisioning_profile: text("provisioning-profile").map(|p| dir.join(p)),
            updates,
            documents,
            entitlements: Extra::table(m, "entitlements")?,
            plist: Extra::table(m, "plist")?,
            package: name,
            dir,
            bin,
        })
    }
}

// ---- new ----

fn new(args: &[String]) -> Result<()> {
    let a = Args::parse(args, &["--name", "--id", "--ferrokit-path"], &["--document"])?;
    let [path] = a.positional.as_slice() else { return Err("usage: cargo ferrokit new <path> [--document]".into()) };
    let path = PathBuf::from(path);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("the path needs a folder name")?;
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') || name.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(format!("`{name}` isn't a valid package name: use letters, digits, - and _"));
    }
    if path.exists() && path.read_dir().map_err(|e| e.to_string())?.next().is_some() {
        return Err(format!("{} already exists and isn't empty", path.display()));
    }
    let display = a.value("--name").map(String::from).unwrap_or_else(|| {
        name.split(['-', '_'])
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ")
    });
    let id = a.value("--id").map(String::from).unwrap_or_else(|| format!("com.example.{}", name.to_lowercase().replace('_', "-")));
    let dep = match a.value("--ferrokit-path") {
        Some(p) => {
            let p = PathBuf::from(p).canonicalize().map_err(|e| format!("--ferrokit-path {p}: {e}"))?;
            if !p.join("Cargo.toml").exists() {
                return Err(format!("{} has no Cargo.toml (point it at crates/ferrokit)", p.display()));
            }
            format!("{{ path = {:?} }}", p.to_string_lossy())
        }
        None => format!("\"{}\"", FERROKIT_VERSION),
    };
    let document = a.flag("--document");
    let documents = if document {
        "\n# Files the app opens: the Finder offers it for these (Open With, double-click\n# once chosen), and dropping them on its icon opens them.\n[[package.metadata.ferrokit.documents]]\nname = \"Text\"\nextensions = [\"txt\", \"md\"]\n"
    } else {
        ""
    };
    let fill = |t: &str| t.replace("{{name}}", &name).replace("{{display}}", &display).replace("{{id}}", &id).replace("{{dep}}", &dep).replace("{{documents}}", documents);
    let main = if document { include_str!("../templates/document/main.rs") } else { include_str!("../templates/app/main.rs") };
    std::fs::create_dir_all(path.join("src")).map_err(|e| e.to_string())?;
    let write = |file: &str, text: String| std::fs::write(path.join(file), text).map_err(|e| format!("{file}: {e}"));
    let manifest: String = include_str!("../templates/Cargo.toml.in").lines().skip_while(|l| l.starts_with('#')).map(|l| format!("{l}\n")).collect();
    write("Cargo.toml", fill(&manifest).trim_end().to_string() + "\n")?;
    write("src/main.rs", fill(main))?;
    write(".gitignore", "/target\n".into())?;
    // For AI coding assistants: how a Ferrokit app is written, and its traps.
    write("AGENTS.md", fill(include_str!("../templates/AGENTS.md")).replace("{{version}}", FERROKIT_VERSION))?;
    write("CLAUDE.md", "@AGENTS.md\n".into())?;
    println!("Made {} ({}) in {}", display, if document { "a document app" } else { "an app" }, path.display());
    println!("Next:\n    cd {}\n    cargo ferrokit run      # run it\n    cargo ferrokit check    # lint, test and run its self-checks\n    cargo ferrokit bundle   # make {display}.app", path.display());
    Ok(())
}

/// The ferrokit that new apps depend on.
const FERROKIT_VERSION: &str = "0.6";

// ---- bundle ----

struct BundleOptions {
    release: bool,
    universal: bool,
    sign: String,
    out: Option<PathBuf>,
    /// The Mac App Store's additions: a build number, a profile to embed
    /// and entitlements from it; no hardened runtime.
    store: Option<Store>,
}

#[derive(Default)]
struct Store {
    build: Option<String>,
    profile: Option<PathBuf>,
    entitlements: Vec<(String, String)>,
}

fn bundle_command(args: &[String]) -> Result<()> {
    let a = Args::parse(args, &["--bin", "--sign", "--out", "--build"], &["--debug", "--universal", "--zip", "--app-store", "--dry-run"])?;
    let project = Project::load(a.value("--bin"))?;
    if a.flag("--app-store") {
        return app_store(&project, a.value("--build"), a.flag("--dry-run"), a.value("--out").map(PathBuf::from));
    }
    if a.flag("--dry-run") || a.value("--build").is_some() {
        return Err("--dry-run and --build go with --app-store".into());
    }
    let sign = a.value("--sign").map(String::from).or_else(|| project.sign.clone()).unwrap_or_else(|| "-".into());
    let app = bundle(&project, &BundleOptions { release: !a.flag("--debug"), universal: a.flag("--universal"), sign, out: a.value("--out").map(PathBuf::from), store: None })?;
    if a.flag("--zip") {
        let zip = zip(&app)?;
        println!("{}", zip.display());
    }
    println!("{}", app.display());
    Ok(())
}

fn native_triple() -> &'static str {
    if cfg!(target_arch = "x86_64") { "x86_64-apple-darwin" } else { "aarch64-apple-darwin" }
}

/// Builds the binary and makes the signed .app; its path.
fn bundle(p: &Project, o: &BundleOptions) -> Result<PathBuf> {
    let profile = if o.release { "release" } else { "debug" };
    // Sparkle, unless the App Store does the updating.
    let sparkle = match (&p.updates, &o.store) {
        (Some(u), None) => Some(u.sparkle_dir()?),
        (Some(_), Some(_)) => {
            eprintln!("    note: the Mac App Store updates the app: Sparkle and the feed are left out");
            None
        }
        (None, _) => None,
    };
    step(&format!("Building {} ({profile})", p.bin));
    let mut binaries = Vec::new();
    let triples: Vec<&str> = if o.universal { vec!["aarch64-apple-darwin", "x86_64-apple-darwin"] } else { vec![] };
    if triples.is_empty() {
        let mut args = vec!["build", "-p", &p.package, "--bin", &p.bin];
        if o.release {
            args.push("--release");
        }
        sh(&cargo(), &args, Some(&p.dir))?;
        binaries.push(p.target_dir.join(profile).join(&p.bin));
    } else {
        let installed = output("rustup", &["target", "list", "--installed"], None).unwrap_or_default();
        for t in &triples {
            if !installed.lines().any(|l| l.trim() == *t) && *t != native_triple() {
                return Err(format!("the Rust target {t} isn't installed: rustup target add {t}"));
            }
            let mut args = vec!["build", "-p", &p.package, "--bin", &p.bin, "--target", t];
            if o.release {
                args.push("--release");
            }
            sh(&cargo(), &args, Some(&p.dir))?;
            binaries.push(p.target_dir.join(t).join(profile).join(&p.bin));
        }
    }

    step(&format!("Bundling {}.app", p.display));
    let out = o.out.clone().unwrap_or_else(|| p.target_dir.join(profile).join("bundle"));
    let app = out.join(format!("{}.app", p.display));
    // Replaced, never updated in place: macOS kills a binary rewritten under
    // a cached signature.
    if app.exists() {
        std::fs::remove_dir_all(&app).map_err(|e| format!("{}: {e}", app.display()))?;
    }
    let (macos, resources) = (app.join("Contents/MacOS"), app.join("Contents/Resources"));
    std::fs::create_dir_all(&macos).and_then(|_| std::fs::create_dir_all(&resources)).map_err(|e| e.to_string())?;
    let exe = macos.join(&p.bin);
    if binaries.len() == 1 {
        std::fs::copy(&binaries[0], &exe).map_err(|e| format!("{}: {e}", binaries[0].display()))?;
    } else {
        let mut args = vec!["-create".to_string()];
        args.extend(binaries.iter().map(|b| b.to_string_lossy().into_owned()));
        args.extend(["-output".into(), exe.to_string_lossy().into_owned()]);
        sh("lipo", &args.iter().map(String::as_str).collect::<Vec<_>>(), None)?;
    }
    let icon = match &p.icon {
        Some(icon) => {
            make_icon(icon, &resources.join("AppIcon.icns"))?;
            true
        }
        None => false,
    };
    let build = o.store.as_ref().and_then(|s| s.build.as_deref());
    std::fs::write(app.join("Contents/Info.plist"), info_plist(p, icon, build, sparkle.is_some())).map_err(|e| e.to_string())?;
    sh("plutil", &["-lint", "-s", &app.join("Contents/Info.plist").to_string_lossy()], None)?;
    std::fs::write(app.join("Contents/PkgInfo"), "APPL????").map_err(|e| e.to_string())?;
    // An .lproj folder per language: how macOS learns the app is localised.
    for lang in std::iter::once("en").chain(p.languages.iter().map(String::as_str)).filter(|_| !p.languages.is_empty()) {
        let dir = resources.join(format!("{lang}.lproj"));
        std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join("InfoPlist.strings"), "")).map_err(|e| e.to_string())?;
    }
    let framework = app.join("Contents/Frameworks/Sparkle.framework");
    if let Some(dir) = &sparkle {
        std::fs::create_dir_all(app.join("Contents/Frameworks")).map_err(|e| e.to_string())?;
        // ditto keeps the framework's symbolic links.
        sh("ditto", &[&dir.join("Sparkle.framework").to_string_lossy(), &framework.to_string_lossy()], None)?;
    }
    if let Some(profile) = o.store.as_ref().and_then(|s| s.profile.as_ref()) {
        std::fs::copy(profile, app.join("Contents/embedded.provisionprofile")).map_err(|e| format!("{}: {e}", profile.display()))?;
    }

    step(if o.sign == "-" { "Signing (ad hoc)" } else { "Signing" });
    let app_s = app.to_string_lossy().into_owned();
    // Nothing copied in keeps its extended attributes: a provisioning
    // profile or an icon downloaded with a browser carries a quarantine
    // mark, which the App Store refuses inside an app.
    let _ = Command::new("xattr").args(["-cr", &app_s]).status();
    let entitlements = out.join(format!("{}.entitlements", p.bin));
    let entitlements_s = entitlements.to_string_lossy().into_owned();
    // The hardened runtime is for Developer ID; the App Store's sandbox
    // takes its place there.
    let mut args = if o.store.is_some() { vec!["--force", "--sign", &o.sign] } else { vec!["--force", "--options", "runtime", "--sign", &o.sign] };
    if sparkle.is_some() {
        // Sparkle's own programs first, inside out (as Sparkle's docs sign
        // them), with the app's identity; the app's signature seals them.
        let versions = framework.join("Versions/B");
        let parts = [
            (versions.join("XPCServices/Installer.xpc"), false),
            (versions.join("XPCServices/Downloader.xpc"), true),
            (versions.join("Autoupdate"), false),
            (versions.join("Updater.app"), false),
            (framework.clone(), false),
        ];
        for (part, keep_entitlements) in parts {
            let part = part.to_string_lossy().into_owned();
            let mut a = vec!["--force", "--options", "runtime", "--sign", &o.sign];
            if keep_entitlements {
                a.push("--preserve-metadata=entitlements");
            }
            if o.sign != "-" {
                a.push("--timestamp");
            }
            a.push(&part);
            sh("codesign", &a, None)?;
        }
    }
    let mut trues = if p.sandbox { sandbox_entitlements(p.network) } else { vec![] };
    if sparkle.is_some() && o.sign == "-" {
        // Ad hoc code has no team, so library validation (part of the
        // hardened runtime) would refuse Sparkle; signed with a Developer
        // ID, the app and Sparkle share the team and need no exception.
        trues.push("com.apple.security.cs.disable-library-validation");
    }
    if !trues.is_empty() || !p.entitlements.is_empty() {
        let extra: Vec<(&str, &str)> = o.store.iter().flat_map(|s| s.entitlements.iter().map(|(k, v)| (k.as_str(), v.as_str()))).collect();
        // A sandboxed app reaches Sparkle's installer through these services.
        let services: Vec<String> = if sparkle.is_some() && p.sandbox { vec![format!("{}-spks", p.id), format!("{}-spki", p.id)] } else { vec![] };
        let arrays: Vec<(&str, &[String])> = if services.is_empty() { vec![] } else { vec![("com.apple.security.temporary-exception.mach-lookup.global-name", &services)] };
        // The app's own entitlements last (printing, a usage it declares).
        let own: String = p.entitlements.iter().map(|(k, v)| Extra::xml(k, v)).collect();
        let file = entitlements_plist(&trues, &extra, &arrays).replace("</dict>\n</plist>\n", &format!("{own}</dict>\n</plist>\n"));
        std::fs::write(&entitlements, file).map_err(|e| e.to_string())?;
        args.extend(["--entitlements", &entitlements_s]);
    }
    if o.sign != "-" {
        args.push("--timestamp");
    }
    args.push(&app_s);
    sh("codesign", &args, None)?;
    sh("codesign", &["--verify", "--strict", "--deep", &app_s], None)?;
    Ok(app)
}

/// The App Sandbox's entitlements: files the user chooses (open and save
/// panels, dropped files), app-scoped security bookmarks so Open Recent and
/// reopening at launch reach them again, and outgoing network if asked.
fn sandbox_entitlements(network: bool) -> Vec<&'static str> {
    let mut keys = vec!["com.apple.security.app-sandbox", "com.apple.security.files.user-selected.read-write", "com.apple.security.files.bookmarks.app-scope"];
    if network {
        keys.push("com.apple.security.network.client");
    }
    keys
}

/// An entitlements file: `trues` set to true, then strings and arrays.
fn entitlements_plist(trues: &[&str], strings: &[(&str, &str)], arrays: &[(&str, &[String])]) -> String {
    let mut body: String = trues.iter().map(|k| format!("    <key>{k}</key>\n    <true/>\n")).collect();
    body.extend(strings.iter().map(|(k, v)| format!("    <key>{}</key>\n    <string>{}</string>\n", xml(k), xml(v))));
    body.extend(arrays.iter().map(|(k, vs)| {
        let items: String = vs.iter().map(|v| format!("        <string>{}</string>\n", xml(v))).collect();
        format!("    <key>{}</key>\n    <array>\n{items}    </array>\n", xml(k))
    }));
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n{body}</dict>\n</plist>\n")
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn info_plist(p: &Project, icon: bool, build: Option<&str>, sparkle: bool) -> String {
    let key = |k: &str, v: &str| format!("  <key>{k}</key><string>{}</string>\n", xml(v));
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n",
    );
    s += &key("CFBundleDevelopmentRegion", "en");
    s += &key("CFBundleIdentifier", &p.id);
    s += &key("CFBundleName", &p.display);
    s += &key("CFBundleDisplayName", &p.display);
    s += &key("CFBundleExecutable", &p.bin);
    s += &key("CFBundleInfoDictionaryVersion", "6.0");
    s += &key("CFBundlePackageType", "APPL");
    s += &key("CFBundleShortVersionString", &p.version);
    s += &key("CFBundleVersion", build.unwrap_or(&p.version));
    s += &key("LSMinimumSystemVersion", &p.min_macos);
    s += &key("NSPrincipalClass", "NSApplication");
    s += "  <key>NSHighResolutionCapable</key><true/>\n  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>\n";
    if icon {
        s += &key("CFBundleIconFile", "AppIcon");
    }
    if let Some(c) = &p.category {
        s += &key("LSApplicationCategoryType", c);
    }
    if let Some(u) = p.updates.as_ref().filter(|_| sparkle) {
        // Sparkle's settings: where to look, and the key updates are signed with.
        s += &key("SUFeedURL", &u.feed);
        s += &key("SUPublicEDKey", &u.public_key);
        if p.sandbox {
            // Sandboxed, Sparkle installs (and downloads, without network
            // access) through its XPC services.
            s += "  <key>SUEnableInstallerLauncherService</key><true/>\n";
            if !p.network {
                s += "  <key>SUEnableDownloaderService</key><true/>\n";
            }
        }
    }
    if !p.languages.is_empty() {
        // Its languages, so macOS shows it (and AppKit's own panels and
        // menu items) in the user's language when it has it.
        let langs: String = std::iter::once("en").chain(p.languages.iter().map(String::as_str)).map(|l| format!("<string>{}</string>", xml(l))).collect();
        s += &format!("  <key>CFBundleLocalizations</key><array>{langs}</array>\n");
    }
    if !p.documents.is_empty() {
        // Editor role, Alternate rank: offered for these files, never made
        // their default without the user choosing it.
        s += "  <key>CFBundleDocumentTypes</key><array>\n";
        for d in &p.documents {
            let exts: String = d.extensions.iter().map(|e| format!("<string>{}</string>", xml(e))).collect();
            s += &format!(
                "    <dict><key>CFBundleTypeName</key><string>{}</string><key>CFBundleTypeRole</key><string>{}</string><key>LSHandlerRank</key><string>Alternate</string><key>CFBundleTypeExtensions</key><array>{exts}</array></dict>\n",
                xml(&d.name),
                xml(&d.role)
            );
            // Files beside a document that the app may open with it (a
            // database's journal): the sandbox lets the app reach them
            // through a file presenter (ferrokit's `related_files`).
            if !d.related.is_empty() {
                let exts: String = d.extensions.iter().flat_map(|e| d.related.iter().map(move |r| format!("<string>{}{}</string>", xml(e), xml(r)))).collect();
                s += &format!(
                    "    <dict><key>CFBundleTypeName</key><string>{} (related)</string><key>CFBundleTypeRole</key><string>Editor</string><key>LSHandlerRank</key><string>None</string><key>NSIsRelatedItemType</key><true/><key>CFBundleTypeExtensions</key><array>{exts}</array></dict>\n",
                    xml(&d.name)
                );
            }
        }
        s += "  </array>\n";
    }
    // The app's own keys last (a usage description, export compliance).
    for (k, v) in &p.plist {
        s += &Extra::xml(k, v);
    }
    s += "</dict></plist>\n";
    s
}

/// An .icns from a square PNG (or a copy of an .icns).
fn make_icon(icon: &Path, out: &Path) -> Result<()> {
    match icon.extension().and_then(|e| e.to_str()) {
        Some("icns") => std::fs::copy(icon, out).map(|_| ()).map_err(|e| format!("{}: {e}", icon.display())),
        Some("png") => {
            let set = std::env::temp_dir().join(format!("cargo-ferrokit-{}", std::process::id())).join("AppIcon.iconset");
            std::fs::create_dir_all(&set).map_err(|e| e.to_string())?;
            let src = icon.to_string_lossy().into_owned();
            for size in [16, 32, 128, 256, 512] {
                for (scale, suffix) in [(1, ""), (2, "@2x")] {
                    let px = (size * scale).to_string();
                    let dst = set.join(format!("icon_{size}x{size}{suffix}.png"));
                    output("sips", &["-z", &px, &px, &src, "--out", &dst.to_string_lossy()], None)?;
                }
            }
            let result = sh("iconutil", &["-c", "icns", &set.to_string_lossy(), "-o", &out.to_string_lossy()], None);
            let _ = std::fs::remove_dir_all(set.parent().unwrap());
            result
        }
        _ => Err(format!("the icon {} must be a .png or .icns", icon.display())),
    }
}

/// `<app>.zip` next to the app (ditto, as the Finder's Compress does).
fn zip(app: &Path) -> Result<PathBuf> {
    let zip = app.with_extension("zip");
    let _ = std::fs::remove_file(&zip);
    sh("ditto", &["-c", "-k", "--sequesterRsrc", "--keepParent", &app.to_string_lossy(), &zip.to_string_lossy()], None)?;
    Ok(zip)
}

// ---- the Mac App Store ----

/// What an App Store provisioning profile says: (application identifier,
/// team identifier), from its entitlements as JSON.
fn profile_ids(entitlements_json: &str) -> Result<(String, String)> {
    let e: serde_json::Value = serde_json::from_str(entitlements_json).map_err(|e| format!("the profile's entitlements: {e}"))?;
    let get = |keys: &[&str]| keys.iter().find_map(|k| e[*k].as_str().map(String::from));
    let app = get(&["com.apple.application-identifier", "application-identifier"]).ok_or("the profile names no application identifier")?;
    let team = get(&["com.apple.developer.team-identifier"]).or_else(|| app.split_once('.').map(|(t, _)| t.to_string())).ok_or("the profile names no team")?;
    Ok((app, team))
}

/// Reads a provisioning profile's identifiers (decoding it doesn't need
/// the keychain) and checks it's for this app.
fn read_profile(path: &Path, bundle_id: &str) -> Result<(String, String)> {
    let decoded = output("security", &["cms", "-D", "-i", &path.to_string_lossy()], None).map_err(|e| format!("{} isn't a provisioning profile: {e}", path.display()))?;
    let tmp = std::env::temp_dir().join(format!("cargo-ferrokit-profile-{}.plist", std::process::id()));
    std::fs::write(&tmp, decoded).map_err(|e| e.to_string())?;
    let json = output("plutil", &["-extract", "Entitlements", "json", "-o", "-", &tmp.to_string_lossy()], None);
    let platforms = output("plutil", &["-extract", "Platform", "json", "-o", "-", &tmp.to_string_lossy()], None).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    let (app, team) = profile_ids(&json?)?;
    if !platforms.is_empty() && !platforms.contains("OSX") {
        return Err(format!("{} is a profile for {}, not macOS: make a Mac App Store profile at developer.apple.com", path.display(), platforms.trim()));
    }
    if app != format!("{team}.{bundle_id}") {
        return Err(format!("{} is for {app}, but this app is {team}.{bundle_id}: make a profile for this bundle id (or fix `id`)", path.display()));
    }
    Ok((app, team))
}

/// `bundle --app-store`: a universal, sandboxed app with its profile, then
/// a signed installer package for App Store Connect.
fn app_store(p: &Project, build: Option<&str>, dry_run: bool, out: Option<PathBuf>) -> Result<()> {
    if !p.sandbox {
        return Err("the Mac App Store needs the App Sandbox: set `sandbox = true` in [package.metadata.ferrokit]".into());
    }
    if p.category.is_none() {
        return Err("the Mac App Store needs a category: set `category = \"public.app-category.…\"` in [package.metadata.ferrokit]".into());
    }
    if let Some(b) = build.filter(|b| b.is_empty() || !b.split('.').all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))) {
        return Err(format!("--build {b}: numbers and dots, as 42 or 1.2.42"));
    }
    let missing = |what: &str, key: &str| format!("the Mac App Store needs {what}: set `{key}` in [package.metadata.ferrokit]");
    let mut notes = Vec::new();
    let sign = match (&p.app_store_sign, dry_run) {
        (Some(s), false) => s.clone(),
        (None, false) => return Err(missing("an Apple Distribution identity", "app-store-sign")),
        (s, true) => {
            if s.is_none() {
                notes.push(missing("an Apple Distribution identity", "app-store-sign"));
            }
            "-".into()
        }
    };
    let installer = match (&p.installer_sign, dry_run) {
        (Some(s), _) => s.clone(),
        (None, false) => return Err(missing("an installer identity", "installer-sign")),
        (None, true) => {
            notes.push(missing("an installer identity", "installer-sign"));
            "<installer-sign>".into()
        }
    };
    let mut store = Store { build: build.map(String::from), ..Store::default() };
    match &p.provisioning_profile {
        Some(path) => {
            let (app, team) = read_profile(path, &p.id)?;
            store.entitlements = vec![("com.apple.application-identifier".into(), app), ("com.apple.developer.team-identifier".into(), team)];
            store.profile = Some(path.clone());
        }
        None if dry_run => notes.push(missing("a provisioning profile", "provisioning-profile")),
        None => return Err(missing("a provisioning profile", "provisioning-profile")),
    }
    let app = bundle(p, &BundleOptions { release: true, universal: true, sign: sign.clone(), out, store: Some(store) })?;
    let pkg = app.with_extension("pkg");
    let (app_s, pkg_s) = (app.to_string_lossy().into_owned(), pkg.to_string_lossy().into_owned());
    let productbuild = ["--component", app_s.as_str(), "/Applications", "--sign", installer.as_str(), pkg_s.as_str()];
    if dry_run {
        // The package itself, unsigned: what packaging would make.
        let _ = std::fs::remove_file(&pkg);
        sh_quiet("productbuild", &["--component", &app_s, "/Applications", &pkg_s])?;
        step("Dry run: signed ad hoc, package unsigned; with your identities it would run");
        eprintln!("    codesign --force --sign {:?} --entitlements … --timestamp {app_s:?}", p.app_store_sign.as_deref().unwrap_or("<app-store-sign>"));
        eprintln!("    productbuild {}", productbuild.map(|a| format!("{a:?}")).join(" "));
        for n in &notes {
            eprintln!("    note: {n}");
        }
        println!("{}", pkg.display());
        return Ok(());
    }
    step("Packaging for App Store Connect");
    let _ = std::fs::remove_file(&pkg);
    sh_quiet("productbuild", &productbuild)?;
    eprintln!("    upload it with Transporter (from the Mac App Store), or: xcrun altool --upload-package {pkg_s:?} …");
    println!("{}", pkg.display());
    Ok(())
}

// ---- run ----

fn run(args: &[String]) -> Result<()> {
    let a = Args::parse(args, &["--bin"], &["--release"])?;
    let project = Project::load(a.value("--bin"))?;
    let app = bundle(&project, &BundleOptions { release: a.flag("--release"), universal: false, sign: "-".into(), out: None, store: None })?;
    step(&format!("Running {}", project.display));
    // The bundle's own binary: macOS sees the app's name, icon and identity,
    // and its output stays in this terminal.
    let status = Command::new(app.join("Contents/MacOS").join(&project.bin)).args(&a.rest).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("{} exited with {status}", project.display)) }
}

// ---- preview ----

fn preview(args: &[String]) -> Result<()> {
    let a = Args::parse(args, &["--bin"], &["--release"])?;
    let project = Project::load(a.value("--bin"))?;
    let options = BundleOptions { release: a.flag("--release"), universal: false, sign: "-".into(), out: None, store: None };
    // FERROKIT_PREVIEW: windows keep their frames across restarts, and the
    // app doesn't take the focus at launch.
    let launch = |app: &Path| {
        Command::new(app.join("Contents/MacOS").join(&project.bin)).args(&a.rest).env("FERROKIT_PREVIEW", "1").spawn().map_err(|e| e.to_string())
    };
    let mut child = Some(launch(&bundle(&project, &options)?)?);
    step(&format!("Previewing {} (pid {}): save a file to rebuild and restart it; ⌃C stops", project.display, child.as_ref().map_or(0, |c| c.id())));
    let mut seen = sources(&project);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(c) = child.as_mut()
            && let Ok(Some(status)) = c.try_wait()
        {
            eprintln!("{} quit ({status}); save a file to start it again", project.display);
            child = None;
        }
        if sources(&project) == seen {
            continue;
        }
        // An editor can write a save in steps: let it finish.
        std::thread::sleep(Duration::from_millis(200));
        seen = sources(&project);
        step("Rebuilding");
        let built = bundle(&project, &options);
        // The build's own writes (Cargo.lock after a dependency change)
        // aren't a save; anything else that changed meanwhile still is.
        let lock = project.dir.join("Cargo.lock");
        match sources(&project).remove(&lock) {
            Some(written) => seen.insert(lock, written),
            None => seen.remove(&lock),
        };
        match built {
            Err(e) => eprintln!("\x1b[31mbuild failed\x1b[0m: {e}\n{} keeps running; fix it and save again", project.display),
            Ok(app) => {
                if let Some(mut old) = child.take() {
                    let _ = old.kill();
                    let _ = old.wait();
                }
                match launch(&app) {
                    Ok(c) => {
                        step(&format!("Restarted {} (pid {})", project.display, c.id()));
                        child = Some(c);
                    }
                    Err(e) => eprintln!("couldn't start {}: {e}", project.display),
                }
            }
        }
    }
}

/// The package's files and when each last changed, leaving out hidden
/// files and folders (version control), the build output and the
/// package's own `target` and `updates` folders (what `appcast` writes). A
/// folder of those names deeper in, such as `src/updates`, is watched.
fn sources(project: &Project) -> std::collections::BTreeMap<PathBuf, std::time::SystemTime> {
    fn walk(dir: &Path, skip: &[PathBuf], out: &mut std::collections::BTreeMap<PathBuf, std::time::SystemTime>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let hidden = name.to_string_lossy().starts_with('.');
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if !hidden && !skip.contains(&path) {
                    walk(&path, skip, out);
                }
            } else if !hidden && let Ok(modified) = meta.modified() {
                out.insert(path, modified);
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(&project.dir, &[project.target_dir.clone(), project.dir.join("target"), project.dir.join("updates")], &mut out);
    out
}

// ---- check ----

fn check(args: &[String]) -> Result<()> {
    let a = Args::parse(args, &["--bin", "--timeout"], &["--no-lint"])?;
    let project = Project::load(a.value("--bin"))?;
    let timeout = a.value("--timeout").map(|t| t.parse::<u64>().map_err(|_| format!("--timeout {t}: seconds"))).transpose()?.unwrap_or(180);
    let mut summary: Vec<(String, std::result::Result<String, String>)> = Vec::new();
    let mut record = |name: &str, r: std::result::Result<String, String>| {
        eprintln!("{} {name}{}", if r.is_ok() { "\x1b[32mok\x1b[0m  " } else { "\x1b[31mFAIL\x1b[0m" }, r.as_ref().map_or_else(|e| format!(": {e}"), |n| if n.is_empty() { String::new() } else { format!(": {n}") }));
        summary.push((name.into(), r));
    };
    let pkg = project.package.clone();
    if !a.flag("--no-lint") {
        step("Lint (clippy, warnings are errors)");
        record("clippy", sh(&cargo(), &["clippy", "-p", &pkg, "--all-targets", "--", "-D", "warnings"], Some(&project.dir)).map(|_| String::new()));
        step("Tests");
        record("tests", sh(&cargo(), &["test", "-p", &pkg], Some(&project.dir)).map(|_| String::new()));
    }
    let app = bundle(&project, &BundleOptions { release: false, universal: false, sign: "-".into(), out: None, store: None });
    match app {
        Err(e) => record("bundle", Err(e)),
        Ok(app) => {
            record("bundle", Ok(app.display().to_string()));
            step("Self-checks (CHECK=all)");
            record("self-checks", self_checks(&app.join("Contents/MacOS").join(&project.bin), Duration::from_secs(timeout)));
        }
    }
    let failed = summary.iter().filter(|(_, r)| r.is_err()).count();
    if failed == 0 {
        eprintln!("\x1b[1mall passed\x1b[0m");
        Ok(())
    } else {
        Err(format!("{failed} failed: {}", summary.iter().filter(|(_, r)| r.is_err()).map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")))
    }
}

/// What a self-check run reported on stderr.
#[derive(Default)]
struct Seen {
    pass: usize,
    skip: usize,
    fail: Vec<String>,
    done: bool,
    failure: Option<String>,
    last: Option<Instant>,
}

impl Seen {
    /// Takes one line of the app's stderr; true if it was a check's or a
    /// caught failure's line.
    fn read(&mut self, line: &str) -> bool {
        if line.starts_with("ferrokit: a panic in") || line.starts_with("ferrokit: an Objective-C exception in") || line.contains("panicked at") {
            // Ferrokit's own line says where; the panic's own line only
            // stands in until it comes.
            if line.starts_with("ferrokit:") || self.failure.is_none() {
                self.failure = Some(line.to_string());
            }
            self.last = Some(Instant::now());
            return true;
        }
        let Some(rest) = line.strip_prefix("[check] ") else { return false };
        self.last = Some(Instant::now());
        if rest.starts_with("done") {
            self.done = true;
            return true;
        }
        let words: Vec<&str> = rest.split_whitespace().collect();
        match words.iter().position(|w| matches!(*w, "PASS" | "FAIL" | "SKIP")) {
            Some(i) if words[i] == "PASS" => self.pass += 1,
            Some(i) if words[i] == "SKIP" => self.skip += 1,
            Some(i) => self.fail.push(words[..i].join(" ")),
            None => {}
        }
        true
    }
}

/// How a self-check run ended.
enum Ended {
    /// The app exited: whether with success, and the status as text.
    Exited(bool, String),
    /// Killed: no check line for 15 s after a caught failure.
    Stalled,
    /// Killed at the timeout.
    TimedOut(Duration),
}

/// The verdict on a run. It passes only if the app exited with success,
/// at least one check passed, none failed, and no panic or exception was
/// caught on the way (the app goes on after one, so its exit status and
/// its check lines alone don't show it).
fn verdict(ended: Ended, s: &Seen) -> std::result::Result<String, String> {
    let (pass, fail) = (s.pass, &s.fail);
    let skipped = if s.skip > 0 { format!(", {} skipped", s.skip) } else { String::new() };
    let failure = s.failure.as_ref().map(|f| format!("; it hit {f}")).unwrap_or_default();
    match ended {
        Ended::Stalled => Err(format!("stopped reporting checks after a failure ({pass} passed, {} failed so far){failure}", fail.len())),
        Ended::TimedOut(t) if pass + fail.len() == 0 => {
            Err(format!("the app ran {}s without reporting checks: run them when CHECK=all and exit (see the template's self_checks){failure}", t.as_secs()))
        }
        Ended::TimedOut(t) => Err(format!("timed out after {}s ({pass} passed so far){failure}", t.as_secs())),
        Ended::Exited(_, st) if !fail.is_empty() => Err(format!("{} of {} failed: {} ({st})", fail.len(), pass + fail.len(), fail.join(", "))),
        Ended::Exited(false, st) => Err(format!("{pass} passed, but the app exited with {st}{}{failure}", if s.done { "" } else { " before finishing" })),
        Ended::Exited(true, _) if s.failure.is_some() => Err(format!("{pass} passed{skipped}, but a panic or exception was caught on the way{failure}")),
        Ended::Exited(true, _) if pass == 0 => Err(format!(
            "the app exited without a passing check{skipped}: print `[check] <name> PASS` lines to stderr when CHECK=all (see the template's self_checks)"
        )),
        Ended::Exited(true, _) => Ok(format!("{pass} passed{skipped}")),
    }
}

/// Runs the app with CHECK=all and reads its `[check] name PASS|FAIL|SKIP
/// note` lines (a name may have spaces), until it exits or `timeout`. A
/// caught panic or exception followed by 15 s without a check line ends it
/// early: the app goes on after one, so it would otherwise wait out the
/// timeout.
fn self_checks(exe: &Path, timeout: Duration) -> std::result::Result<String, String> {
    let mut child = Command::new(exe).env("CHECK", "all").stderr(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let stderr = child.stderr.take().expect("piped");
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Seen::default()));
    let s2 = seen.clone();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
            if s2.lock().unwrap().read(&line) {
                eprintln!("    {line}");
            }
        }
    });
    let start = Instant::now();
    let ended = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break Ended::Exited(status.success(), status.to_string());
        }
        let quiet = {
            let s = seen.lock().unwrap();
            s.failure.is_some() && s.last.is_some_and(|t| t.elapsed() > Duration::from_secs(15))
        };
        if quiet || start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break if quiet { Ended::Stalled } else { Ended::TimedOut(timeout) };
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // The app's last lines, but not for longer than the timeout allows: a
    // helper the app started can hold its stderr open after it is gone.
    let drain = Instant::now() + timeout.saturating_sub(start.elapsed()).min(Duration::from_secs(2));
    while !reader.is_finished() && Instant::now() < drain {
        std::thread::sleep(Duration::from_millis(20));
    }
    let s = std::mem::take(&mut *seen.lock().unwrap());
    verdict(ended, &s)
}

// ---- appcast ----

/// `appcast`: the bundled app zipped into the updates folder, and the feed
/// written (or brought up to date) there by Sparkle's generate_appcast,
/// which signs each update with the private key.
fn appcast(args: &[String]) -> Result<()> {
    let a = Args::parse(args, &["--bin", "--key-file", "--app", "--out"], &[])?;
    let p = Project::load(a.value("--bin"))?;
    let u = p.updates.as_ref().ok_or("no updates set up: add [package.metadata.ferrokit.updates] with `feed`, `public-key` and `sparkle` (see cargo ferrokit --help)")?;
    let sparkle = u.sparkle_dir()?;
    let app = a.value("--app").map(PathBuf::from).unwrap_or_else(|| p.target_dir.join("release/bundle").join(format!("{}.app", p.display)));
    if !app.is_dir() {
        return Err(format!("{} isn't there: make it first (cargo ferrokit notarize, or bundle)", app.display()));
    }
    let plist = app.join("Contents/Info.plist").to_string_lossy().into_owned();
    let read = |key: &str| output("plutil", &["-extract", key, "raw", "-o", "-", &plist], None).map(|s| s.trim().to_string());
    let key = read("SUPublicEDKey").map_err(|_| format!("{} has no update settings: bundle it again (the updates settings are new)", app.display()))?;
    if key != u.public_key {
        return Err(format!("{} was bundled with another public key: bundle it again", app.display()));
    }
    if !app.join("Contents/Frameworks/Sparkle.framework").is_dir() {
        return Err(format!("{} has no Sparkle.framework: bundle it again", app.display()));
    }
    let version = read("CFBundleShortVersionString")?;
    let out = a.value("--out").map(PathBuf::from).unwrap_or_else(|| p.dir.join("updates"));
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let zip = out.join(format!("{} {version}.zip", p.display));
    if zip.exists() {
        return Err(format!("{} is already there: each update needs a higher version (`version` in Cargo.toml)", zip.display()));
    }
    let app_s = app.to_string_lossy().into_owned();
    let signature = Command::new("codesign").args(["-dv", &app_s]).output().map(|o| String::from_utf8_lossy(&o.stderr).into_owned()).unwrap_or_default();
    if signature.contains("Signature=adhoc") {
        eprintln!("    note: {} is signed ad hoc: other Macs refuse it; release with `cargo ferrokit notarize`", app.display());
    }

    step(&format!("Zipping {} {version}", p.display));
    sh("ditto", &["-c", "-k", "--sequesterRsrc", "--keepParent", &app_s, &zip.to_string_lossy()], None)?;

    step("Writing the feed");
    let feed = out.join(u.feed_name());
    let (prefix, feed_s, out_s) = (u.url_prefix(), feed.to_string_lossy().into_owned(), out.to_string_lossy().into_owned());
    let mut gen_args: Vec<&str> = Vec::new();
    if let Some(k) = a.value("--key-file") {
        gen_args.extend(["--ed-key-file", k]);
    }
    gen_args.extend(["--download-url-prefix", &prefix, "-o", &feed_s, &out_s]);
    if let Err(e) = sh(&sparkle.join("bin/generate_appcast").to_string_lossy(), &gen_args, None) {
        // A zip without a feed entry would be skipped next time: take it back.
        let _ = std::fs::remove_file(&zip);
        return Err(e);
    }
    eprintln!("    upload the new files in {} ({} {version}.zip, {} and any .delta files) to {prefix}", out.display(), p.display, u.feed_name());
    println!("{}", feed.display());
    Ok(())
}

// ---- notarize ----

fn notarize(args: &[String]) -> Result<()> {
    let a = Args::parse(args, &["--bin", "--profile", "--sign"], &["--dry-run", "--universal"])?;
    let project = Project::load(a.value("--bin"))?;
    let sign = a.value("--sign").map(String::from).or_else(|| project.sign.clone()).filter(|s| s != "-").ok_or(
        "notarizing needs a Developer ID: set `sign = \"Developer ID Application: …\"` in [package.metadata.ferrokit] or pass --sign",
    )?;
    let profile = a.value("--profile").map(String::from).or_else(|| project.notary_profile.clone()).ok_or(
        "notarizing needs a notary profile: make one with `xcrun notarytool store-credentials <name> --apple-id … --team-id …`, then set notary-profile in Cargo.toml or pass --profile",
    )?;
    let app = bundle(&project, &BundleOptions { release: true, universal: a.flag("--universal"), sign, out: None, store: None })?;
    let zip_path = zip(&app)?;
    let (zip_s, app_s) = (zip_path.to_string_lossy().into_owned(), app.to_string_lossy().into_owned());
    let submit = ["notarytool", "submit", &zip_s, "--keychain-profile", &profile, "--wait"];
    if a.flag("--dry-run") {
        step("Dry run: would now run");
        eprintln!("    xcrun {}\n    xcrun stapler staple {app_s}\n    ditto -c -k --sequesterRsrc --keepParent {app_s} {zip_s}\n    spctl -a -vv {app_s}", submit.join(" "));
        println!("{}", zip_path.display());
        return Ok(());
    }
    step("Notarizing (uploads to Apple and waits)");
    let result = output("xcrun", &submit, None)?;
    eprintln!("{}", result.lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"));
    if !result.contains("status: Accepted") {
        return Err(format!("not accepted; see: xcrun notarytool log <id> --keychain-profile {profile}"));
    }
    step("Stapling");
    sh("xcrun", &["stapler", "staple", &app_s], None)?;
    let zip_path = zip(&app)?;
    sh("spctl", &["-a", "-vv", &app_s], None)?;
    println!("{}", zip_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn seen(lines: &[&str]) -> Seen {
        let mut s = Seen::default();
        lines.iter().for_each(|l| {
            s.read(l);
        });
        s
    }

    #[test]
    fn a_run_without_a_passing_check_fails() {
        let ok = || Ended::Exited(true, "exit status: 0".into());
        assert!(verdict(ok(), &seen(&[])).is_err());
        assert!(verdict(ok(), &seen(&["[check] input SKIP no trust", "[check] done, 0 failed"])).is_err());
        assert_eq!(verdict(ok(), &seen(&["[check] adds a task PASS", "[check] input SKIP no trust"])), Ok("1 passed, 1 skipped".into()));
        assert!(verdict(ok(), &seen(&["[check] a PASS", "[check] b c FAIL why"])).unwrap_err().contains("b c"));
        assert!(verdict(Ended::Exited(false, "signal: 6".into()), &seen(&["[check] a PASS"])).is_err());
    }

    #[test]
    fn a_caught_panic_fails_a_run_that_exits_cleanly() {
        let s = seen(&["[check] a PASS", "ferrokit: a panic in a control's action: boom", "[check] done, 0 failed"]);
        let e = verdict(Ended::Exited(true, "exit status: 0".into()), &s).unwrap_err();
        assert!(e.contains("a panic in a control's action"), "{e}");
        let s = seen(&["[check] a PASS", "ferrokit: an Objective-C exception in a timer: NSRangeException"]);
        assert!(verdict(Ended::Exited(true, "exit status: 0".into()), &s).is_err());
    }

    /// A helper the app started keeps its stderr open: the run still ends
    /// within the timeout.
    #[test]
    fn a_helper_holding_stderr_does_not_outlast_the_timeout() {
        use std::os::unix::fs::PermissionsExt;
        let exe = std::env::temp_dir().join(format!("cargo-ferrokit-test-{}.sh", std::process::id()));
        std::fs::write(&exe, "#!/bin/sh\nsleep 5 &\necho '[check] a PASS' >&2\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let start = Instant::now();
        let r = self_checks(&exe, Duration::from_millis(500));
        let took = start.elapsed();
        let _ = std::fs::remove_file(&exe);
        assert!(took < Duration::from_secs(3), "took {took:?}");
        assert_eq!(r, Ok("1 passed".into()));
    }

    #[test]
    fn arguments() {
        let a = Args::parse(&s(&["app", "--bin", "x", "--zip", "--out=dist", "--", "-v"]), &["--bin", "--out"], &["--zip"]).unwrap();
        assert_eq!((a.positional.clone(), a.value("--bin"), a.value("--out"), a.flag("--zip"), a.rest.clone()), (s(&["app"]), Some("x"), Some("dist"), true, s(&["-v"])));
        assert!(Args::parse(&s(&["--nope"]), &[], &[]).is_err());
        assert!(Args::parse(&s(&["--bin"]), &["--bin"], &[]).is_err());
    }

    #[test]
    fn settings_are_checked() {
        let ok = serde_json::json!({ "name": "A", "sandbox": true, "documents": [{ "name": "Text", "extensions": ["txt"] }], "updates": { "feed": "https://x", "public-key": "k" }, "plist": { "ITSAppUsesNonExemptEncryption": false, "LSUIElement": true, "CFBundleURLSchemes": ["a"] } });
        assert_eq!(check_settings(&ok), Ok(()));
        assert_eq!(Extra::table(&ok, "plist").unwrap().len(), 3);
        assert!(Extra::table(&ok, "entitlements").unwrap().is_empty());
        // A quoted true, another spelling, a key in the wrong table.
        for wrong in [serde_json::json!({ "sandbox": "true" }), serde_json::json!({ "Sandbox": true }), serde_json::json!({ "min_macos": "15.0" }), serde_json::json!({ "documents": [{ "name": "Text", "sandbox": true }] }), serde_json::json!({ "updates": { "feed": "https://x", "public-key": "k", "sandbox": true } })] {
            assert!(check_settings(&wrong).is_err(), "{wrong}");
        }
        assert!(Extra::table(&serde_json::json!({ "plist": { "a": 3 } }), "plist").is_err());
        assert!(Args::parse(&s(&["--sign", "--zip"]), &["--sign"], &["--zip"]).is_err(), "an option isn't a value");
        assert!(Args::parse(&s(&["--sign", "-"]), &["--sign"], &[]).is_ok(), "ad hoc is");
    }

    #[test]
    fn plist() {
        let p = Project {
            package: "notes".into(),
            version: "1.2.0".into(),
            dir: PathBuf::new(),
            target_dir: PathBuf::new(),
            bin: "notes".into(),
            display: "Notes & More".into(),
            id: "com.example.notes".into(),
            icon: None,
            min_macos: "15.0".into(),
            category: None,
            documents: vec![DocType { name: "Text".into(), extensions: vec!["txt".into(), "md".into()], related: vec![], role: "Editor".into() }],
            languages: vec!["de".into()],
            sign: None,
            notary_profile: None,
            sandbox: false,
            network: false,
            app_store_sign: None,
            installer_sign: None,
            provisioning_profile: None,
            updates: None,
            entitlements: vec![("com.apple.security.print".into(), Extra::Bool(true))],
            plist: vec![("ITSAppUsesNonExemptEncryption".into(), Extra::Bool(false)), ("NSHumanReadableCopyright".into(), Extra::Text("© 2026 A & B".into()))],
        };
        let plist = info_plist(&p, true, None, false);
        assert!(info_plist(&p, true, Some("42"), false).contains("<key>CFBundleVersion</key><string>42</string>"));
        assert!(plist.contains("<key>CFBundleName</key><string>Notes &amp; More</string>"));
        assert!(plist.contains("<key>CFBundleShortVersionString</key><string>1.2.0</string>"));
        assert!(plist.contains("<array><string>txt</string><string>md</string></array>"));
        assert!(plist.contains("<key>CFBundleIconFile</key><string>AppIcon</string>"));
        assert!(plist.contains("<key>CFBundleLocalizations</key><array><string>en</string><string>de</string></array>"));
        assert!(!plist.contains("NSIsRelatedItemType"), "no related files declared");
        // The app's own keys, by type.
        assert!(plist.contains("<key>ITSAppUsesNonExemptEncryption</key>\n    <false/>"), "{plist}");
        assert!(plist.contains("<key>NSHumanReadableCopyright</key>\n    <string>© 2026 A &amp; B</string>"));
        assert_eq!(Extra::xml("com.apple.security.print", &p.entitlements[0].1), "    <key>com.apple.security.print</key>\n    <true/>\n");
        // Related files: their extensions, declared as related items.
        let mut p = p;
        p.documents = vec![
            DocType { name: "Database".into(), extensions: vec!["sqlite".into(), "db".into()], related: vec!["-wal".into(), "-journal".into()], role: "Editor".into() },
            DocType { name: "Data File".into(), extensions: vec!["csv".into()], related: vec![], role: "Viewer".into() },
        ];
        let plist = info_plist(&p, true, None, false);
        assert!(plist.contains("<key>CFBundleTypeName</key><string>Database (related)</string>"), "{plist}");
        // A viewer's role.
        assert!(plist.contains("<key>CFBundleTypeName</key><string>Data File</string><key>CFBundleTypeRole</key><string>Viewer</string>"), "{plist}");
        assert!(plist.contains("<key>NSIsRelatedItemType</key><true/><key>CFBundleTypeExtensions</key><array><string>sqlite-wal</string><string>sqlite-journal</string><string>db-wal</string><string>db-journal</string></array>"), "{plist}");
    }

    #[test]
    fn entitlement_files() {
        let plain = entitlements_plist(&sandbox_entitlements(false), &[], &[]);
        assert!(plain.contains("<key>com.apple.security.app-sandbox</key>\n    <true/>"));
        assert!(plain.contains("com.apple.security.files.bookmarks.app-scope") && !plain.contains("network.client"));
        assert!(entitlements_plist(&sandbox_entitlements(true), &[], &[]).contains("<key>com.apple.security.network.client</key>"));
        let store = entitlements_plist(&sandbox_entitlements(false), &[("com.apple.application-identifier", "TEAM.com.example.a")], &[]);
        assert!(store.contains("<key>com.apple.application-identifier</key>\n    <string>TEAM.com.example.a</string>"));
    }

    #[test]
    fn profile_identifiers() {
        let mac = r#"{"com.apple.application-identifier":"ABCDE12345.com.example.notes","com.apple.developer.team-identifier":"ABCDE12345","keychain-access-groups":["ABCDE12345.*"]}"#;
        assert_eq!(profile_ids(mac).unwrap(), ("ABCDE12345.com.example.notes".into(), "ABCDE12345".into()));
        let ios = r#"{"application-identifier":"ABCDE12345.com.example.notes"}"#;
        assert_eq!(profile_ids(ios).unwrap().1, "ABCDE12345");
        assert!(profile_ids("{}").is_err());
    }
}
