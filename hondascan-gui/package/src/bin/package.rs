//! `cargo run --release --bin package`: build a folder (and an archive) you can copy to another computer: the release
//! binary, the `hondascan-rpc` backend and the `hondascan` CLI next to it (backend.rs looks there), the generated database
//! (encrypted — see below), the recorded-car profiles when the source tree has any (the public source has none) and the GD101
//! driver's source (hondascan::resource finds them next to the executables, in the same data/, profiles/, drivers/ layout as
//! the source tree), and on Linux the udev rule for
//! the GODIAG GD101. The GD101's J2534 mode needs a C compiler (cc, gcc or clang) on the target once, to build its driver
//! from drivers/gd101_j2534.c (hondascan/src/transport/j2534.rs, gd101_library).
//!
//! `data/hondascan.db.enc` ships instead of the plain `hondascan.db` the source tree builds: `encrypt-db` seals it
//! with ChaCha20-Poly1305 (hondascan::dbcrypt; release builds set `HONDASCAN_DB_KEY`, the public `hondascan/release-key.txt`),
//! which also makes a damaged copy fail to open instead of answering wrongly; `db.rs` decrypts it in memory when the packaged
//! binary opens it. The key is public: this is integrity, not secrecy. A checkout's own `cargo test` still
//! runs against the plain file `build-db` writes; only this packaging step touches the encrypted form.
//!
//! The folder also carries `VERSION` (`hondascan <version>`), `manifest.json` (name, version, target OS/arch, build time
//! and every shipped file with its size and SHA-256) and `SHA256SUMS` (every shipped file except itself, in `sha256sum`
//! format). The three Unix executables are mode 0755, and `<archive>.sha256` holds the archive's own hash. After the
//! archive is written the packaged `hondascan selfcheck` runs with a temporary `HONDASCAN_HOME`; a failed check fails the
//! package build.
//!
//! Writes dist/hondascan-<version>-<os>-<arch>/ and a .tar.gz (.zip on Windows, made by the system `tar`, which is bsdtar there).
//!
//! `PACKAGE_TARGET=x86_64-pc-windows-gnu cargo run --release --bin package` builds the Windows archive from a Linux machine (mingw-w64
//! as the linker, `zip` for the archive; the release container does this): the binaries come from `target/<triple>/release`, and the
//! packaged `hondascan.exe selfcheck` runs under `wine` when that is installed, else it is reported as not run.
use std::path::{Path, PathBuf};
use std::process::Command;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

#[path = "../macos.rs"]
mod macos;

fn run(cmd: &mut Command) -> Res<()> {
    println!("+ {cmd:?}");
    let st = cmd.status()?;
    if st.success() { Ok(()) } else { Err(format!("{cmd:?} failed: {st}").into()) }
}

/// `RUSTFLAGS` for a shipped binary: remaps this machine's source paths (this checkout, `~/.cargo`, `~/.rustup`) out of
/// panic messages and debug strings, drops the build-id, and sets
/// `hondascan_packaged` so `hondascan::resource`'s dev-source-tree fallback (a `CARGO_MANIFEST_DIR` string literal,
/// which the remap flags above cannot touch) compiles out instead of baking this machine's path into the binary.
/// Applied to every release build whose output is copied into the dist folder, not to `build-db`/`encrypt-db`
/// (dev-only, not shipped).
fn release_rustflags(repo_root: &Path, elf: bool) -> Res<String> {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).map_err(|_| "HOME (USERPROFILE on Windows) not set")?;
    // only ELF has a build-id: PE and Mach-O linkers have no such flag (mingw's ld has one, not worth testing)
    let build_id = if elf { " -C link-arg=-Wl,--build-id=none" } else { "" };
    Ok(format!(
        "--remap-path-prefix={}=. --remap-path-prefix={home}/.cargo=/c --remap-path-prefix={home}/.rustup=/r{build_id} --cfg hondascan_packaged",
        repo_root.display()
    ))
}

/// `std::fs::copy` with the paths in its error (the bare "Permission denied" it gives says nothing about which file).
fn copy_file(from: &Path, to: &Path) -> Res<()> {
    std::fs::copy(from, to).map(|_| ()).map_err(|e| format!("copying {} to {}: {e}", from.display(), to.display()).into())
}

fn copy_dir(from: &Path, to: &Path) -> Res<()> {
    std::fs::create_dir_all(to).map_err(|e| format!("creating {}: {e}", to.display()))?;
    for e in std::fs::read_dir(from).map_err(|e| format!("reading {}: {e}", from.display()))? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() { copy_dir(&e.path(), &dest)? } else { copy_file(&e.path(), &dest)?; }
    }
    Ok(())
}

/// One shipped file as `manifest.json` and `SHA256SUMS` list it: a dist-relative path with `/` separators, the size in
/// bytes and the lowercase hex SHA-256 of the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShippedFile {
    path: String,
    size: u64,
    sha256: String,
}

/// A relative path in the archive's portable form: `/` separators and no `.` or empty components.
fn normalize_path(path: &str) -> String {
    path.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".").collect::<Vec<_>>().join("/")
}

/// `path` relative to `root`, in the manifest's `/`-separated form.
fn relative_path(root: &Path, path: &Path) -> String {
    normalize_path(&path.strip_prefix(root).unwrap_or(path).to_string_lossy())
}

