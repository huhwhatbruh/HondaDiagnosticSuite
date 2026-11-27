//! The adapter list in the header, run for real: `qml/TopBar.qml` in Qt's `qmltestrunner` against a stand-in `AppModel`, whose
//! `refreshAdapters` answers as the app's does (later, and only a changed list is announced). It is QML behaviour, which no Rust
//! test sees: a list replaced under a ComboBox puts it back on its first row while the Connect button keeps the old choice.
//! Needs the `qmltestrunner` of the Qt the app is built with (`qmake -query QT_INSTALL_BINS`) and its QtTest module.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const APP_MODEL: &str = r#"
pragma Singleton
import QtQuick
QtObject {
    property int link: 0
    property bool scanning: false
    property string vin: ""
    property string adapter: ""
    property string adapters: "[]"
    property string lastAdapter: ""
    property string kline: "{}"
    property string plugged: "[]"   // what the next look at the ports finds
    function refreshAdapters() { Qt.callLater(() => { adapters = plugged }) }
}
"#;

const CASES: &str = r#"
import QtQuick
import QtTest
import com.hondascan.app

Item {
    width: 1000; height: 600
    Component { id: header; TopBar { width: 1000 } }

    TestCase {
        name: "AdapterList"; when: windowShown
        property var bar
        property var combo

        function findCombo(item) {
            if (item.popup !== undefined && item.currentIndex !== undefined) return item
            for (const child of item.children) { const found = findCombo(child); if (found) return found }
            return null
        }
        function rows(specs) { return JSON.stringify(specs.map(s => ({ spec: s, label: "label " + s }))) }
        // the app as it started: these adapters, and the one used last time
        function start(specs, saved) {
            AppModel.adapter = ""; AppModel.lastAdapter = saved
            AppModel.adapters = rows(specs); AppModel.plugged = rows(specs)
            bar = createTemporaryObject(header, parent)
            combo = findCombo(bar)
            wait(20)
        }
        function openList() { mouseClick(combo); tryVerify(() => combo.popup.opened); wait(20) }
        function closeList() { keyClick(Qt.Key_Escape); tryVerify(() => !combo.popup.visible) }
        // the person picks a row of the open list with the pointer
        function pick(spec) {
            openList()
            const at = bar.list.findIndex(a => a.spec === spec)
            verify(at >= 0, spec + " is not in the list")
            combo.popup.contentItem.positionViewAtIndex(at, ListView.Contain)
            wait(20)
            mouseClick(combo.popup.contentItem.itemAtIndex(at))
            tryVerify(() => !combo.popup.visible)
        }
        // what the closed list shows is what Connect would open
        function shows(spec) {
            compare(bar.spec, spec)
            compare(combo.displayText, "label " + spec)
        }

        function test_an_adapter_plugged_in_later_is_there_when_the_list_is_opened() {
            start(["gd101", "sim", "custom"], "")
            AppModel.plugged = rows(["gd101", "sim", "mvci:/dev/x", "custom"])
            openList()
            compare(combo.count, 4)
            closeList()
            pick("mvci:/dev/x")
            shows("mvci:/dev/x")
        }
        function test_a_new_list_keeps_the_row_the_person_picked() {
            start(["gd101", "sim", "elm:/dev/b", "custom"], "gd101")
            pick("sim")
            AppModel.plugged = rows(["gd101", "sim", "elm:/dev/b", "mvci:/dev/x", "custom"])   // the row's place stays
            openList(); closeList()
            shows("sim")
            pick("elm:/dev/b")
            AppModel.plugged = rows(["gd101", "sim", "elm:/dev/a", "elm:/dev/b", "custom"])    // the row moves
            openList(); closeList()
            shows("elm:/dev/b")
        }
        function test_the_saved_adapter_is_picked_once_it_is_plugged_in() {
            start(["gd101", "sim", "custom"], "mvci:/dev/x")
            compare(bar.spec, "mvci:/dev/x")   // not there yet: the custom row holds its spec
            AppModel.plugged = rows(["gd101", "sim", "mvci:/dev/x", "custom"])
            openList(); closeList()
            shows("mvci:/dev/x")
        }
        function test_a_typed_spec_survives_a_new_list() {
            start(["gd101", "sim", "custom"], "mvci:/dev/gone")
            bar.custom = ""
            const field = findField(bar)
            field.forceActiveFocus()
            keyClick(Qt.Key_X)
            compare(bar.spec, "x")
            AppModel.plugged = rows(["gd101", "sim", "elm:/dev/a", "custom"])
            openList(); closeList()
            compare(bar.spec, "x")
            compare(combo.displayText, "label custom")
        }
        function findField(item) {
            if (item.textEdited !== undefined && item.visible) return item
            for (const child of item.children) { const found = findField(child); if (found) return found }
            return null
        }
        function test_an_unplugged_choice_is_not_swapped_for_another_adapter() {
            start(["gd101", "sim", "elm:/dev/a", "custom"], "")
            pick("elm:/dev/a")
            AppModel.plugged = rows(["gd101", "sim", "elm:/dev/b", "custom"])
            openList(); closeList()
            compare(bar.spec, "elm:/dev/a")
            compare(combo.displayText, "label custom")
        }
        function test_the_adapter_in_use_stays_chosen_after_disconnect() {
            start(["gd101", "sim", "custom"], "gd101")
            AppModel.adapter = "sim"   // connected by the backend or the welcome button
            shows("sim")
            AppModel.plugged = rows(["gd101", "sim", "elm:/dev/a", "custom"])
            openList(); closeList()
            shows("sim")
        }
    }
}
"#;

/// Where the build's Qt keeps its tools.
fn qt_runner() -> PathBuf {
    let qmakes = std::env::var("QMAKE").ok().into_iter().chain(["qmake6".to_string(), "qmake".to_string()]);
    for qmake in qmakes {
        let Ok(out) = Command::new(&qmake).args(["-query", "QT_INSTALL_BINS"]).output() else { continue };
        let runner = Path::new(String::from_utf8_lossy(&out.stdout).trim()).join(if cfg!(windows) { "qmltestrunner.exe" } else { "qmltestrunner" });
        if out.status.success() && runner.is_file() {
            return runner;
        }
    }
    panic!("no qmltestrunner beside Qt's tools (set QMAKE to the qmake of the Qt the app is built with)");
}

#[test]
fn the_adapter_list_looks_again_when_opened_and_keeps_the_choice() {
    let dir = std::env::temp_dir().join(format!("hondascan-gui-{}-topbar", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let module = dir.join("com/hondascan/app");
    fs::create_dir_all(&module).unwrap();
    let mut qmldir = "module com.hondascan.app\nsingleton AppModel 1.0 AppModel.qml\n".to_string();
    for entry in fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/qml")).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some(name) = file.strip_suffix(".qml") else { continue };
        fs::copy(&path, module.join(&file)).unwrap();
        qmldir += &format!("{}{name} 1.0 {file}\n", if name == "Theme" { "singleton " } else { "" });
    }
    fs::write(module.join("qmldir"), qmldir).unwrap();
    fs::write(module.join("AppModel.qml"), APP_MODEL).unwrap();
    let cases = dir.join("tst_adapter_list.qml");
    fs::write(&cases, CASES).unwrap();
    let out = Command::new(qt_runner())
        .arg("-import").arg(&dir).arg("-input").arg(&cases).args(["-o", "-,txt"])
        .env("QT_QPA_PLATFORM", "offscreen").env("QT_FORCE_STDERR_LOGGING", "1").env("QML_DISABLE_DISK_CACHE", "1")
        .output().expect("qmltestrunner starts");
    let _ = fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}
