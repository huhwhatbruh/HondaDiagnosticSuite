//! The K-line screen's actions (kline.rs): VKWP systems on an ISO 14230 adapter, generic OBD-II on an `elm-obd` one. Read only.
use super::*;

impl qobject::AppModel {
    pub(super) fn publish_kline(mut self: Pin<&mut Self>) {
        let v = self.rust().kline_state.view();
        self.as_mut().set_kline(q(&v));
    }

    /// A connect said the adapter is on the K-line: pick the screen and start with what it shows first.
    pub(super) fn kline_connected(mut self: Pin<&mut Self>, spec: &str, kline: Option<&str>) -> bool {
        let mode = crate::kline::mode_for(spec, kline);
        self.as_mut().rust_mut().kline_state = crate::kline::Kline { mode, ..Default::default() };
        self.as_mut().publish_kline();
        match mode {
            "kwp" => self.kline_scan(),
            "obd" => self.obd_read(),
            _ => return false,
        }
        true
    }

    fn kline_busy(mut self: Pin<&mut Self>, what: &str) {
        let mut st = self.as_mut().rust_mut();
        st.kline_state.busy = what.to_string();
        st.kline_state.error.clear();
    }

    /// The reply to a K-line request: the error shown on the screen, or `ok` runs with the answer. True when the answer was taken;
    /// false for an error and for a reply that arrives after a disconnect (its link is gone), so the request that would follow is not sent.
    fn kline_reply(mut self: Pin<&mut Self>, r: Reply, ok: impl FnOnce(&mut crate::kline::Kline, Value)) -> bool {
        if *self.link() != 2 { return false; }
        let taken = r.is_ok();
        {
            let mut rm = self.as_mut().rust_mut();
            rm.kline_state.busy.clear();
            match r { Ok(v) => ok(&mut rm.kline_state, v), Err(e) => rm.kline_state.error = e }
        }
        self.publish_kline();
        taken
    }

    /// Ask every VKWP address whether an ECU answers (one TesterPresent each; slow when many are silent).
    pub fn kline_scan(mut self: Pin<&mut Self>) {
        if *self.link() != 2 || !self.rust().kline_state.busy.is_empty() { return; }
        {
            let mut rm = self.as_mut().rust_mut();
            rm.kline_state.scan = None;
            rm.kline_state.open = None;
        }
        self.as_mut().kline_busy("Looking for systems on the K-line…");
        self.as_mut().publish_kline();
        self.rpc("kline_scan", json!({}), |m, r| { m.kline_reply(r, |k, v| k.scan = Some(v)); });
    }

    /// Open one answering system: its codes and live values.
    pub fn kline_open(mut self: Pin<&mut Self>, address: i32, system: &QString) {
        if *self.link() != 2 || !self.rust().kline_state.busy.is_empty() { return; }
        let system = system.to_string();
        {
            let mut rm = self.as_mut().rust_mut();
            rm.kline_state.open = Some((address as i64, system.clone()));
            rm.kline_state.dtcs = None;
            rm.kline_state.live = None;
        }
        self.as_mut().kline_busy("Reading codes and live values…");
        self.as_mut().publish_kline();
        let p = json!({ "address": address, "system": system });
        self.rpc("kline_dtcs", p.clone(), |mut m, r| {
            // the live values follow once the codes are answered, so the bus has one request at a time; a rejected code read does not
            // cancel them (its error stays on screen), only a disconnect does
            if *m.link() != 2 { return; }
            m.as_mut().kline_reply(r, |k, v| k.dtcs = Some(v));
            let codes_error = m.rust().kline_state.error.clone();
            m.as_mut().kline_busy("Reading live values…");
            m.as_mut().rust_mut().kline_state.error = codes_error;
            m.as_mut().publish_kline();
            m.rpc("kline_live", p, |m, r| { m.kline_reply(r, |k, v| k.live = Some(v)); });
        });
    }

    /// Generic OBD-II: the check engine light, the codes and the VIN, then the live values.
    pub fn obd_read(mut self: Pin<&mut Self>) {
        if *self.link() != 2 || !self.rust().kline_state.busy.is_empty() { return; }
        {
            // the last read's answers go, as in kline_open: a read that fails must not leave old codes and values on the screen
            let mut rm = self.as_mut().rust_mut();
            rm.kline_state.obd = None;
            rm.kline_state.obd_live = None;
        }
        self.as_mut().kline_busy("Reading the car…");
        self.as_mut().publish_kline();
        self.rpc("obd_scan", json!({}), |mut m, r| {
            if m.as_mut().kline_reply(r, |k, v| k.obd = Some(v)) {
                m.as_mut().kline_busy("Reading live values…");
                m.as_mut().publish_kline();
                m.rpc("obd_live", json!({}), |m, r| { m.kline_reply(r, |k, v| k.obd_live = Some(v)); });
            }
        });
    }
}
