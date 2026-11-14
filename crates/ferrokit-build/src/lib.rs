//! Compiles a Mac app's Swift code from `build.rs` and links it into the
//! app, so Rust can call Swift (and SwiftUI) through `@_cdecl` functions.
//!
//! ```no_run
//! // build.rs
//! fn main() {
//!     ferrokit_build::swift("swift").compile();
//! }
//! ```
//!
//! ```toml
//! # Cargo.toml
//! [build-dependencies]
//! ferrokit-build = "0.8"
//! ```
//!
//! Every `.swift` file under the folder (here `swift/`, next to
//! `Cargo.toml`) is compiled by `swiftc` into one static library, which is
//! linked into the package's binaries. The Swift runtime, SwiftUI and the
//! rest of Apple's frameworks come with macOS, so nothing is copied into the
//! app; the frameworks the Swift code imports are linked automatically.
//! `swiftc` comes with the Command Line Tools (`xcode-select --install`),
//! which a Rust build on a Mac needs anyway; Xcode works too.
//!
//! On the Swift side, export functions with C names; on the Rust side,
//! declare them in an `extern "C"` block:
//!
//! ```swift
//! // swift/Greeting.swift
//! @_cdecl("greeting_length")
//! public func greetingLength(_ name: UnsafePointer<CChar>) -> Int {
//!     "Hello, \(String(cString: name))!".count
//! }
//! ```
//!
//! ```no_run
//! unsafe extern "C" {
//!     fn greeting_length(name: *const std::ffi::c_char) -> isize;
//! }
//! // SAFETY: a NUL-terminated string.
//! let n = unsafe { greeting_length(c"Ferrokit".as_ptr()) };
//! ```
//!
//! A SwiftUI view goes into a Ferrokit window through `ferrokit::native`: a
//! Swift function returns an `NSHostingView` (`Unmanaged.passRetained(view)
//! .toOpaque()`), and `native(|_| unsafe { Retained::from_raw(ptr.cast()) }
//! .unwrap())` wraps it. Ferrokit's guide, chapter 14, has the whole recipe.
//!
//! The settings follow the build: the target's architecture (so universal
//! builds work), optimisation and debug info from the Cargo profile, and the
//! minimum macOS from `MACOSX_DEPLOYMENT_TARGET`, else 15.0.
//!
//! Put the Swift code in the package that builds the app (its binary or
//! example): the link settings Cargo passes from a build script reach that
//! package's own binaries only.

#![warn(missing_docs)]

use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Swift code to compile: the `.swift` files under `dir` (relative to the
/// package's `Cargo.toml`). Set options, then call
/// [`compile`](Swift::compile).
pub fn swift(dir: impl AsRef<Path>) -> Swift {
    Swift { dir: dir.as_ref().to_path_buf(), module: None, min_macos: None, swift_version: None, flags: Vec::new() }
}

/// Swift code to compile into the package (see [`swift`]).
#[derive(Debug, Clone)]
pub struct Swift {
    dir: PathBuf,
    module: Option<String>,
    min_macos: Option<String>,
    swift_version: Option<String>,
    flags: Vec<String>,
}

/// Why the Swift code couldn't be compiled.
#[derive(Debug)]
pub enum Error {
    /// Not building for macOS.
    NotMacos(String),
    /// The folder has no `.swift` files (or doesn't exist).
    NoSources(PathBuf),
    /// `xcrun` or `swiftc` couldn't be run.
    NoToolchain(String),
    /// `swiftc` reported errors (they're above, in the build output).
    Compile,
    /// A Cargo variable a build script gets is missing (not run by Cargo).
    NotInBuildScript(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotMacos(os) => write!(f, "ferrokit-build compiles Swift for macOS only (the target's OS is {os})"),
            Error::NoSources(dir) => write!(f, "no .swift files in {}", dir.display()),
            Error::NoToolchain(why) => write!(f, "couldn't run swiftc ({why}); install the Command Line Tools with `xcode-select --install`, or Xcode"),
            Error::Compile => write!(f, "swiftc failed; its errors are above"),
            Error::NotInBuildScript(var) => write!(f, "{var} isn't set: call ferrokit-build from a build script (build.rs)"),
        }
    }
}

impl std::error::Error for Error {}

impl Swift {
    /// The Swift module's name, which Swift code in other modules would
    /// `import`. By default the package's name in UpperCamelCase
    /// (`my-app` → `MyApp`).
    pub fn module(mut self, name: &str) -> Self {
        self.module = Some(name.to_string());
        self
    }

    /// The oldest macOS the Swift code is compiled for, such as `"15.0"`.
    /// By default `MACOSX_DEPLOYMENT_TARGET`, else 15.0 (Ferrokit's
    /// minimum).
    pub fn min_macos(mut self, version: &str) -> Self {
        self.min_macos = Some(version.to_string());
        self
    }

    /// The Swift language mode (`"5"` or `"6"`); by default `swiftc`'s
    /// own.
    pub fn swift_version(mut self, version: &str) -> Self {
        self.swift_version = Some(version.to_string());
        self
    }

    /// Passes `flag` to `swiftc` as it is (`"-warnings-as-errors"`, `"-D"`
    /// and `"FEATURE"` as two flags…).
    pub fn flag(mut self, flag: &str) -> Self {
        self.flags.push(flag.to_string());
        self
    }

