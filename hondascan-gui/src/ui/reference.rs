//! Everything a system can report, in HDS's own terms: the data group (DID) each signal lives in, its symbol and HDS
//! description, next to the plain name (ported from hondascan-app/src/components/DataReference.tsx).
use crate::api::Catalog;
use crate::names::friendly;
use crate::theme::*;
use crate::widgets::window;
use eframe::egui::{self, RichText, Ui};

pub fn show(ui: &mut Ui, cat: Option<&Catalog>, system: &str, query: &mut String) {
    window(ui, "Data reference", "", false, |ui| {
        let Some(cat) = cat else { return { ui.add_space(8.0); ui.label(RichText::new("Loading…").color(MUTED_FG)); } };
        ui.add_space(8.0);
        let q = query.trim().to_lowercase();
        let rows: Vec<_> = cat.dids.iter().flat_map(|d| d.signals.iter().map(move |s| (d.did, s))).filter(|(did, s)| {
            q.is_empty() || format!("{} {} {} {:04x}", friendly(s).name, s.symbol, s.description, did).to_lowercase().contains(&q)
        }).collect();
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.add(egui::TextEdit::singleline(query).hint_text("Search name, symbol or data group").desired_width(300.0));
            ui.label(RichText::new(format!("{} signals in {} data groups of {system}", rows.len(), cat.dids.len())).size(12.0).color(MUTED_FG));
        });
        ui.horizontal_wrapped(|ui| { ui.add_space(14.0); ui.label(RichText::new("The car sends its data in numbered groups (data identifiers, DIDs). Each group holds several signals; the symbol is Honda’s short name for one. Names in plain words are on the Live data tab.").size(12.0).color(MUTED_FG)); });
        ui.add_space(6.0);
        let widths = [230.0, 300.0, 340.0, 110.0, 70.0];
        let head = ["Name", "Honda’s description", "What Honda says", "Symbol", "Group"];
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            for (h, w) in head.iter().zip(widths) { ui.add_sized([w, 18.0], egui::Label::new(RichText::new(*h).size(12.0).color(MUTED_FG))); }
            ui.label(RichText::new("Unit").size(12.0).color(MUTED_FG));
        });
        ui.separator();
        egui::ScrollArea::vertical().max_height(560.0).show_rows(ui, 24.0, rows.len(), |ui, range| {
            for (did, s) in &rows[range] {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let cell = |ui: &mut Ui, w: f32, t: String, c: egui::Color32, mono_font: bool| {
                        let rt = RichText::new(t).color(c).font(if mono_font { mono(12.0) } else { sans(12.5) });
                        ui.add_sized([w, 22.0], egui::Label::new(rt).truncate());
                    };
                    cell(ui, widths[0], friendly(s).name, FOREGROUND, false);
                    cell(ui, widths[1], s.description.clone(), MUTED_FG, false);
                    cell(ui, widths[2], s.help.clone().unwrap_or("—".into()).replace('\n', " "), MUTED_FG, false);
                    cell(ui, widths[3], s.symbol.clone(), FOREGROUND, true);
                    cell(ui, widths[4], format!("0x{did:04X}"), FOREGROUND, true);
                    ui.label(RichText::new(crate::format::convert(0.0, &s.unit).1).size(12.0).color(MUTED_FG));
                });
            }
        });
        ui.add_space(8.0);
    });
}
