//! macOS: the desktop app as `Hondascan.app`, cross-built on Linux (release-mac.sh). The executables are in Contents/MacOS, the data in
//! Contents/Resources (hondascan::resource looks there), and the Qt the app needs inside the bundle: the frameworks it and its plugins load
//! (Contents/Frameworks, which the app's rpath `@executable_path/../Frameworks` names), the Cocoa platform plugin (Contents/PlugIns) and
//! the QML modules (Contents/Resources/qml), with a qt.conf saying so. Nothing here can run a Mac program, so the package is checked by
//! structure: every Mach-O has a slice for the target and a code signature, and every library it loads resolves inside the bundle or is
//! the system's.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{qmake_query, Res};

/// The Mach-O architecture name of a Rust target's first field.
pub fn arch_name(rust_arch: &str) -> &'static str { if rust_arch == "aarch64" { "arm64" } else { "x86_64" } }

fn objdump() -> String { std::env::var("LLVM_OBJDUMP").ok().filter(|o| !o.is_empty()).unwrap_or_else(|| "llvm-objdump".into()) }

/// `llvm-objdump --macho` of one architecture slice of `file`, as text.
fn macho(file: &Path, arch: &str, what: &str) -> Res<String> {
    let o = Command::new(objdump()).args(["--macho", what, &format!("--arch-name={arch}")]).arg(file).output()
        .map_err(|e| format!("cannot run {}: {e}", objdump()))?;
    if !o.status.success() { return Err(format!("{} --macho {what} {} failed: {}", objdump(), file.display(), String::from_utf8_lossy(&o.stderr).trim()).into()); }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The libraries a Mach-O file loads (`--dylibs-used`: a first line naming the file, then one `\t<install name> (compatibility ...)` each).
pub fn parse_dylibs(text: &str) -> Vec<String> {
    text.lines().skip(1).filter_map(|l| l.trim().split(" (compatibility").next()).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// `@rpath/QtGui.framework/Versions/A/QtGui` -> `QtGui`; anything else is not a framework loaded by rpath.
pub fn rpath_framework(install_name: &str) -> Option<&str> {
    install_name.strip_prefix("@rpath/")?.split_once(".framework/").map(|(name, _)| name)
}

fn is_system(install_name: &str) -> bool { install_name.starts_with("/usr/lib/") || install_name.starts_with("/System/") }

/// A Mach-O file or a universal one, by its first four bytes.
fn is_macho(p: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut magic)).is_ok()
        && matches!(u32::from_be_bytes(magic), 0xcafebabe | 0xfeedfacf | 0xcffaedfe)
}

/// Copy a tree keeping its symlinks (a framework is mostly symlinks into Versions/A), leaving out what `skip` names.
fn copy_tree(from: &Path, to: &Path, skip: &dyn Fn(&str) -> bool) -> Res<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        if skip(&name) { continue; }
        let (src, dest) = (e.path(), to.join(&name));
        let kind = e.file_type()?;
        if kind.is_symlink() { std::os::unix::fs::symlink(std::fs::read_link(&src)?, &dest)?; }
        else if kind.is_dir() { copy_tree(&src, &dest, skip)?; }
        else { std::fs::copy(&src, &dest).map_err(|err| format!("copying {}: {err}", src.display()))?; }
    }
    Ok(())
}

fn machos_in(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let (p, kind) = (e.path(), e.file_type());
        if kind.as_ref().is_ok_and(|k| k.is_symlink()) { continue; }
        if p.is_dir() { machos_in(&p, out); } else if is_macho(&p) { out.push(p); }
    }
}

/// Headers and debug symbols are left out of the bundle: they are for building and debugging, not running.
fn not_shipped(name: &str) -> bool { name == "Headers" || name.ends_with(".dSYM") || name.ends_with(".prl") }

pub fn info_plist(version: &str) -> String {
    format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>hondascan-gui</string>
    <key>CFBundleIdentifier</key><string>io.github.huhwhatbruh.hondascan</string>
    <key>CFBundleName</key><string>Hondascan</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>CFBundleVersion</key><string>{version}</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#)
}

