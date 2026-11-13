use cxx_qt_build::{CxxQtBuilder, QResource, QResources, QmlFile, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.hondascan.app")
            .qml_files([
                QmlFile::from("qml/Theme.qml").singleton(true),
                QmlFile::from("qml/Main.qml"),
                QmlFile::from("qml/Gauge.qml"),
                QmlFile::from("qml/KlineView.qml"),
                QmlFile::from("qml/Panel.qml"),
                QmlFile::from("qml/Button.qml"),
                QmlFile::from("qml/ControlFace.qml"),
                QmlFile::from("qml/Segmented.qml"),
                QmlFile::from("qml/Check.qml"),
                QmlFile::from("qml/Caption.qml"),
                QmlFile::from("qml/ConditionCard.qml"),
                QmlFile::from("qml/Chip.qml"),
                QmlFile::from("qml/Field.qml"),
                QmlFile::from("qml/Area.qml"),
                QmlFile::from("qml/Glyph.qml"),
                QmlFile::from("qml/Tappable.qml"),
                QmlFile::from("qml/Sheet.qml"),
                QmlFile::from("qml/TopBar.qml"),
                QmlFile::from("qml/Sidebar.qml"),
                QmlFile::from("qml/Overview.qml"),
                QmlFile::from("qml/Welcome.qml"),
                QmlFile::from("qml/Hero.qml"),
                QmlFile::from("qml/HexMesh.qml"),
                QmlFile::from("qml/Health.qml"),
                QmlFile::from("qml/CodesList.qml"),
                QmlFile::from("qml/Toasts.qml"),
                QmlFile::from("qml/SystemView.qml"),
                QmlFile::from("qml/ReadCodesDialog.qml"),
                QmlFile::from("qml/LivePanel.qml"),
                QmlFile::from("qml/Spark.qml"),
                QmlFile::from("qml/AlertBar.qml"),
                QmlFile::from("qml/BaselineCard.qml"),
                QmlFile::from("qml/FreezeDialog.qml"),
                QmlFile::from("qml/MarkerDialog.qml"),
                QmlFile::from("qml/ShareDialog.qml"),
                QmlFile::from("qml/CarCard.qml"),
                QmlFile::from("qml/SystemIcon.qml"),
                QmlFile::from("qml/ConfirmDialog.qml"),
                QmlFile::from("qml/OpsTabs.qml"),
                QmlFile::from("qml/RunningBars.qml"),
                QmlFile::from("qml/Table.qml"),
                QmlFile::from("qml/HistoryView.qml"),
                QmlFile::from("qml/GlanceCard.qml"),
                QmlFile::from("qml/MapsCard.qml"),
                QmlFile::from("qml/ReferenceTab.qml"),
                QmlFile::from("qml/SelfCheck.qml"),
                QmlFile::from("qml/DriveCard.qml"),
            ])
            .depend("QtQuick")
            .depend("QtQuick.Shapes")
            .depend("QtQuick.Controls"),
    )
    // The Noto fonts the app is drawn in, and Barlow Semi Condensed for the dials' big figures (all OFL, assets/fonts/OFL-*.txt), so it looks the same on a machine that has none of them.
    // Loaded in Main.qml as qrc:/assets/fonts/<file>.ttf.
    .qrc_resources(QResources::new().resource(QResource::new().prefix("/").files(FONTS)))
    .files(["src/model.rs"])
    .build();

    // A packaged app carries its Qt next to it (package.rs: lib/, plugins/, qml/). RPATH, not RUNPATH: it is also searched for the
    // libraries Qt's own plugins load, so a bundled Qt never mixes with the system's.
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "linux") {
        println!("cargo::rustc-link-arg=-Wl,--disable-new-dtags,-rpath,$ORIGIN/lib");
    }
}

const FONTS: [&str; 5] = [
    "assets/fonts/NotoSans-Regular.ttf",
    "assets/fonts/NotoSans-Bold.ttf",
    "assets/fonts/NotoSansMono-Regular.ttf",
    "assets/fonts/NotoSansMono-Medium.ttf",
    "assets/fonts/BarlowSemiCondensed-SemiBold.ttf",
];
