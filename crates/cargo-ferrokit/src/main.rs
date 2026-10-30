//! `cargo ferrokit`: make, run, bundle, check and notarize native Mac apps
//! built with appkit-ui. Settings live in the app's Cargo.toml under
//! `[package.metadata.ferrokit]` (see the template `cargo ferrokit new`
//! writes).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

const USAGE: &str = "\
cargo ferrokit: native Mac apps with appkit-ui

USAGE:
    cargo ferrokit new <path> [--document] [--name <app name>] [--id <bundle id>]
                              [--appkit-ui-path <dir>]
        Makes a new app: a to-do list, or with --document a text document app.
        --appkit-ui-path uses a local checkout of appkit-ui instead of crates.io.

    cargo ferrokit run [--release] [--bin <name>] [-- <args>…]
        Builds, bundles (so the app has its name, icon and identity) and runs
        it, with its output in the terminal.

    cargo ferrokit bundle [--debug] [--bin <name>] [--universal] [--sign <identity>]
                          [--zip] [--out <dir>]
        Makes <name>.app (release by default), signed with the hardened
        runtime: ad hoc, or with `sign` from Cargo.toml or --sign.
        --universal builds for Apple silicon and Intel.

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
    languages = [\"de\", \"fr\"]     # its translations (see appkit-ui's App::translations)
    sign = \"Developer ID Application: …\"
    notary-profile = \"…\"       # xcrun notarytool store-credentials …
    [[package.metadata.ferrokit.documents]]
    name = \"Text\"
    extensions = [\"txt\", \"md\"]";

type Result<T> = std::result::Result<T, String>;

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
        Some("bundle") => bundle_command(rest),
        Some("check") => check(rest),
        Some("notarize") => notarize(rest),
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
                let value = it.next().ok_or_else(|| format!("{a} needs a value"))?;
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
    documents: Vec<(String, Vec<String>)>,
    /// Languages the app speaks besides English (its translations).
    languages: Vec<String>,
    sign: Option<String>,
    notary_profile: Option<String>,
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
        let text = |key: &str| m[key].as_str().map(String::from);
        let dir = dir_of(package);
        let documents = m["documents"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| {
                let exts = d["extensions"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(String::from)).collect();
                (d["name"].as_str().unwrap_or("Document").to_string(), exts)
            })
            .collect();
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
            notary_profile: text("notary-profile"),
            documents,
            package: name,
            dir,
            bin,
        })
    }
}

// ---- new ----