/// The manifest's file list: paths normalized, `exclude` dropped, and the rest sorted by path so two runs with the same
/// input produce the same order.
fn manifest_files(files: Vec<(String, Vec<u8>)>, exclude: &[&str]) -> Vec<ShippedFile> {
    let mut out: Vec<ShippedFile> = files
        .into_iter()
        .map(|(path, bytes)| ShippedFile { path: normalize_path(&path), size: bytes.len() as u64, sha256: sha256_hex(&bytes) })
        .filter(|f| !exclude.contains(&f.path.as_str()))
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Res<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let (path, kind) = (e.path(), e.file_type()?);
        // a symlink (a macOS framework is mostly Versions/Current links) points at a file listed in its own right
        if kind.is_symlink() { continue; }
        if kind.is_dir() { collect_files(root, &path, out)? } else { out.push((relative_path(root, &path), std::fs::read(&path)?)) }
    }
    Ok(())
}

/// Every regular file under `root`, recursively, read into memory and ready for `manifest_files`.
fn shipped_files(root: &Path, exclude: &[&str]) -> Res<Vec<ShippedFile>> {
    let mut raw = Vec::new();
    collect_files(root, root, &mut raw)?;
    Ok(manifest_files(raw, exclude))
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The complete `manifest.json` text.
fn manifest_json(version: &str, os: &str, arch: &str, built: u64, files: &[ShippedFile]) -> String {
    let entries: Vec<String> = files
        .iter()
        .map(|f| format!("{{\"path\":{},\"size\":{},\"sha256\":\"{}\"}}", json_string(&f.path), f.size, f.sha256))
        .collect();
    format!(
        "{{\"schema\":1,\"name\":\"hondascan\",\"version\":{},\"os\":{},\"arch\":{},\"built\":{},\"files\":[{}]}}",
        json_string(version),
        json_string(os),
        json_string(arch),
        built,
        entries.join(",")
    )
}

/// The complete `SHA256SUMS` text: one `<hex>  <path>` line per file, in the order given (which is sorted by path).
fn sums_text(files: &[ShippedFile]) -> String {
    files.iter().map(|f| format!("{}  {}\n", f.sha256, f.path)).collect()
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Write `VERSION`, `manifest.json` and `SHA256SUMS` into a dist folder whose copies are all in place. The manifest lists
/// every file except the metadata pair; `SHA256SUMS` lists everything except itself, which includes `manifest.json`.
fn write_metadata(out: &Path, version: &str, os: &str, arch: &str, built: u64) -> Res<()> {
    std::fs::write(out.join("VERSION"), format!("hondascan {version}\n"))?;
    let shipped = shipped_files(out, &["manifest.json", "SHA256SUMS"])?;
    std::fs::write(out.join("manifest.json"), manifest_json(version, os, arch, built, &shipped))?;
    let summed = shipped_files(out, &["SHA256SUMS"])?;
    std::fs::write(out.join("SHA256SUMS"), sums_text(&summed))?;
    Ok(())
}

/// SHA-256 (FIPS 180-4) of `data`, lowercase hex. Kept here, with std only, so packaging has no dependency on the
/// `hondascan` crate.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bits = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());

    let mut w = [0u32; 64];
    for block in msg.as_chunks::<64>().0 {
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// The Unix executables must be executable after a copy that may not have kept the mode; Windows has no mode.
#[cfg(unix)]
fn make_executable(path: &Path) -> Res<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Res<()> {
    Ok(())
}

/// Run the packaged CLI's offline `selfcheck` against a fresh, empty data directory, so the check proves the executables
/// and their bundled resources work on a machine that does not have the source tree.
fn verify_packaged_cli(out: &Path, windows: bool, cross: bool) -> Res<()> {
    let exe = if windows { "hondascan.exe" } else { "hondascan" };
    let cli = out.join(exe);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let home = std::env::temp_dir().join(format!("hondascan-package-check-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&home)?;

    let mut cmd = if cross { Command::new("wine") } else { Command::new(&cli) };
    if cross {
        cmd.arg(&cli).env("WINEPREFIX", home.join("wine")).env("WINEDEBUG", "-all");   // a fresh prefix of its own: never the caller's
    }
    cmd.arg("selfcheck").env("HONDASCAN_HOME", &home).current_dir(&home);
    // No other HONDASCAN_* override: the check is about what the package carries. HONDASCAN_DB is the one fallback the
    // task allows, for a layout where the database is not beside the CLI.
    for var in ["HONDASCAN_ADAPTER", "HONDASCAN_DB", "HONDASCAN_GD101_SRC", "HONDASCAN_J2534", "HONDASCAN_PROFILES"] {
        cmd.env_remove(var);
    }
    let db = out.join("data").join("hondascan.db");
    let db_enc = db.with_extension("db.enc");
    if !db.exists() && !db_enc.exists() {
        println!("no database beside the CLI; setting HONDASCAN_DB={}", db.display());
        cmd.env("HONDASCAN_DB", &db);
    }
    println!("+ {cmd:?}");

    let output = cmd.output();
    let _ = std::fs::remove_dir_all(&home);
    let output = output.map_err(|e| format!("could not run {}: {e}", cli.display()))?;
    if !output.status.success() {
        eprintln!("--- packaged `{} selfcheck` stdout ---\n{}", cli.display(), String::from_utf8_lossy(&output.stdout));
        eprintln!("--- packaged `{} selfcheck` stderr ---\n{}", cli.display(), String::from_utf8_lossy(&output.stderr));
        return Err(format!("packaged `{} selfcheck` failed: {}", cli.display(), output.status).into());
    }
    Ok(())
}

/// Words that mark the research README; the package README must not contain any of them.
/// Libraries the target is expected to have: the C and C++ runtimes, the graphics stack and the desktop's own libraries. Everything
/// else the app links (Qt, and what Qt needs: ICU, harfbuzz, and the xcb helper libraries of its X11 plugin, libxcb-icccm, -image,
/// -keysyms, -cursor, ... and libxkbcommon-x11, which many desktops lack) is shipped in lib/. The core libxcb and libxkbcommon stay the system's.
const SYSTEM_LIBS: &[&str] = &[
    "linux-vdso", "ld-linux", "libc.so", "libm.so", "libdl.so", "libpthread.so", "librt.so", "libutil.so", "libresolv.so", "libgcc_s", "libstdc++",
    "libGL", "libEGL", "libOpenGL", "libGLX", "libGLdispatch", "libdrm", "libgbm", "libX11", "libXau", "libXdmcp", "libXext", "libXrender", "libxcb.so",
    "libwayland", "libxkbcommon.so", "libfontconfig", "libfreetype", "libglib", "libgobject", "libgio", "libgmodule", "libgthread", "libpcre", "libffi",
    "libz.so", "libexpat", "libdbus", "libsystemd", "libcap", "libgcrypt", "libgpg-error", "liblz4", "liblzma", "libselinux", "libmount", "libblkid",
    "libuuid", "libudev", "libbsd", "libmd.so",
];

fn is_system_lib(name: &str) -> bool { SYSTEM_LIBS.iter().any(|p| name.starts_with(p)) }

/// What `ldd` says `file` needs: (name as the loader asks for it, where it resolves). A line that does not resolve is an error: the
/// package would ship a program that cannot start.
fn ldd(file: &Path) -> Res<Vec<(String, PathBuf)>> {
    let o = Command::new("ldd").arg(file).output().map_err(|e| format!("running ldd on {}: {e}", file.display()))?;
    parse_ldd(&String::from_utf8_lossy(&o.stdout)).map_err(|e| format!("{}: {e}", file.display()).into())
}

fn parse_ldd(text: &str) -> Result<Vec<(String, PathBuf)>, String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.contains("not found") { return Err(format!("a library is not found: {line}")); }
        if let Some((name, rest)) = line.split_once(" => ") {
            if let Some(path) = rest.split(" (").next().filter(|p| p.starts_with('/')) { // the loader is sometimes listed by absolute path (/lib64/ld-linux-x86-64.so.2): only the file name matters
                out.push((name.trim().rsplit('/').next().unwrap_or_default().to_string(), PathBuf::from(path))); }
        }
    }
    Ok(out)
}

fn qmake_query(key: &str) -> Res<PathBuf> {
    let qmake = std::env::var("QMAKE").ok().filter(|q| !q.is_empty()).unwrap_or_else(|| if Command::new("qmake6").arg("-v").output().is_ok() { "qmake6".into() } else { "qmake".into() });
    let o = Command::new(&qmake).args(["-query", key]).output().map_err(|e| format!("cannot run {qmake}: {e}"))?;
    let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if v.is_empty() { return Err(format!("{qmake} -query {key} said nothing").into()); }
    Ok(PathBuf::from(v))
}

fn shared_objects(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() { shared_objects(&p, out); } else if p.file_name().is_some_and(|n| n.to_string_lossy().contains(".so")) { out.push(p); }
    }
}

/// Linux: ship Qt next to the app, so the target needs no Qt: the QML modules the app imports (qml/), the platform plugins (plugins/),
/// every library the app, the plugins and the QML modules need except the system ones above (lib/), and `qt.conf` pointing at them.
/// The app is linked with RPATH `$ORIGIN/lib` (build.rs), which is also searched for what the plugins load.
fn bundle_qt(out: &Path, exe: &Path) -> Res<()> {
    let (qml, plugins) = (qmake_query("QT_INSTALL_QML")?, qmake_query("QT_INSTALL_PLUGINS")?);
    for m in ["QtQml", "QtQuick"] {
        copy_dir(&qml.join(m), &out.join("qml").join(m))?;
    }
    for p in ["platforms", "xcbglintegrations", "platforminputcontexts", "wayland-decoration-client", "wayland-graphics-integration-client", "wayland-shell-integration"] {
        if plugins.join(p).is_dir() { copy_dir(&plugins.join(p), &out.join("plugins").join(p))?; }
    }
    // the startup animation's frames are JPEG (assets/intro): Qt decodes them with its JPEG plugin, which nothing else here pulls in
    std::fs::create_dir_all(out.join("plugins/imageformats"))?;
    copy_file(&plugins.join("imageformats/libqjpeg.so"), &out.join("plugins/imageformats/libqjpeg.so"))?;
    let mut files = vec![exe.to_path_buf()];
    shared_objects(&out.join("qml"), &mut files);
    shared_objects(&out.join("plugins"), &mut files);
    let mut libs: std::collections::BTreeMap<String, PathBuf> = Default::default();
    for f in &files {
        for (name, path) in ldd(f)? {
            if !is_system_lib(&name) { libs.entry(name).or_insert(path); }
        }
    }
    std::fs::create_dir_all(out.join("lib"))?;
    for (name, path) in &libs {
        copy_file(path, &out.join("lib").join(name))?;
    }
    std::fs::write(out.join("qt.conf"), "[Paths]\nPrefix = .\nLibraries = lib\nPlugins = plugins\nQml2Imports = qml\n")?;
    println!("Qt bundled: {} libraries, qml/, plugins/", libs.len());
    Ok(())
}

/// DLLs every Windows machine has (and the api-ms-win-* / ext-ms-* API sets): never shipped. Anything else a shipped file imports must be
/// found in Qt's bin/ and is shipped, or the package fails.
const WINDOWS_SYSTEM_DLLS: &[&str] = &[
    "kernel32", "kernelbase", "ntdll", "bcryptprimitives", "user32", "gdi32", "advapi32", "shell32", "ole32", "oleaut32", "ws2_32", "winmm", "imm32", "shlwapi",
    "version", "dwmapi", "uxtheme", "d3d9", "d3d11", "d3d12", "dxgi", "d3dcompiler_47", "dcomp", "opengl32", "comdlg32", "setupapi", "netapi32",
    "userenv", "wtsapi32", "crypt32", "bcrypt", "msvcrt", "ucrtbase", "secur32", "iphlpapi", "dnsapi", "mpr", "authz", "winspool.drv",
    "comctl32", "cfgmgr32", "powrprof", "propsys", "shcore", "d2d1", "dwrite", "dxva2", "ncrypt", "winhttp", "wininet", "normaliz", "rpcrt4",
    "hid", "msimg32", "usp10", "wsock32", "mswsock", "dbghelp", "psapi", "synchronization", "runtimeobject",
];

fn is_windows_system_dll(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("api-ms-win-") || n.starts_with("ext-ms-")
        || WINDOWS_SYSTEM_DLLS.iter().any(|s| n == *s || n.strip_suffix(".dll") == Some(s))
}

/// The DLLs a PE file imports, as `objdump -p` lists them (`DLL Name: Qt6Core.dll`); `OBJDUMP` names the objdump (mingw's by default).
fn pe_imports(file: &Path) -> Res<Vec<String>> {
    let objdump = std::env::var("OBJDUMP").ok().filter(|o| !o.is_empty()).unwrap_or_else(|| "x86_64-w64-mingw32-objdump".into());
    let o = Command::new(&objdump).arg("-p").arg(file).output().map_err(|e| format!("cannot run {objdump}: {e}"))?;
    if !o.status.success() { return Err(format!("{objdump} -p {} failed", file.display()).into()); }
    Ok(parse_pe_imports(&String::from_utf8_lossy(&o.stdout)))
}

fn parse_pe_imports(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("DLL Name: ")).map(|s| s.trim().to_string()).collect()
}

fn dlls_in(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() { dlls_in(&p, out); } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll")) { out.push(p); }
    }
}

