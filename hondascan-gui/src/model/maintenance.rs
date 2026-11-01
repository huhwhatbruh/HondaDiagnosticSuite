//! The reference snapshot, service markers, the health report and the export: the model side of baseline.rs and maintenance.rs.
use super::*;

impl qobject::AppModel {
    /// The reference snapshot card and the save dialog's facts, from what is known now.
    pub(super) fn publish_maintenance(mut self: Pin<&mut Self>) {
        let (card, info) = {
            let r = self.rust();
            let vin = self.vin().to_string();
            let vin = Some(vin.as_str()).filter(|v| !v.is_empty());
            (
                baseline::card_json(r.maintenance_data.as_ref(), r.maintenance_err.as_deref(), &r.ecu_list, &r.dtc_state, vin, *self.link() == 2, r.last_report.as_ref()),
                baseline::freeze_info_json(&r.ecu_list, &r.dtc_state, vin),
            )
        };
        self.as_mut().set_maintenance(q(&card));
        self.as_mut().set_freeze_info(q(&info));
    }

    /// Load this car's frozen baselines and service markers (the health card reads them; the dialogs refresh them).
    pub(super) fn refresh_maintenance(mut self: Pin<&mut Self>) {
        let vin = self.vin().to_string();
        if vin.is_empty() { return; }
        let again = self.as_mut().rust_mut().history_state.reload_record();
        if !again.is_empty() { self.as_mut().history_calls(again); }
        self.rpc("maintenance", json!({ "vin": vin }), |mut m, r| {
            match r.and_then(|v| parse::<Maintenance>(v).map_err(|e| format!("unreadable maintenance reply: {e}"))) {
                Ok(d) => { let mut rm = m.as_mut().rust_mut(); rm.maintenance_data = Some(d); rm.maintenance_err = None; }
                Err(e) => m.as_mut().rust_mut().maintenance_err = Some(e),
            }
            m.publish_maintenance();
        });
    }

    pub fn freeze_baseline(self: Pin<&mut Self>, label: &QString) {
        let vin = self.vin().to_string();
        if vin.is_empty() { return; }
        let label = label.to_string();
        let label = label.trim();
        let params = if label.is_empty() { json!({ "vin": vin }) } else { json!({ "vin": vin, "label": label }) };
        self.rpc("baseline_freeze", params, |mut m, r| match r {
            Ok(v) => {
                let f: FreezeReply = parse(v).unwrap_or_default();
                if f.frozen {
                    if let (Some(b), Some(d)) = (f.baseline, &mut m.as_mut().rust_mut().maintenance_data) { d.latest = Some(b); }
                    m.as_mut().say(SUCCESS, "freeze", "Reference snapshot saved",
                        "The health card and the next report compare the car against this one. It cannot be edited or removed.", 10.0);
                } else {
                    m.as_mut().say(WARN, "freeze", "The snapshot was not saved",
                        "The car no longer met the conditions. Read the codes again and check it has no active trouble codes.", 15.0);
                }
                m.as_mut().publish_maintenance();
                m.refresh_maintenance();
            }
            Err(e) => m.say(ERROR, "freeze", "Could not save the snapshot", &e, 15.0),
        });
    }

    pub fn add_marker(self: Pin<&mut Self>, kind: &QString, note: &QString, mileage: &QString) {
        let vin = self.vin().to_string();
        if vin.is_empty() { return; }
        let input = match check_marker(&kind.to_string(), &note.to_string(), &mileage.to_string()) {
            Ok(i) => i,
            Err(e) => return self.say(ERROR, "marker", "Could not add the service marker", &e, 12.0),
        };
        let mut params = json!({ "vin": vin, "kind": input.kind, "note": input.note });
        if let Some(m) = input.mileage_miles { params["mileage_km"] = json!(crate::maintenance::miles_to_km(m)); }
        self.rpc("marker_add", params, |mut m, r| match r {
            Ok(_) => { m.as_mut().say(SUCCESS, "marker", "Service marker added", "It is kept with this car's history.", 6.0); m.refresh_maintenance(); }
            Err(e) => m.say(ERROR, "marker", "Could not add the service marker", &e, 12.0),
        });
    }

    pub fn remove_marker(self: Pin<&mut Self>, id: i32) {
        self.rpc("marker_remove", json!({ "id": id }), |mut m, r| match r {
            Ok(_) => { m.as_mut().say(INFO, "marker", "Service marker removed", "", 6.0); m.refresh_maintenance(); }
            Err(e) => m.say(ERROR, "marker", "Could not remove the service marker", &e, 12.0),
        });
    }

    pub fn print_report(self: Pin<&mut Self>) {
        let vin = self.vin().to_string();
        let params = if vin.is_empty() { json!({}) } else { json!({ "vin": vin }) };
        self.rpc("health_report", params, |mut m, r| match r {
            Ok(v) => {
                let rep: ReportReply = parse(v).unwrap_or_default();
                let verdict = baseline::verdict_label(&rep.verdict);
                let text = match (rep.summary.is_empty(), rep.path.is_empty()) {
                    (false, false) => format!("{}\n{}", rep.summary, rep.path),
                    (false, true) => rep.summary.clone(),
                    (true, false) => rep.path.clone(),
                    (true, true) => "The backend did not say where the report was written.".into(),
                };
                m.as_mut().say(SUCCESS, "report", &format!("Health report: {verdict}"), &text, 15.0);
                m.as_mut().rust_mut().last_report = Some(rep);
                m.as_mut().publish_history();
                m.publish_maintenance();
            }
            Err(e) => m.say(ERROR, "report", "Could not write the health report", &e, 15.0),
        });
    }

    pub fn export_data(self: Pin<&mut Self>) {
        self.rpc("export", json!({}), |mut m, r| match r {
            Ok(v) => {
                let e: ExportReply = parse(v).unwrap_or_default();
                m.as_mut().say(SUCCESS, "export", "Data exported", &format!("Backup: {}\nJSON: {}", e.backup_path, e.json_path), 15.0);
                m.as_mut().rust_mut().history_state.export = Some(e);
                m.publish_history();
            }
            Err(e) => m.say(ERROR, "export", "Could not export the data", &e, 15.0),
        });
    }

    pub fn marker_check(&self, kind: &QString, note: &QString, mileage: &QString) -> QString {
        QString::from(check_marker(&kind.to_string(), &note.to_string(), &mileage.to_string()).err().unwrap_or_default().as_str())
    }

    pub fn open_path(&self, path: &QString) {
        let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
        // a path that starts with a dash would be read as an option by the opener
        let path = path.to_string();
        let path = if path.starts_with('-') { format!("./{path}") } else { path };
        let _ = std::process::Command::new(opener).arg(path).spawn();
    }
}