fn new(args: &[String]) -> Result<()> {
    let a = Args::parse(args, &["--name", "--id", "--appkit-ui-path"], &["--document"])?;
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
    let dep = match a.value("--appkit-ui-path") {
        Some(p) => {
            let p = PathBuf::from(p).canonicalize().map_err(|e| format!("--appkit-ui-path {p}: {e}"))?;
            if !p.join("Cargo.toml").exists() {
                return Err(format!("{} has no Cargo.toml (point it at crates/appkit-ui)", p.display()));
            }
            format!("{{ path = {:?} }}", p.to_string_lossy())
        }
        None => format!("\"{}\"", APPKIT_UI_VERSION),
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
    println!("Made {} ({}) in {}", display, if document { "a document app" } else { "an app" }, path.display());
    println!("Next:\n    cd {}\n    cargo ferrokit run      # run it\n    cargo ferrokit check    # lint, test and run its self-checks\n    cargo ferrokit bundle   # make {display}.app", path.display());
    Ok(())
}

/// The appkit-ui that new apps depend on.
const APPKIT_UI_VERSION: &str = "0.3";

// ---- bundle ----

struct BundleOptions {
    release: bool,
    universal: bool,
    sign: String,
    out: Option<PathBuf>,
}

fn bundle_command(args: &[String]) -> Result<()> {
    let a = Args::parse(args, &["--bin", "--sign", "--out"], &["--debug", "--universal", "--zip"])?;
    let project = Project::load(a.value("--bin"))?;
    let sign = a.value("--sign").map(String::from).or_else(|| project.sign.clone()).unwrap_or_else(|| "-".into());
    let app = bundle(&project, &BundleOptions { release: !a.flag("--debug"), universal: a.flag("--universal"), sign, out: a.value("--out").map(PathBuf::from) })?;
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
    std::fs::write(app.join("Contents/Info.plist"), info_plist(p, icon)).map_err(|e| e.to_string())?;
    sh("plutil", &["-lint", "-s", &app.join("Contents/Info.plist").to_string_lossy()], None)?;
    std::fs::write(app.join("Contents/PkgInfo"), "APPL????").map_err(|e| e.to_string())?;
    // An .lproj folder per language: how macOS learns the app is localised.
    for lang in std::iter::once("en").chain(p.languages.iter().map(String::as_str)).filter(|_| !p.languages.is_empty()) {
        let dir = resources.join(format!("{lang}.lproj"));
        std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join("InfoPlist.strings"), "")).map_err(|e| e.to_string())?;
    }

    step(if o.sign == "-" { "Signing (ad hoc)" } else { "Signing" });
    let app_s = app.to_string_lossy().into_owned();
    let mut args = vec!["--force", "--options", "runtime", "--sign", &o.sign];
    if o.sign != "-" {
        args.push("--timestamp");
    }
    args.push(&app_s);
    sh("codesign", &args, None)?;
    sh("codesign", &["--verify", "--strict", "--deep", &app_s], None)?;
    Ok(app)
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn info_plist(p: &Project, icon: bool) -> String {
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
    s += &key("CFBundleVersion", &p.version);
    s += &key("LSMinimumSystemVersion", &p.min_macos);
    s += &key("NSPrincipalClass", "NSApplication");
    s += "  <key>NSHighResolutionCapable</key><true/>\n  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>\n";
    if icon {
        s += &key("CFBundleIconFile", "AppIcon");
    }
    if let Some(c) = &p.category {
        s += &key("LSApplicationCategoryType", c);
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
        for (name, exts) in &p.documents {
            let exts: String = exts.iter().map(|e| format!("<string>{}</string>", xml(e))).collect();
            s += &format!(
                "    <dict><key>CFBundleTypeName</key><string>{}</string><key>CFBundleTypeRole</key><string>Editor</string><key>LSHandlerRank</key><string>Alternate</string><key>CFBundleTypeExtensions</key><array>{exts}</array></dict>\n",
                xml(name)
            );
        }
        s += "  </array>\n";
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

// ---- run ----

fn run(args: &[String]) -> Result<()> {
    let a = Args::parse(args, &["--bin"], &["--release"])?;
    let project = Project::load(a.value("--bin"))?;
    let app = bundle(&project, &BundleOptions { release: a.flag("--release"), universal: false, sign: "-".into(), out: None })?;
    step(&format!("Running {}", project.display));
    // The bundle's own binary: macOS sees the app's name, icon and identity,
    // and its output stays in this terminal.
    let status = Command::new(app.join("Contents/MacOS").join(&project.bin)).args(&a.rest).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("{} exited with {status}", project.display)) }
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
    let app = bundle(&project, &BundleOptions { release: false, universal: false, sign: "-".into(), out: None });
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

/// Runs the app with CHECK=all and reads its `[check] name PASS|FAIL note`
/// lines, until it exits or `timeout`.
fn self_checks(exe: &Path, timeout: Duration) -> std::result::Result<String, String> {
    let mut child = Command::new(exe).env("CHECK", "all").stderr(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let stderr = child.stderr.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let (mut pass, mut fail, mut done) = (0, Vec::new(), false);
        for line in BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
            let Some(rest) = line.strip_prefix("[check] ") else { continue };
            eprintln!("    {line}");
            if rest.starts_with("done") {
                done = true;
            } else if rest.split_whitespace().nth(1) == Some("PASS") {
                pass += 1;
            } else if rest.split_whitespace().nth(1) == Some("FAIL") {
                fail.push(rest.split_whitespace().next().unwrap_or_default().to_string());
            }
        }
        (pass, fail, done)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break Some(status);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let (pass, fail, done) = reader.join().unwrap_or_default();
    match status {
        None if pass + fail.len() == 0 => Err(format!(
            "the app ran {}s without reporting checks: run them when CHECK=all and exit (see the template's self_checks)",
            timeout.as_secs()
        )),
        None => Err(format!("timed out after {}s ({pass} passed so far)", timeout.as_secs())),
        Some(s) if !fail.is_empty() => Err(format!("{} of {} failed: {} ({s})", fail.len(), pass + fail.len(), fail.join(", "))),
        Some(s) if !s.success() => Err(format!("{pass} passed, but the app exited with {s}{}", if done { "" } else { " before finishing" })),
        Some(_) => Ok(format!("{pass} passed")),
    }
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
    let app = bundle(&project, &BundleOptions { release: true, universal: a.flag("--universal"), sign, out: None })?;
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

    #[test]
    fn arguments() {
        let a = Args::parse(&s(&["app", "--bin", "x", "--zip", "--out=dist", "--", "-v"]), &["--bin", "--out"], &["--zip"]).unwrap();
        assert_eq!((a.positional.clone(), a.value("--bin"), a.value("--out"), a.flag("--zip"), a.rest.clone()), (s(&["app"]), Some("x"), Some("dist"), true, s(&["-v"])));
        assert!(Args::parse(&s(&["--nope"]), &[], &[]).is_err());
        assert!(Args::parse(&s(&["--bin"]), &["--bin"], &[]).is_err());
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
            documents: vec![("Text".into(), vec!["txt".into(), "md".into()])],
            languages: vec!["de".into()],
            sign: None,
            notary_profile: None,
        };
        let plist = info_plist(&p, true);
        assert!(plist.contains("<key>CFBundleName</key><string>Notes &amp; More</string>"));
        assert!(plist.contains("<key>CFBundleShortVersionString</key><string>1.2.0</string>"));
        assert!(plist.contains("<array><string>txt</string><string>md</string></array>"));
        assert!(plist.contains("<key>CFBundleIconFile</key><string>AppIcon</string>"));
        assert!(plist.contains("<key>CFBundleLocalizations</key><array><string>en</string><string>de</string></array>"));
    }
}