/// Windows: ship Qt next to the app. The QML modules (qml/) and the platform plugins (plugins/) as on Linux; every DLL the app and those
/// import, and what those import in turn, from the Windows Qt's bin/ (its DLLs and the MinGW runtime it was built with) beside the .exe,
/// where Windows looks first; `qt.conf` pointing at the rest. QMAKE is the cross qmake of a Windows Qt (release-cross.sh).
fn bundle_qt_windows(out: &Path, exe: &Path) -> Res<()> {
    let (qml, plugins, bins) = (qmake_query("QT_INSTALL_QML")?, qmake_query("QT_INSTALL_PLUGINS")?, qmake_query("QT_INSTALL_BINS")?);
    for m in ["QtQml", "QtQuick"] {
        copy_dir(&qml.join(m), &out.join("qml").join(m))?;
    }
    copy_dir(&plugins.join("platforms"), &out.join("plugins/platforms"))?;
    // the startup animation's frames are JPEG (assets/intro): Qt decodes them with its JPEG plugin, which nothing else here pulls in
    std::fs::create_dir_all(out.join("plugins/imageformats"))?;
    copy_file(&plugins.join("imageformats/qjpeg.dll"), &out.join("plugins/imageformats/qjpeg.dll"))?;
    let available: std::collections::BTreeMap<String, PathBuf> = std::fs::read_dir(&bins)?.flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll")))
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().to_ascii_lowercase(), p))).collect();
    let mut queue = vec![exe.to_path_buf()];
    dlls_in(&out.join("qml"), &mut queue);
    dlls_in(&out.join("plugins"), &mut queue);
    let mut shipped: std::collections::BTreeSet<String> = Default::default();
    while let Some(file) = queue.pop() {
        for name in pe_imports(&file)? {
            let key = name.to_ascii_lowercase();
            if is_windows_system_dll(&name) || shipped.contains(&key) { continue; }
            let from = available.get(&key)
                .ok_or_else(|| format!("{} imports {name}, which is neither in {} nor a Windows system DLL", file.display(), bins.display()))?;
            let to = out.join(from.file_name().ok_or("a DLL path has no file name")?);
            copy_file(from, &to)?;
            shipped.insert(key);
            queue.push(to);
        }
    }
    std::fs::write(out.join("qt.conf"), "[Paths]\nPrefix = .\nPlugins = plugins\nQml2Imports = qml\n")?;
    println!("Qt bundled: {} DLLs, qml/, plugins/", shipped.len());
    Ok(())
}

