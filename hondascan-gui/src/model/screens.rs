//! History, the engine maps, the data reference and the installation check: the model side of history.rs, maps.rs, reference.rs and selfcheck.rs.
use super::*;

impl qobject::AppModel {
    /// The installation check, from what the backend answered and what only the app knows.
    pub(super) fn publish_selfcheck(mut self: Pin<&mut Self>) {
        let v = {
            let r = self.rust();
            selfcheck_json(&SelfCheckState { check: r.self_check.clone(), err: r.self_check_err.clone(), backend_version: r.backend_version.clone(),
                settings_known: r.settings.is_some(), settings_writable: r.settings_writable })
        };
        self.as_mut().set_selfcheck(q(&v));
    }

    /// Ask the backend to check this computer (offline: it opens no adapter) and remember the result.
    pub(super) fn run_self_check(mut self: Pin<&mut Self>) {
        let writable = self.rust().settings.as_ref().is_some_and(|s| s.writable());
        self.as_mut().rust_mut().settings_writable = writable;
        self.as_mut().publish_selfcheck();
        self.rpc("self_check", json!({}), |mut m, r| {
            match r {
                Ok(v) => {
                    let sc: SelfCheck = parse(v).unwrap_or_default();
                    let writable = m.rust().settings_writable;
                    let ok = passed(&sc, writable);
                    if let Some(st) = &mut m.as_mut().rust_mut().settings {
                        let mut s = st.last.clone();
                        s.self_check_at = unix_now();
                        s.self_check_ok = ok;
                        st.save(&s);
                    }
                    let mut rm = m.as_mut().rust_mut();
                    rm.self_check = Some(sc);
                    rm.self_check_err = None;
                }
                Err(e) => { let mut rm = m.as_mut().rust_mut(); rm.self_check = None; rm.self_check_err = Some(e); }
            }
            m.publish_selfcheck();
        });
    }

    pub(super) fn publish_history(mut self: Pin<&mut Self>) {
        let v = {
            let rm = self.as_mut().rust_mut();
            let mut rm = rm;
            let report = rm.last_report.clone();
            rm.history_state.report = report;
            rm.history_state.ecus = rm.ecu_list.clone();
            rm.history_state.history_json()
        };
        self.as_mut().set_history(q(&v));
    }

    /// Send the calls the History screen asks for and feed each reply back to it (the ones it asks for next are sent in turn).
    pub(super) fn history_calls(mut self: Pin<&mut Self>, calls: Vec<(String, Value)>) {
        for (method, params) in calls {
            let id = params["id"].as_u64().unwrap_or(0) as u32;
            let name = method.clone();
            self.as_mut().rpc(&method, params, move |mut m, r| {
                let vin = m.vin().to_string();
                let current = Some(vin.as_str()).filter(|v| !v.is_empty());
                let next = {
                    let mut rm = m.as_mut().rust_mut();
                    let h = &mut rm.history_state;
                    match name.as_str() {
                        "vehicles" => h.on_vehicles(r, current),
                        "vehicle" => { h.on_vehicle(r); Vec::new() }
                        "maintenance" => { h.on_maintenance(r); Vec::new() }
                        "audit" => { h.on_audit(r); Vec::new() }
                        "session" => { h.on_session(id, r); Vec::new() }
                        _ => Vec::new(),
                    }
                };
                m.as_mut().publish_history();
                if !next.is_empty() { m.history_calls(next); }
            });
        }
    }

    /// Spawn errors are ignored: a machine with no desktop still records the path, and saying so would only add noise to a successful write.
    pub fn select_map(mut self: Pin<&mut Self>, symbol: &QString) {
        self.as_mut().rust_mut().map_sel = symbol.to_string();
        let sel = self.rust().map_sel.clone();
        let v = maps_json(self.rust().status_data.as_ref().and_then(|s| s.maps.as_ref()), &sel);
        self.as_mut().set_maps(q(&v));
    }

    pub fn history_open(mut self: Pin<&mut Self>) {
        let first = { let mut rm = self.as_mut().rust_mut(); rm.history_state.reload(); rm.history_state.ensure_started() };
        self.as_mut().publish_history();
        if let Some(call) = first { self.history_calls(vec![call]); }
    }

    pub fn history_select(mut self: Pin<&mut Self>, vin: &QString) {
        let calls = self.as_mut().rust_mut().history_state.select(&vin.to_string());
        self.as_mut().publish_history();
        self.history_calls(calls);
    }

    pub fn history_session(self: Pin<&mut Self>, id: i32) { self.history_calls(vec![History::session_call(id as u32)]); }

    pub fn reference(&self, query: &QString) -> QString {
        let r = self.rust();
        let ecu = self.watched_ecu();
        let cat = ecu.as_ref().and_then(|e| r.catalogs.get(&e.system));
        q(&reference_json(cat.map(|c| &**c), ecu.as_ref().map_or("", |e| e.system.as_str()), &query.to_string()))
    }
}

impl qobject::AppModel {
    /// Whether sharing a car report is on, and whether the first-launch question is still due (the backend keeps both: share.rs).
    /// A build with no collector address has nothing to send to, so it asks nothing.
    pub(super) fn load_share(self: Pin<&mut Self>) {
        self.rpc("share_status", json!({}), |mut m, r| {
            let Ok(v) = r else { return };
            let asked = v["asked"].as_bool().unwrap_or(true);
            let can = v["can_send"].as_bool().unwrap_or(false);
            m.as_mut().set_share_on(v["enabled"].as_bool().unwrap_or(false));
            m.as_mut().set_share_can(can);
            m.as_mut().set_share_ask(can && !asked);
        });
    }

    pub fn share_answer(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().set_share_ask(false); // answered, whatever the backend says next: the card never comes back on its own
        self.rpc("share_set", json!({"enabled": on}), |mut m, r| match r {
            Ok(v) => m.as_mut().set_share_on(v["enabled"].as_bool().unwrap_or(false)),
            Err(e) => m.say(WARN, "share", "Could not save the sharing choice", &e, 8.0),
        });
    }

    pub fn share_preview(self: Pin<&mut Self>) {
        self.rpc("share_preview", json!({}), |mut m, r| {
            let text = r.ok().and_then(|v| v["text"].as_str().map(String::from)).unwrap_or_default();
            m.as_mut().set_share_text(QString::from(text.as_str()));
        });
    }
}