/// Put the Qt the app needs into `app` (Hondascan.app), starting from its executable `exe`, for the architecture `arch` (arm64 or x86_64).
pub fn bundle_qt(app: &Path, exe: &Path, arch: &str, version: &str) -> Res<()> {
    let (libs, qml, plugins) = (qmake_query("QT_INSTALL_LIBS")?, qmake_query("QT_INSTALL_QML")?, qmake_query("QT_INSTALL_PLUGINS")?);
    let contents = app.join("Contents");
    for m in ["QtQml", "QtQuick"] {
        copy_tree(&qml.join(m), &contents.join("Resources/qml").join(m), &not_shipped)?;
    }
    copy_tree(&plugins.join("platforms"), &contents.join("PlugIns/platforms"), &|n: &str| not_shipped(n) || n != "libqcocoa.dylib")?;
    // the startup animation's frames are JPEG (assets/intro): Qt decodes them with its JPEG plugin
    copy_tree(&plugins.join("imageformats"), &contents.join("PlugIns/imageformats"), &|n: &str| not_shipped(n) || n != "libqjpeg.dylib")?;
    let mut queue = vec![exe.to_path_buf()];
    machos_in(&contents.join("Resources/qml"), &mut queue);
    machos_in(&contents.join("PlugIns"), &mut queue);
    let mut shipped: std::collections::BTreeSet<String> = Default::default();
    while let Some(file) = queue.pop() {
        for dep in parse_dylibs(&macho(&file, arch, "--dylibs-used")?) {
            if is_system(&dep) { continue; }
            let name = rpath_framework(&dep).ok_or_else(|| format!("{} loads {dep}, which is neither a Qt framework nor the system's", file.display()))?;
            if !shipped.insert(name.to_string()) { continue; }
            let from = libs.join(format!("{name}.framework"));
            if !from.is_dir() { return Err(format!("{} loads {dep}, and {} is not there", file.display(), from.display()).into()); }
            let to = contents.join("Frameworks").join(format!("{name}.framework"));
            copy_tree(&from, &to, &not_shipped)?;
            queue.push(to.join("Versions/A").join(name));
        }
    }
    std::fs::write(contents.join("Resources/qt.conf"), "[Paths]\nPlugins = PlugIns\nQml2Imports = Resources/qml\n")?;
    std::fs::write(contents.join("Info.plist"), info_plist(version))?;
    println!("Qt bundled: {} frameworks, Contents/Resources/qml, Contents/PlugIns", shipped.len());
    Ok(())
}

/// The bundle by structure, since a Mac program cannot run here: every Mach-O inside has an `arch` slice and a code signature (Apple
/// silicon starts nothing unsigned; the linker signs what it links, Qt's frameworks come signed), every framework it loads by rpath is in
/// Contents/Frameworks, and the app's rpath names that folder.
pub fn verify(app: &Path, arch: &str) -> Res<()> {
    let contents = app.join("Contents");
    let mut files = Vec::new();
    machos_in(&contents, &mut files);
    let mut problems = Vec::new();
    for f in &files {
        let headers = match macho(f, arch, "--private-headers") { Ok(h) => h, Err(e) => { problems.push(format!("no {arch} slice: {e}")); continue } };
        if !headers.contains("LC_CODE_SIGNATURE") { problems.push(format!("{} has no code signature", f.display())); }
        for dep in parse_dylibs(&macho(f, arch, "--dylibs-used")?) {
            if let Some(name) = rpath_framework(&dep) {
                if !contents.join("Frameworks").join(format!("{name}.framework/Versions/A")).join(name).is_file() {
                    problems.push(format!("{} loads {dep}, which is not in Contents/Frameworks", f.display()));
                }
            }
        }
    }
    let exe = contents.join("MacOS/hondascan-gui");
    if !macho(&exe, arch, "--private-headers")?.contains("@executable_path/../Frameworks") {
        problems.push("hondascan-gui has no rpath @executable_path/../Frameworks".into());
    }
    if !problems.is_empty() { return Err(format!("the macOS app is not complete:\n  {}", problems.join("\n  ")).into()); }
    println!("the macOS app checked by structure: {} Mach-O files, each with an {arch} slice and a signature, every Qt framework inside", files.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_libraries_a_mach_o_loads_are_read_and_qt_frameworks_are_told_apart() {
        let text = "/x/hondascan-gui:\n\t@rpath/QtGui.framework/Versions/A/QtGui (compatibility version 6.0.0, current version 6.8.3)\n\
                    \t/usr/lib/libc++.1.dylib (compatibility version 1.0.0, current version 2200.27.0)\n";
        let deps = parse_dylibs(text);
        assert_eq!(deps, ["@rpath/QtGui.framework/Versions/A/QtGui", "/usr/lib/libc++.1.dylib"]);
        assert_eq!(rpath_framework(&deps[0]), Some("QtGui"));
        assert_eq!(rpath_framework(&deps[1]), None);
        assert!(is_system(&deps[1]) && !is_system(&deps[0]));
        assert_eq!(rpath_framework("@loader_path/libfoo.dylib"), None);
    }

    #[test]
    fn the_info_plist_names_the_executable_and_the_version() {
        let p = info_plist("1.2.3");
        assert!(p.contains("<string>hondascan-gui</string>") && p.contains("<string>1.2.3</string>") && p.contains("<string>12.0</string>"));
    }
}
