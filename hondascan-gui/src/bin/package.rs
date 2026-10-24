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
use std::path::{Path, PathBuf};
use std::process::Command;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

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
fn release_rustflags(repo_root: &Path) -> Res<String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    Ok(format!(
        "--remap-path-prefix={}=. --remap-path-prefix={home}/.cargo=/c --remap-path-prefix={home}/.rustup=/r \
         -C link-arg=-Wl,--build-id=none --cfg hondascan_packaged",
        repo_root.display()
    ))
}

fn copy_dir(from: &Path, to: &Path) -> Res<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() { copy_dir(&e.path(), &dest)? } else { std::fs::copy(e.path(), dest)?; }
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
        let path = e.path();
        if e.file_type()?.is_dir() { collect_files(root, &path, out)? } else { out.push((relative_path(root, &path), std::fs::read(&path)?)) }
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
fn verify_packaged_cli(out: &Path) -> Res<()> {
    let exe = if cfg!(windows) { "hondascan.exe" } else { "hondascan" };
    let cli = out.join(exe);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let home = std::env::temp_dir().join(format!("hondascan-package-check-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&home)?;

    let mut cmd = Command::new(&cli);
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
const BANNED_IN_PACKAGE_README: &[&str] = &["release-key", "hds 3.105", "decompil", "installer media", "/home/", "umf", "vendor"];

fn main() -> Res<()> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let krate = here.join("../hondascan");
    let db = krate.join("data/hondascan.db");
    let db_enc = db.with_extension("db.enc");
    let exe = if cfg!(windows) { ".exe" } else { "" };
    if !db.exists() {                                            // generated, never tracked
        run(Command::new("cargo").args(["run", "--release", "--bin", "build-db"]).current_dir(&krate))?;
    }
    let repo_root = here.parent().unwrap_or(here);
    let rustflags = release_rustflags(repo_root)?;
    run(Command::new("cargo").args(["build", "--release"]).current_dir(here).env("RUSTFLAGS", &rustflags))?;
    run(Command::new("cargo").args(["build", "--release", "--bins"]).current_dir(&krate).env("RUSTFLAGS", &rustflags))?;
    // Ship the encrypted form, not the plain SQLite file: `hondascan.db` is readable with `sqlite3` as-is, and it
    // carries the decoded HDS tables, which is the data this package exists to not hand out for free. `db.rs`
    // decrypts it in memory on open (`dbcrypt`); re-encrypt every time, so a stale `.enc` from an older `hondascan.db`
    // never ships silently.
    run(Command::new("cargo").args(["run", "--release", "--bin", "encrypt-db"]).current_dir(&krate))?;

    let target_os = std::env::consts::OS;
    let os = if target_os == "macos" { "darwin" } else { target_os };   // the folder name, as before
    let version = env!("CARGO_PKG_VERSION");
    let name = format!("hondascan-{version}-{os}-{}", std::env::consts::ARCH);
    let dist = here.join("dist");
    let out = dist.join(&name);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(out.join("data"))?;
    std::fs::create_dir_all(out.join("drivers"))?;
    let binaries = [format!("hondascan-gui{exe}"), format!("hondascan-rpc{exe}"), format!("hondascan{exe}")];
    std::fs::copy(here.join(format!("target/release/hondascan-gui{exe}")), out.join(&binaries[0]))?;
    for b in ["hondascan-rpc", "hondascan"] {
        std::fs::copy(krate.join(format!("target/release/{b}{exe}")), out.join(format!("{b}{exe}")))?;
    }
    std::fs::copy(&db_enc, out.join("data/hondascan.db.enc"))?;
    if krate.join("profiles").is_dir() {
        copy_dir(&krate.join("profiles"), &out.join("profiles"))?; // recorded cars; the public source has none
    }
    std::fs::copy(krate.join("drivers/gd101_j2534.c"), out.join("drivers/gd101_j2534.c"))?;
    // The archive carries its own short user README, never the repo's README.md (the research notes, which name vendor
    // material and private paths and are not for the public release).
    let readme = std::fs::read_to_string(here.join("assets/PACKAGE-README.md"))?;
    if let Some(word) = BANNED_IN_PACKAGE_README.iter().find(|w| readme.to_lowercase().contains(**w)) {
        return Err(format!("assets/PACKAGE-README.md mentions {word:?}, which must not ship").into());
    }
    std::fs::write(out.join("README.md"), readme)?;
    std::fs::copy(here.join("assets/fonts/OFL-Geist.txt"), out.join("OFL-Geist.txt"))?;
    let rules = krate.join("packaging/60-hondascan.rules");
    if cfg!(target_os = "linux") && rules.exists() {
        std::fs::copy(rules, out.join("60-hondascan.rules"))?;
    }
    for b in &binaries {
        make_executable(&out.join(b))?;
    }

    write_metadata(&out, version, target_os, std::env::consts::ARCH, unix_seconds())?;

    let archive = dist.join(format!("{name}{}", if cfg!(windows) { ".zip" } else { ".tar.gz" }));
    let flags = if cfg!(windows) { "-acf" } else { "-czf" };
    run(Command::new("tar").arg(flags).arg(&archive).arg("-C").arg(&dist).arg(&name))?;
    let archive_name = archive.file_name().ok_or("archive path has no file name")?.to_string_lossy().into_owned();
    let archive_sum = sha256_hex(&std::fs::read(&archive)?);
    let sum_path: PathBuf = dist.join(format!("{archive_name}.sha256"));
    std::fs::write(&sum_path, format!("{archive_sum}  {archive_name}\n"))?;

    verify_packaged_cli(&out)?;

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
        let readme = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/PACKAGE-README.md")).unwrap().to_lowercase();
        for w in BANNED_IN_PACKAGE_README {
            assert!(!readme.contains(w), "package README mentions {w:?}");
        }
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
