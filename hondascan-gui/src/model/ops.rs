//! Bi-directional tests, write functions and the hand-back when the backend dies: the model side of actuator.rs and writes.rs.
use super::*;

impl qobject::AppModel {
    /// The system screen is on `watching`: its ecu, found among the scan's.
    pub(super) fn watched_ecu(&self) -> Option<Ecu> {
        let r = self.rust();
        r.watching.as_ref().and_then(|k| r.ecu_list.iter().find(|e| e.key() == *k)).cloned()
    }

    /// The bi-directional tests and write functions of the open system, and the bar for a running test.
    pub(super) fn publish_ops(mut self: Pin<&mut Self>) {
        let (act, tests, writes_v) = {
            let r = self.rust();
            let vin = self.vin().to_string();
            let ecu = self.watched_ecu();
            let offered = ecu.as_ref();
            (
                actuator::running_json(r.act_status.as_ref(), &r.act_tests),
                offered.map_or_else(|| json!({}), |e| actuator::tests_json(e, &r.act_tests, r.act_status.as_ref())),
                offered.map_or_else(|| json!({}), |e| writes::writes_json(e, &r.write_fns, Some(vin.as_str()))),
            )
        };
        let selftests = {
            let r = self.rust();
            self.watched_ecu().and_then(|e| r.status_data.as_ref().and_then(|s| selftests_for(&s.self_tests, &e)).map(selftests_json)).unwrap_or(Value::Null)
        };
        self.as_mut().set_selftests(q(&selftests));
        self.as_mut().set_act(q(&act));
        self.as_mut().set_actuator(q(&tests));
        self.as_mut().set_writes(q(&writes_v));
    }

    pub fn act_start(self: Pin<&mut Self>, test: &QString, value: i32, ticked: bool) {
        let Some(ecu) = self.watched_ecu() else { return };
        let key = test.to_string();
        let params = actuator::start_params(&ecu, &self.rust().act_tests, &key, value as i64, ticked);
        let params = match params { Ok(p) => p, Err(e) => return self.say(ERROR, "act", "Test not started", &e, 10.0) };
        let name = self.rust().act_tests.iter().find(|t| t.key == key).map(|t| t.name.clone()).unwrap_or_default();
        let failed = name.clone();
        self.rpc("actuator_start", params, move |mut m, r| match r {
            Ok(v) => {
                m.as_mut().rust_mut().act_status = parse(v).ok();
                m.as_mut().publish_ops();
                m.say(INFO, "act", &format!("{name} started"), "Control goes back to the car at the time limit or when you stop it.", 5.0);
            }
            Err(e) => m.say(ERROR, "act", &format!("{failed} not started"), &e, 10.0),
        });
    }

    pub fn act_stop(self: Pin<&mut Self>) {
        self.rpc("actuator_stop", json!({}), |mut m, r| match r {
            Ok(v) => { m.as_mut().rust_mut().act_status = parse(v).ok(); m.publish_ops(); }
            Err(e) => m.say(ERROR, "act", "Could not stop the test", &e, 12.0),
        });
    }

    pub fn write_run(mut self: Pin<&mut Self>, key: &QString, ticked: bool) {
        let Some(ecu) = self.watched_ecu() else { return };
        if !self.write_running().is_empty() { return; } // one write at a time
        let key = key.to_string();
        let vin = self.vin().to_string();
        let params = writes::run_params(&ecu, &self.rust().write_fns, &key, Some(vin.as_str()), ticked);
        let params = match params { Ok(p) => p, Err(e) => return self.say(ERROR, "write", "Write not run", &e, 15.0) };
        let name = self.rust().write_fns.iter().find(|w| w.key == key).map(|w| w.name.clone()).unwrap_or_default();
        self.as_mut().set_write_running(QString::from(name.as_str()));
        self.rpc("write_run", params, move |mut m, r| {
            m.as_mut().set_write_running(QString::default());
            match r {
                Ok(v) => {
                    let res: WriteResult = parse(v).unwrap_or_default();
                    let text = writes::result_text(&res);
                    if res.ok { m.say(SUCCESS, "write", &format!("{name} complete"), &text, 10.0); }
                    else { m.say(WARN, "write", &format!("{name} did not confirm"), &text, 15.0); }
                }
                Err(e) => m.say(ERROR, "write", &format!("{name} not run"), &e, 15.0),
            }
        });
    }

    pub fn write_cancel(self: Pin<&mut Self>) { self.rpc("write_cancel", json!({}), |_, _| {}); }

    /// The answer to `recover`: whether the interrupted write or test was ended on the car.
    pub(super) fn on_recover(self: Pin<&mut Self>, spec: &str, r: Reply) {
        let n = crate::handback::after_recover(spec, r.map(|v| parse(v).unwrap_or_default()));
        self.say(n.level as i32, "handback", &n.title, &n.body, n.seconds);
    }
}