/// The packaged app starts on its own Qt, against the simulated car, with no display: it must load every QML file and save a picture of
/// its window. Libraries the loader took from outside the package (other than the system ones) fail the check.
fn verify_packaged_gui(out: &Path) -> Res<()> {
    let tmp = std::env::temp_dir().join(format!("hondascan-package-gui-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let shot = tmp.join("window.png");
    let log = tmp.join("stderr.log");
    let mut child = Command::new(out.join("hondascan-gui"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &tmp)
        .env("HONDASCAN_HOME", &tmp)
        .env("HONDASCAN_ADAPTER", "sim")
        .env("HONDASCAN_SHOT", &shot)
        .env("HONDASCAN_INTRO", "always")   // the startup animation plays too: its JPEG frames need the plugin shipped above
        .env("QT_QPA_PLATFORM", "offscreen")
        .env("LD_DEBUG", "libs")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&log)?)   // a file, not a pipe: the loader log is long and nobody reads a pipe while the app runs
        .spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    let status = loop {
        if let Some(s) = child.try_wait()? { break s; }
        if std::time::Instant::now() > deadline { let _ = child.kill(); return Err("the packaged app did not finish in 90 s".into()); }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let err = std::fs::read_to_string(&log).unwrap_or_default();
    let size = std::fs::metadata(&shot).map_or(0, |m| m.len());
    let _ = std::fs::remove_dir_all(&tmp);
    let outside = qt_from_outside(&err, out);
    if !outside.is_empty() {
        return Err(format!("the packaged app loaded Qt from outside the package (it must run on its own): {}", outside.join(", ")).into());
    }
    if let Some(l) = err.lines().find(|l| l.contains("intro: ")) {
        return Err(format!("the startup animation did not play: {l}").into());
    }
    if !status.success() || size < 1000 {
        return Err(format!("the packaged app did not start ({status}, picture {size} bytes): {}", err.trim()).into());
    }
    println!("the packaged app started on its own Qt and drew its window ({size} bytes)");
    Ok(())
}

/// The packaged Windows app starts under wine on the Qt beside it, against the simulated car, and saves a picture of its window: the
/// proof that every DLL, plugin and QML module it needs is in the package. Under `xvfb-run` when there is no display, so no window
/// opens on a desktop. (Windows looks for DLLs beside the .exe first and wine's PATH holds no Qt, so a DLL left out fails here.)
fn verify_packaged_gui_windows(out: &Path) -> Res<()> {
    let tmp = std::env::temp_dir().join(format!("hondascan-package-wingui-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let shot = tmp.join("window.png");
    let log = tmp.join("stderr.log");
    let headless = std::env::var_os("DISPLAY").is_none();
    let mut cmd = if headless { let mut c = Command::new("xvfb-run"); c.args(["-a", "wine"]); c } else { Command::new("wine") };
    let mut child = cmd.arg(out.join("hondascan-gui.exe"))
        .env("HONDASCAN_HOME", &tmp)
        .env("HONDASCAN_ADAPTER", "sim")
        .env("HONDASCAN_SHOT", &shot)
        .env("HONDASCAN_INTRO", "always")   // the startup animation plays too: its JPEG frames need the plugin shipped above
        .env("WINEDEBUG", "-all")
        .env("WINEPREFIX", tmp.join("wine"))   // a fresh prefix of its own: never the caller's wine setup
        // Qt's default on Windows is Direct3D 11. Under wine (its D3D11 over a software GL on xvfb) the Shapes' curve renderer draws
        // nothing: the dials' arcs, the car and its symbols were missing in the picture while all the rest was there. On OpenGL wine
        // draws everything, so the check runs on OpenGL; the packaged app keeps Qt's default. (Measured 2026-10-29, wine 9.0.)
        .env("QSG_RHI_BACKEND", "opengl")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&log)?)
        .spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);   // the first wine start creates its prefix
    let status = loop {
        if let Some(s) = child.try_wait()? { break s; }
        if std::time::Instant::now() > deadline { let _ = child.kill(); return Err("the packaged Windows app did not finish in 180 s under wine".into()); }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let err = std::fs::read_to_string(&log).unwrap_or_default();
    let size = std::fs::metadata(&shot).map_or(0, |m| m.len());
    if let (true, Some(name)) = (size >= 1000, out.file_name()) {   // beside the package (dist/<name>-window.png), for a look
        let _ = std::fs::copy(&shot, out.with_file_name(format!("{}-window.png", name.to_string_lossy())));
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if let Some(l) = err.lines().find(|l| l.contains("intro: ")) {
        return Err(format!("the startup animation did not play under wine: {l}").into());
    }
    if !status.success() || size < 1000 {
        return Err(format!("the packaged Windows app did not start under wine ({status}, picture {size} bytes): {}", err.trim()).into());
    }
    println!("the packaged Windows app started under wine on its own Qt and drew its window ({size} bytes)");
    Ok(())
}

/// The Qt libraries the loader initialised (`LD_DEBUG=libs` prints `calling init: <path>`) that are not inside the package.
fn qt_from_outside(loader_log: &str, package: &Path) -> Vec<String> {
    let inside = package.to_string_lossy();
    loader_log.lines().filter_map(|l| l.split_once("calling init: ").map(|(_, p)| p.trim()))
        .filter(|p| p.rsplit('/').next().is_some_and(|n| n.starts_with("libQt6")) && !p.starts_with(inside.as_ref())).map(String::from).collect()
}

const BANNED_IN_PACKAGE_README: &[&str] = &["release-key", "hds 3.105", "decompil", "installer media", "/home/", "umf", "vendor"];

fn main() -> Res<()> {
    // this crate (`package/`) builds the app crate one level up and writes dist/ there
    let app_dir = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))?;
    let here = app_dir.as_path();
    let krate = here.join("../hondascan");
    let db = krate.join("data/hondascan.db");
    let db_enc = db.with_extension("db.enc");
    // PACKAGE_TARGET: a target triple to cross-build for (the Windows archive is made on Linux)
    let target = std::env::var("PACKAGE_TARGET").ok().filter(|t| !t.is_empty());
    let windows = target.as_deref().map_or(cfg!(windows), |t| t.contains("windows"));
    let mac = target.as_deref().map_or(cfg!(target_os = "macos"), |t| t.contains("apple-darwin"));
    let cross = target.is_some();
    let exe = if windows { ".exe" } else { "" };
    let release_dir = target.as_deref().map_or("target/release".to_string(), |t| format!("target/{t}/release"));
    let target_args: Vec<&str> = target.as_deref().map_or(vec![], |t| vec!["--target", t]);
    // the desktop app needs a Qt for the target; Qt 6 has no 32-bit Windows build, so that archive carries the command line only
    let gui = !(windows && target.as_deref().is_some_and(|t| t.starts_with("i686")));
    if !db.exists() {                                            // generated, never tracked
        run(Command::new("cargo").args(["run", "--release", "--bin", "build-db"]).current_dir(&krate))?;
    }
    let repo_root = here.parent().unwrap_or(here);
    let mut rustflags = release_rustflags(repo_root, !windows && !mac)?;
    if mac {
        rustflags.push_str(" -C link-arg=-Wl,-rpath,@executable_path/../Frameworks");   // the app's Qt is in Contents/Frameworks
        // ld64.lld, named here: qt-build-utils otherwise looks at the build machine's own `ld` and, finding GNU ld on Linux, adds
        // -fuse-ld=gold, which cannot link for macOS
        rustflags.push_str(" -C link-arg=-fuse-ld=lld");
    }
    if gui {
        let mut build = Command::new("cargo");
        build.args(["build", "--release"]).args(&target_args).current_dir(here).env("RUSTFLAGS", &rustflags);
        if windows {
            // The app's C++ runtime linked in, not libstdc++-6.dll: Rust links GCC's runtime statically, so its emulated TLS is the .exe's
            // own copy, and `std::call_once` stored its callable there while the DLL's __once_proxy read the DLL's copy and called null
            // (seen under wine). Qt's DLLs keep the libstdc++-6.dll they were built with; no C++ library type crosses between the two.
            let cxx = std::env::var("CXX").ok().filter(|c| !c.is_empty()).unwrap_or_else(|| "x86_64-w64-mingw32-g++".into());
            let o = Command::new(&cxx).arg("-print-file-name=libstdc++.a").output().map_err(|e| format!("cannot run {cxx}: {e}"))?;
            let lib = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
            let dir = lib.parent().filter(|_| lib.is_absolute()).ok_or_else(|| format!("{cxx} does not know where libstdc++.a is"))?;
            build.env("CXXSTDLIB", "static=stdc++").env("RUSTFLAGS", format!("{rustflags} -L native={}", dir.display()));
        }
        run(&mut build)?;
    }
    run(Command::new("cargo").args(["build", "--release", "--bins"]).args(&target_args).current_dir(&krate).env("RUSTFLAGS", &rustflags))?;
    // Ship the encrypted form, not the plain SQLite file: `hondascan.db` is readable with `sqlite3` as-is, and it
    // carries the decoded HDS tables, which is the data this package exists to not hand out for free. `db.rs`
    // decrypts it in memory on open (`dbcrypt`); re-encrypt every time, so a stale `.enc` from an older `hondascan.db`
    // never ships silently.
    run(Command::new("cargo").args(["run", "--release", "--bin", "encrypt-db"]).current_dir(&krate))?;

    let target_os = if cross { if windows { "windows" } else if mac { "macos" } else { "linux" } } else { std::env::consts::OS };
    let arch = target.as_deref().and_then(|t| t.split('-').next()).unwrap_or(std::env::consts::ARCH);
    let os = if target_os == "macos" { "darwin" } else { target_os };   // the folder name, as before
    let version = env!("CARGO_PKG_VERSION");
    let name = format!("hondascan-{version}-{os}-{arch}");
    let dist = here.join("dist");
    let out = dist.join(&name);
    let _ = std::fs::remove_dir_all(&out);
    // where the executables and their data go: the archive's top folder, or on macOS inside Hondascan.app (macos.rs)
    let app = out.join("Hondascan.app");
    let (bin_dir, res_dir) = if mac { (app.join("Contents/MacOS"), app.join("Contents/Resources")) } else { (out.clone(), out.clone()) };
    std::fs::create_dir_all(&bin_dir)?;
    std::fs::create_dir_all(res_dir.join("data"))?;
    std::fs::create_dir_all(res_dir.join("drivers"))?;
    let mut binaries = vec![format!("hondascan-rpc{exe}"), format!("hondascan{exe}")];
    if gui {
        binaries.insert(0, format!("hondascan-gui{exe}"));
        std::fs::copy(here.join(format!("{release_dir}/hondascan-gui{exe}")), bin_dir.join(&binaries[0]))?;
    }
    for b in ["hondascan-rpc", "hondascan"] {
        std::fs::copy(krate.join(format!("{release_dir}/{b}{exe}")), bin_dir.join(format!("{b}{exe}")))?;
    }
    std::fs::copy(&db_enc, res_dir.join("data/hondascan.db.enc"))?;
    if krate.join("profiles").is_dir() {
        copy_dir(&krate.join("profiles"), &res_dir.join("profiles"))?; // recorded cars; the public source has none
    }
    std::fs::copy(krate.join("drivers/gd101_j2534.c"), res_dir.join("drivers/gd101_j2534.c"))?;
    // The archive carries its own short user README, never the repo's README.md (the research notes, which name vendor
    // material and private paths and are not for the public release).
    let readme = std::fs::read_to_string(here.join("assets/PACKAGE-README.md"))?;
    if let Some(word) = BANNED_IN_PACKAGE_README.iter().find(|w| readme.to_lowercase().contains(**w)) {
        return Err(format!("assets/PACKAGE-README.md mentions {word:?}, which must not ship").into());
    }
    std::fs::write(out.join("README.md"), readme)?;
    std::fs::copy(here.join("assets/fonts/OFL-Noto.txt"), out.join("OFL-Noto.txt"))?;
    let rules = krate.join("packaging/60-hondascan.rules");
    if target_os == "linux" && rules.exists() {
        std::fs::copy(rules, out.join("60-hondascan.rules"))?;
    }
    if !windows {
        for b in &binaries {
            make_executable(&bin_dir.join(b))?;
        }
    }
    if target_os == "linux" && !cross {
        bundle_qt(&out, &out.join(&binaries[0]))?;
    }
    if windows && gui {
        bundle_qt_windows(&out, &out.join(&binaries[0]))?;
    }
    if mac {
        macos::bundle_qt(&app, &bin_dir.join(&binaries[0]), macos::arch_name(arch), version)?;
    }

    write_metadata(&out, version, target_os, arch, unix_seconds())?;

    let archive = dist.join(format!("{name}{}", if windows { ".zip" } else { ".tar.gz" }));
    if windows && !cfg!(windows) {
        let _ = std::fs::remove_file(&archive);
        run(Command::new("zip").arg("-qr").arg(&archive).arg(&name).current_dir(&dist))?;   // GNU tar cannot write a zip
    } else {
        let flags = if windows { "-acf" } else { "-czf" };
        run(Command::new("tar").arg(flags).arg(&archive).arg("-C").arg(&dist).arg(&name))?;
    }
    let archive_name = archive.file_name().ok_or("archive path has no file name")?.to_string_lossy().into_owned();
    let archive_sum = sha256_hex(&std::fs::read(&archive)?);
    let sum_path: PathBuf = dist.join(format!("{archive_name}.sha256"));
    std::fs::write(&sum_path, format!("{archive_sum}  {archive_name}\n"))?;

    if target_os == "linux" && !cross {
        verify_packaged_gui(&out)?;
    }
    let wine = Command::new("wine").arg("--version").output().is_ok_and(|o| o.status.success());
    if windows && gui && wine {
        verify_packaged_gui_windows(&out)?;
    } else if windows && gui {
        println!("the packaged desktop app was not started: wine is not installed");
    }
    if mac {
        macos::verify(&app, macos::arch_name(arch))?;
        println!("the packaged selfcheck was not run: a macOS program cannot run here");
    } else if cross && !wine {
        println!("the packaged selfcheck was not run: {} is not a native executable and wine is not installed", out.display());
    } else {
        verify_packaged_cli(&out, windows, cross)?;
    }

    println!("{}", out.display());
    println!("{} ({:.1} MB)", archive.display(), std::fs::metadata(&archive)?.len() as f64 / 1e6);
    println!("{archive_sum}  {archive_name}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("hondascan-package-{tag}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn package_readme_is_clean() {
        let readme = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/PACKAGE-README.md")).unwrap().to_lowercase();
        for w in BANNED_IN_PACKAGE_README {
            assert!(!readme.contains(w), "package README mentions {w:?}");
        }
    }

    #[test]
    fn ldd_lines_become_names_and_paths_and_a_missing_library_is_an_error() {
        let ok = "\tlinux-vdso.so.1 (0x00007ffd)\n\tlibQt6Core.so.6 => /usr/lib/libQt6Core.so.6 (0x00007f00)\n\t/lib64/ld-linux-x86-64.so.2 (0x00007f01)\n";
        assert_eq!(parse_ldd(ok).unwrap(), vec![("libQt6Core.so.6".to_string(), PathBuf::from("/usr/lib/libQt6Core.so.6"))]);
        assert!(parse_ldd("\tlibfoo.so.1 => not found\n").is_err(), "a package that cannot start is not built");
        let abs = "\t/lib64/ld-linux-x86-64.so.2 => /usr/lib64/ld-linux-x86-64.so.2 (0x00007f01)\n";
        let listed = parse_ldd(abs).unwrap();
        assert_eq!(listed[0].0, "ld-linux-x86-64.so.2", "named by file name, so it is a system library and never a path to write to");
        assert!(is_system_lib(&listed[0].0));
    }

    #[test]
    fn a_qt_library_from_outside_the_package_is_found_in_the_loader_log() {
        let log = "  1: calling init: /opt/p/lib/libQt6Core.so.6\n  1: calling init: /usr/lib/libQt6Quick.so.6\n  1: calling init: /usr/lib/libz.so.1\n  1: calling init: /opt/p/lib/libicuuc.so.78\n";
        assert_eq!(qt_from_outside(log, Path::new("/opt/p")), vec!["/usr/lib/libQt6Quick.so.6".to_string()], "only Qt, only from outside");
        assert!(qt_from_outside("calling init: /opt/p/lib/libQt6Gui.so.6", Path::new("/opt/p")).is_empty());
    }

    #[test]
    fn pe_imports_are_read_from_objdump_and_system_dlls_are_not_shipped() {
        let text = "The Import Tables (interpreted .idata section contents)\n\tDLL Name: Qt6Core.dll\n\tvma:  Hint/Ord Member-Name\n\
                    \tDLL Name: KERNEL32.dll\n\tDLL Name: api-ms-win-crt-runtime-l1-1-0.dll\n\tDLL Name: libstdc++-6.dll\n";
        assert_eq!(parse_pe_imports(text), ["Qt6Core.dll", "KERNEL32.dll", "api-ms-win-crt-runtime-l1-1-0.dll", "libstdc++-6.dll"]);
        assert!(is_windows_system_dll("KERNEL32.dll") && is_windows_system_dll("api-ms-win-crt-runtime-l1-1-0.dll") && is_windows_system_dll("d3d11.dll"));
        assert!(!is_windows_system_dll("Qt6Core.dll") && !is_windows_system_dll("libstdc++-6.dll") && !is_windows_system_dll("libwinpthread-1.dll"));
    }

    #[test]
    fn the_c_runtime_and_the_graphics_stack_stay_with_the_target_and_qt_goes_with_the_app() {
        for system in ["libc.so.6", "libstdc++.so.6", "libGL.so.1", "libxcb.so.1", "libX11.so.6", "libfontconfig.so.1", "ld-linux-x86-64.so.2", "libxkbcommon.so.0"] { assert!(is_system_lib(system), "{system}"); }
        for ours in ["libQt6Core.so.6", "libQt6Quick.so.6", "libicuuc.so.78", "libharfbuzz.so.0", "libdouble-conversion.so.3",
                     "libxcb-icccm.so.4", "libxcb-cursor.so.0", "libxcb-keysyms.so.1", "libxkbcommon-x11.so.0"] { assert!(!is_system_lib(ours), "{ours}"); }
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        // 56 bytes: the padding spills into a second 64-byte block.
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn paths_are_normalized_for_the_manifest() {
        assert_eq!(normalize_path("./profiles\\civic.json"), "profiles/civic.json");
        assert_eq!(normalize_path("data//hondascan.db"), "data/hondascan.db");
        assert_eq!(normalize_path(""), "");
        let root = Path::new("dist").join("hondascan-0.1.0-linux-x86_64");
        assert_eq!(relative_path(&root, &root.join("profiles").join("civic.json")), "profiles/civic.json");
    }

    #[test]
    fn manifest_entries_are_sorted_with_sizes_and_hashes() {
        let files = manifest_files(
            vec![
                ("profiles\\civic.json".to_string(), b"abc".to_vec()),
                ("./VERSION".to_string(), b"hondascan 0.1.0\n".to_vec()),
                ("manifest.json".to_string(), b"{}".to_vec()),
            ],
            &["manifest.json", "SHA256SUMS"],
        );
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["VERSION", "profiles/civic.json"]);
        assert_eq!(files[0].size, 16);
        assert_eq!(files[1].size, 3);
        assert_eq!(files[1].sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn shipped_files_walks_recursively_and_excludes_by_name() {
        let root = temp_root("walk");
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(root.join("VERSION"), b"hondascan 0.1.0\n").unwrap();
        std::fs::write(root.join("data").join("hondascan.db"), b"abc").unwrap();
        std::fs::write(root.join("SHA256SUMS"), b"self\n").unwrap();
        let files = shipped_files(&root, &["SHA256SUMS"]).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["VERSION", "data/hondascan.db"]);
        assert_eq!(files[1].size, 3);
        assert_eq!(files[1].sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn metadata_lists_the_files_but_never_itself() {
        let root = temp_root("metadata");
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(root.join("data").join("hondascan.db"), b"abc").unwrap();
        write_metadata(&root, "0.1.0", "linux", "x86_64", 1_700_000_000).unwrap();
        let manifest = std::fs::read_to_string(root.join("manifest.json")).unwrap();
        let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(manifest.contains("\"path\":\"VERSION\""));
        assert!(manifest.contains("\"path\":\"data/hondascan.db\""));
        assert!(!manifest.contains("\"path\":\"manifest.json\""));
        assert!(!manifest.contains("\"path\":\"SHA256SUMS\""));
        assert!(sums.contains(&format!("{}  VERSION\n", sha256_hex(b"hondascan 0.1.0\n"))));
        assert!(sums.contains(&format!("{}  data/hondascan.db\n", sha256_hex(b"abc"))));
        assert!(sums.contains(&format!("{}  manifest.json\n", sha256_hex(manifest.as_bytes()))));
        assert!(!sums.lines().any(|l| l.ends_with("  SHA256SUMS")));
    }

    #[test]
    fn manifest_json_matches_the_documented_shape() {
        let files = vec![ShippedFile { path: "VERSION".to_string(), size: 16, sha256: "ab".to_string() }];
        assert_eq!(
            manifest_json("0.1.0", "linux", "x86_64", 1_700_000_000, &files),
            "{\"schema\":1,\"name\":\"hondascan\",\"version\":\"0.1.0\",\"os\":\"linux\",\"arch\":\"x86_64\",\
             \"built\":1700000000,\"files\":[{\"path\":\"VERSION\",\"size\":16,\"sha256\":\"ab\"}]}"
        );
    }

    #[test]
    fn sha256sums_lines_are_hex_two_spaces_path() {
        let files = vec![
            ShippedFile { path: "VERSION".to_string(), size: 16, sha256: "00".to_string() },
            ShippedFile { path: "profiles/civic.json".to_string(), size: 3, sha256: "11".to_string() },
        ];
        assert_eq!(sums_text(&files), "00  VERSION\n11  profiles/civic.json\n");
    }
}
