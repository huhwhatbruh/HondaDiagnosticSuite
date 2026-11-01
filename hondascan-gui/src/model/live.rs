//! Live data: the glance cards, the Hero and the open system screen: the model side of glance.rs and sysview.rs, and the car drawing.
use super::*;

impl qobject::AppModel {
    /// The hero's figures and the glance cards, from what the polls have read.
    pub(super) fn publish_glances(mut self: Pin<&mut Self>) {
        let (hero, layout, live) = {
            let r = self.rust();
            (
                r.glance_polls.first().and_then(Option::as_ref).map_or_else(|| json!({}), Glance::figures),
                Value::Array(r.glance_polls.iter().flatten().map(Glance::layout_json).collect()),
                Value::Array(r.glance_polls.iter().flatten().map(Glance::live_json).collect()),
            )
        };
        self.as_mut().set_engine(q(&hero));
        // the layout's string is the same between updates, so the setter stays quiet and the cards' gauges and tiles are kept
        self.as_mut().set_glances(q(&layout));
        self.as_mut().set_glances_live(q(&live));
    }

    pub(super) fn publish_system(mut self: Pin<&mut Self>) {
        let (layout, live) = {
            let r = self.rust();
            r.sysview.as_ref().map_or_else(|| (json!({}), json!({})), |v| (v.layout_json(), v.live_json()))
        };
        // the layout's string is the same between updates, so the setter stays quiet and the screen's rows are kept
        self.as_mut().set_system(q(&layout));
        self.as_mut().set_system_live(q(&live));
    }

    pub fn watch_system(mut self: Pin<&mut Self>, key: &QString) {
        let key = key.to_string();
        let Some(e) = self.rust().ecu_list.iter().find(|e| e.key() == key).cloned() else { return };
        if e.system.is_empty() { return; }
        self.as_mut().rust_mut().watching = Some(key);
        self.as_mut().publish_ops();
        if self.rust().catalogs.contains_key(&e.system) { return self.begin_watch(&e); }
        let sys = e.system.clone();
        self.rpc("catalog", json!({ "system": sys }), move |m, r| m.on_catalog(e, r));
    }

    pub fn unwatch_system(mut self: Pin<&mut Self>) {
        {
            let mut r = self.as_mut().rust_mut();
            r.sysview = None;
            r.watching = None;
        }
        self.as_mut().publish_ops();
        self.publish_system();
    }

    pub fn system_query(self: Pin<&mut Self>, q: &QString) { let q = q.to_string(); self.with_view(|v| v.set_query(&q)); }

    pub fn toggle_group(self: Pin<&mut Self>, did: i32) { self.with_view(|v| v.toggle_group(did as u32)); }

    pub fn open_all_groups(self: Pin<&mut Self>, on: bool) { self.with_view(|v| v.open_all(on)); }

    pub fn set_running(self: Pin<&mut Self>, on: bool) { self.with_view(|v| v.set_running(on)); }

    pub fn toggle_pin(mut self: Pin<&mut Self>, did: i32, index: i32) {
        let Some((system, pins)) = self.as_mut().with_view(|v| (v.ecu.system.clone(), v.toggle_pin((did as u32, index as usize)))) else { return };
        // remembered per system, so the next visit starts with the same gauges
        if let Some(st) = &mut self.as_mut().rust_mut().settings {
            let mut s = st.last.clone();
            s.pins.insert(system, pins);
            st.save(&s);
        }
    }

    pub fn car_keys(&self, side: f64) -> QString { q(&crate::car::keys_json(&self.rust().ecu_list, side as f32)) }

    /// Start each glance's poll: at once when its system's catalog is here, else after it is fetched.
    pub(super) fn start_glances(mut self: Pin<&mut Self>) {
        for (idx, def) in DEFS.iter().enumerate() {
            let Some(e) = ecu_for(def, &self.rust().ecu_list) else { continue };
            if self.rust().catalogs.contains_key(&e.system) { self.as_mut().start_glance(idx, &e); }
            else {
                let sys = e.system.clone();
                self.rpc("catalog", json!({ "system": sys }), move |m, r| m.on_catalog(e, r));
            }
        }
    }

    /// Open the system screen's live view of `e` (its catalog is here).
    pub(super) fn begin_watch(mut self: Pin<&mut Self>, e: &Ecu) {
        let Some(cat) = self.rust().catalogs.get(&e.system).cloned() else { return };
        let pins = self.rust().settings.as_ref().and_then(|s| s.last.pins.get(&e.system).cloned());
        self.as_mut().rust_mut().sysview = Some(SysView::new(e.clone(), cat, pins));
        self.as_mut().publish_ops();
        self.publish_system();
    }

    pub(super) fn start_glance(mut self: Pin<&mut Self>, idx: usize, e: &Ecu) {
        let Some(cat) = self.rust().catalogs.get(&e.system).cloned() else { return };
        self.as_mut().rust_mut().glance_polls[idx] = Some(Glance::new(&DEFS[idx], e.clone(), cat));
        self.publish_glances();
    }

    pub(super) fn on_glance_live(mut self: Pin<&mut Self>, idx: usize, r: Reply) {
        if let Some(Some(g)) = self.as_mut().rust_mut().glance_polls.get_mut(idx) { g.live.answered(r, GLANCE_EVERY); }
        self.publish_glances();
    }

    pub(super) fn on_system_live(mut self: Pin<&mut Self>, r: Reply) {
        if let Some(v) = &mut self.as_mut().rust_mut().sysview { v.live.answered(r, SYSTEM_EVERY); }
        self.publish_system();
    }

    /// Run a change on the open system view, remember its pins when they changed, and show the result.
    pub(super) fn with_view<R>(mut self: Pin<&mut Self>, f: impl FnOnce(&mut SysView) -> R) -> Option<R> {
        let out = self.as_mut().rust_mut().sysview.as_mut().map(f);
        self.publish_system();
        out
    }
}