    /// Compiles the Swift code and links it into the package, or stops the
    /// build with what went wrong (`swiftc`'s errors are in the build
    /// output).
    pub fn compile(self) {
        if let Err(e) = self.try_compile() {
            panic!("{e}");
        }
    }

    /// [`compile`](Self::compile), returning the error instead of stopping
    /// the build. On success, the static library's path.
    pub fn try_compile(self) -> Result<PathBuf, Error> {
        let var = |name: &'static str| env::var(name).map_err(|_| Error::NotInBuildScript(name));
        let os = var("CARGO_CFG_TARGET_OS")?;
        if os != "macos" {
            return Err(Error::NotMacos(os));
        }
        let manifest = PathBuf::from(var("CARGO_MANIFEST_DIR")?);
        let out = PathBuf::from(var("OUT_DIR")?);
        let dir = manifest.join(&self.dir);
        let sources = sources(&dir);
        // Watched before anything can fail, so adding the first file reruns.
        println!("cargo:rerun-if-changed={}", dir.display());
        for s in &sources {
            println!("cargo:rerun-if-changed={}", s.display());
        }
        println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
        println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
        if sources.is_empty() {
            return Err(Error::NoSources(dir));
        }

        let arch = match var("CARGO_CFG_TARGET_ARCH")?.as_str() {
            "aarch64" => "arm64".to_string(),
            other => other.to_string(),
        };
        let min = self.min_macos.clone().or_else(|| env::var("MACOSX_DEPLOYMENT_TARGET").ok()).unwrap_or_else(|| "15.0".into());
        let module = self.module.clone().unwrap_or_else(|| module_name(&env::var("CARGO_PKG_NAME").unwrap_or_default()));
        let sdk = xcrun(&["--sdk", "macosx", "--show-sdk-path"])?;
        let swiftc = PathBuf::from(xcrun(&["--find", "swiftc"])?);
        let lib = out.join("libferrokit_swift.a");

        let mut cmd = Command::new(&swiftc);
        cmd.args(["-emit-library", "-static", "-parse-as-library"])
            .args(["-module-name", &module])
            .args(["-target", &format!("{arch}-apple-macos{min}")])
            .arg("-sdk")
            .arg(&sdk)
            .arg("-module-cache-path")
            .arg(out.join("swift-module-cache"));
        let opt = env::var("OPT_LEVEL").unwrap_or_default();
        cmd.arg(if opt == "0" { "-Onone" } else { "-O" });
        if !matches!(env::var("DEBUG").as_deref(), Ok("false" | "0" | "none") | Err(_)) {
            cmd.arg("-g");
        }
        if let Some(v) = &self.swift_version {
            cmd.args(["-swift-version", v]);
        }
        cmd.args(&self.flags).arg("-o").arg(&lib).args(&sources);
        let status = cmd.status().map_err(|e| Error::NoToolchain(format!("{}: {e}", swiftc.display())))?;
        if !status.success() {
            return Err(Error::Compile);
        }

        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static=ferrokit_swift");
        // The runtime's link stubs; the libraries themselves are in the OS.
        println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
        // Compatibility libraries for older targets, in the toolchain.
        if let Some(toolchain) = swiftc.parent().and_then(Path::parent) {
            println!("cargo:rustc-link-search=native={}", toolchain.join("lib/swift/macosx").display());
        }
        // Some runtime libraries (Swift Concurrency) are linked by @rpath.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        Ok(lib)
    }
}

/// `.swift` files under `dir`, sorted (so builds are repeatable).
fn sources(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "swift") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

fn xcrun(args: &[&str]) -> Result<String, Error> {
    let o = Command::new("xcrun").args(args).output().map_err(|e| Error::NoToolchain(format!("xcrun: {e}")))?;
    if !o.status.success() {
        return Err(Error::NoToolchain(format!("xcrun {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// A package name as a Swift module name: `my-app` → `MyApp`, `2d_tools` →
/// `_2dTools`.
fn module_name(package: &str) -> String {
    let mut name: String = package
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut cs = p.chars();
            cs.next().map(|c| c.to_ascii_uppercase().to_string() + cs.as_str()).unwrap_or_default()
        })
        .collect();
    if name.is_empty() {
        name = "App".into();
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_names() {
        assert_eq!(module_name("my-app"), "MyApp");
        assert_eq!(module_name("sql_browser"), "SqlBrowser");
        assert_eq!(module_name("2d-tools"), "_2dTools");
        assert_eq!(module_name("habits"), "Habits");
        assert_eq!(module_name(""), "App");
    }

    #[test]
    fn sources_are_found_in_subfolders_and_sorted() {
        let dir = env::temp_dir().join(format!("ferrokit-build-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("views")).unwrap();
        for f in ["b.swift", "a.swift", "notes.txt", "views/c.swift"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let names: Vec<String> = sources(&dir).iter().map(|p| p.strip_prefix(&dir).unwrap().display().to_string()).collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["a.swift", "b.swift", "views/c.swift"]);
    }

    #[test]
    fn outside_a_build_script_it_says_so() {
        // Cargo sets this for build scripts only.
        if env::var("CARGO_CFG_TARGET_OS").is_err() {
            assert!(matches!(swift("swift").try_compile(), Err(Error::NotInBuildScript(_))));
        }
    }
}
