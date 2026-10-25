//! What comes back from the backend: replies to the calls in flight, and the events it sends on its own.
use super::*;

impl App {
    fn finish(&mut self, ctx: &egui::Context, kind: Job, r: Reply) {
        match (kind, r) {
            (Job::Cleared(e), r) => {
                let name = crate::systems::info(&e.system_type, &e.display).name;
                match r {
                    Ok(_) => self.toasts.push(ctx, "clear", Level::Success, &format!("Codes cleared in {name}"), "", 5.0),
                    Err(err) => self.toasts.push(ctx, "clear", Level::Error, "Clear failed", &err, 8.0),
                }
                if self.link != Link::Connected { return; }
                // every system at that address: the control unit cleared them all (or none), see App::clear
                for x in self.ecus.iter().filter(|x| x.address == e.address).cloned().collect::<Vec<_>>() { self.read_dtcs(&x); }
            }
            (Job::Ports, Ok(v)) => {
                let ports: Vec<Port> = crate::api::parse(v).unwrap_or_default();
                self.adapters = crate::ui::topbar::adapter_list(&ports);
                self.pick_saved();
            }
            (Job::Hello, Ok(v)) => {
                let h: Hello = crate::api::parse(v).unwrap_or_default();
                self.backend_version = h.version.clone();
                if !h.data_dir.is_empty() {
                    let st = crate::settings::Store::open(&h.data_dir);
                    self.custom = if st.last.custom.is_empty() { self.custom.clone() } else { st.last.custom.clone() };
                    self.settings = Some(st);
                    self.settings_writable = self.settings.as_ref().is_some_and(|s| s.writable());
                    self.pick_saved();
                }
                self.pending_recovery = h.pending_recovery.clone();
                if let Some(p) = &self.pending_recovery {
                    let what = if p.kind == "actuator" { "a bi-directional test" } else { "a write" };
                    let which = if p.name.is_empty() { String::new() } else { format!(" ({})", p.name) };
                    let car = if p.vin.is_empty() { "the car".to_string() } else { p.vin.clone() };
                    self.toasts.push(ctx, "pending-recovery", Level::Warn, "A write was interrupted",
                        &format!("The app stopped while {what}{which} was in flight on {car}. The backend hands control back when it connects."), 20.0);
                }
                // check this computer at every start (offline, instant); the welcome screen shows the answer
                self.run_self_check();
                if let Some(a) = h.autoconnect { self.autoconnect = Some(a.clone()); self.connect(&a); }
            }
            // the backend did not start (hondascan-rpc not found or not started): say so now, not at the first Connect
            (j @ (Job::Hello | Job::Ports), Err(e)) => {
                if matches!(j, Job::Hello) { self.self_check_err = Some(e.clone()); }
                self.toasts.push(ctx, "exit", Level::Error, "The scanner backend did not start", &e, 30.0)
            }
            // a reply that arrives after Disconnect (or a backend restart) belongs to a link that is gone
            (Job::Connect(_), _) if self.link != Link::Connecting => {}
            (Job::Scan | Job::Dtcs(_), _) if self.link != Link::Connected => self.scanning = false,
            (Job::Connect(_), Ok(v)) => {
                let c: Connected = crate::api::parse(v).unwrap_or_default();
                self.link = Link::Connected;
                if let Some(w) = c.warning { self.toasts.push(ctx, "warn-adapter", Level::Warn, "Another program may disturb the adapter", &w, 20.0); }
                if let Some(rec) = c.recovery.filter(|r| r.pending) {
                    if rec.handed_back {
                        let sent = if rec.actions.is_empty() { String::new() } else { format!(" Sent: {}.", rec.actions.join(", ")) };
                        self.toasts.push(ctx, "handback", Level::Success, "Control handed back to the car",
                            &format!("A write or test was interrupted when the backend last stopped; its hand-back has now run.{sent}"), 15.0);
                    } else {
                        let err = rec.error.unwrap_or_else(|| "no requests were sent, and the backend did not say why".into());
                        self.toasts.push(ctx, "handback", Level::Error, "Control was not handed back",
                            &format!("{err}. The interrupted operation is kept; fix the cause and reconnect."), 30.0);
                    }
                }
                self.scan();
            }
            (Job::Connect(spec), Err(e)) => {
                self.link = Link::Idle;
                self.toasts.push(ctx, "connect", Level::Error, &format!("Could not open {spec}"), &e, 8.0);
            }
            (Job::Scan, Ok(v)) => {
                let s: ScanResult = crate::api::parse(v).unwrap_or_default();
                self.scanning = false;
                self.vin = (s.vin != "UNKNOWN" && !s.vin.is_empty()).then(|| s.vin.clone());
                self.scan_seconds = Some(s.seconds);
                self.ecus = s.ecus;
                let keys: Vec<String> = self.ecus.iter().map(Ecu::key).collect();
                self.dtcs.retain(|k, _| keys.contains(k)); // a rescan that no longer finds a system drops its codes
                let n = self.ecus.len();
                self.toasts.push(ctx, "scan", Level::Success, &format!("{n} system{} found", if n == 1 { "" } else { "s" }),
                    &format!("Recording session {}", s.session), 5.0);
                for e in self.ecus.clone() { self.read_dtcs(&e); }
                for def in &glance::DEFS {
                    if let Some(e) = self.ecus.iter().find(|e| def.types.contains(&e.system_type.as_str()) && !e.system.is_empty()).cloned() {
                        if !self.catalogs.contains_key(&e.system) { self.job(Job::Catalog(e.system.clone()), "catalog", json!({ "system": e.system })); }
                    }
                }
                self.reset_live();
                self.start_live();
                self.job(Job::ActTests, "actuator_tests", json!({}));
                self.job(Job::Writes, "write_functions", json!({}));
                // the frozen baselines and service markers belong to the car just found; so does its write audit
                self.refresh_maintenance();
                self.load_audit();
                // HONDASCAN_OPEN=<system type>[:codes|tests|writes|data|about]: open a system at start-up (for screenshots)
                if let Ok(v) = std::env::var("HONDASCAN_OPEN") {
                    if v == "history" { self.view = View::History; }
                    let (t, tab) = v.split_once(':').unwrap_or((&v, "live"));
                    if let Some(e) = self.ecus.iter().find(|e| e.system_type == t) { self.view = View::System(e.key()); self.pending_tab = Some(tab.to_string()); }
                }
            }
            (Job::Scan, Err(e)) => { self.scanning = false; self.toasts.push(ctx, "scan", Level::Error, "Scan failed", &e, 8.0); }
            (Job::Status, Ok(v)) => {
                self.status = crate::api::parse(v).ok();
                if let Some(a) = self.status.as_ref().and_then(|s| s.actuator.clone()) { if a.active || self.act.as_ref().is_some_and(|x| x.active) { self.act = Some(a); } }
            }
            (Job::Dtcs(k), r) => {
                let r = r.and_then(|v| crate::api::parse::<DtcReply>(v).map_err(|e| format!("unreadable reply: {e}")));
                self.dtcs.entry(k).or_default().take(r);
            }
            (Job::ActTests, Ok(v)) => self.act_tests = crate::api::parse(v).unwrap_or_default(),
            (Job::Writes, Ok(v)) => self.writes = crate::api::parse(v).unwrap_or_default(),
            (Job::WriteRun(name), Ok(v)) => {
                let r: WriteResult = crate::api::parse(v).unwrap_or_default();
                self.last_write = Some(r.clone());
                let text = crate::ui::write::result_text(&r);
                if r.ok {
                    self.toasts.push(ctx, "write", Level::Success, &format!("{name} complete"), &text, 10.0);
                } else {
                    self.toasts.push(ctx, "write", Level::Warn, &format!("{name} did not confirm"), &text, 15.0);
                }
            }
            (Job::WriteRun(name), Err(e)) => self.toasts.push(ctx, "write", Level::Error, &format!("{name} not run"), &e, 15.0),
            (Job::ActStart(name), Ok(v)) => { self.act = crate::api::parse(v).ok(); self.toasts.push(ctx, "act", Level::Info, &format!("{name} started"), "Control goes back to the car at the time limit or when you stop it.", 5.0); }
            (Job::ActStart(name), Err(e)) => self.toasts.push(ctx, "act", Level::Error, &format!("{name} not started"), &e, 10.0),
            (Job::ActStop, Ok(v)) => self.act = crate::api::parse(v).ok(),
            (Job::ActStop, Err(e)) => self.toasts.push(ctx, "act", Level::Error, "Could not stop the test", &e, 12.0),
            (Job::Catalog(sys), Ok(v)) => {
                // converting the engine's 759 signals took 6-9 ms, a dropped frame when a system opened: done on a thread
                let (tx, ctx, sys) = (self.catalog_tx.clone(), ctx.clone(), sys.clone());
                std::thread::spawn(move || { if let Ok(c) = crate::api::parse::<Catalog>(v) { let _ = tx.send((sys, c)); ctx.request_repaint(); } });
            }
            (Job::SelfCheck, Ok(v)) => {
                let sc: SelfCheck = crate::api::parse(v).unwrap_or_default();
                let ok = crate::ui::selfcheck::passed(&sc, self.settings_writable);
                if let Some(st) = &mut self.settings {
                    let mut s = st.last.clone();
                    s.self_check_at = crate::ui::selfcheck::unix_now();
                    s.self_check_ok = ok;
                    st.save(&s);
                }
                self.self_check = Some(sc);
                self.self_check_err = None;
            }
            (Job::SelfCheck, Err(e)) => {
                self.self_check = None;
                self.self_check_err = Some(e.clone());
            }
            (Job::Maintenance(_vin), Ok(v)) => match crate::api::parse::<Maintenance>(v) {
                Ok(m) => { self.maintenance = Some(m); self.maintenance_err = None; }
                Err(e) => self.maintenance_err = Some(format!("unreadable maintenance reply: {e}")),
            },
            (Job::Maintenance(_), Err(e)) => self.maintenance_err = Some(e),
            (Job::Freeze(_vin), Ok(v)) => {
                let f: FreezeReply = crate::api::parse(v).unwrap_or_default();
                if f.frozen {
                    if let Some(b) = f.baseline { if let Some(m) = &mut self.maintenance { m.latest = Some(b); } }
                    self.toasts.push(ctx, "freeze", Level::Success, "Reference snapshot saved",
                        "The health card and the next report compare the car against this one. It cannot be edited or removed.", 10.0);
                } else {
                    self.toasts.push(ctx, "freeze", Level::Warn, "The snapshot was not saved",
                        "The car no longer met the conditions. Read the codes again and check it has no active trouble codes.", 15.0);
                }
                self.refresh_maintenance();
                self.history.reload_data(&self.be);
            }
            (Job::Freeze(_), Err(e)) => self.toasts.push(ctx, "freeze", Level::Error, "Could not save the snapshot", &e, 15.0),
            (Job::MarkerAdd, Ok(_)) => {
                self.toasts.push(ctx, "marker", Level::Success, "Service marker added", "It is kept with this car's history.", 6.0);
                self.refresh_maintenance();
                self.history.reload_data(&self.be);
            }
            (Job::MarkerAdd, Err(e)) => self.toasts.push(ctx, "marker", Level::Error, "Could not add the service marker", &e, 12.0),
            (Job::MarkerRemove(_id), Ok(_)) => {
                self.toasts.push(ctx, "marker", Level::Info, "Service marker removed", "", 6.0);
                self.refresh_maintenance();
                self.history.reload_data(&self.be);
            }
            (Job::MarkerRemove(_), Err(e)) => self.toasts.push(ctx, "marker", Level::Error, "Could not remove the service marker", &e, 12.0),
            (Job::Report, Ok(v)) => {
                let r: ReportReply = crate::api::parse(v).unwrap_or_default();
                let verdict = crate::ui::health::verdict_label(&r.verdict);
                let text = match (r.summary.is_empty(), r.path.is_empty()) {
                    (false, false) => format!("{}\n{}", r.summary, r.path),
                    (false, true) => r.summary.clone(),
                    (true, false) => r.path.clone(),
                    (true, true) => "The backend did not say where the report was written.".into(),
                };
                self.toasts.push(ctx, "report", Level::Success, &format!("Health report: {verdict}"), &text, 15.0);
                self.last_report = Some(r);
            }
            (Job::Report, Err(e)) => self.toasts.push(ctx, "report", Level::Error, "Could not write the health report", &e, 15.0),
            (Job::Export, Ok(v)) => {
                let e: ExportReply = crate::api::parse(v).unwrap_or_default();
                let text = format!("Backup: {}\nJSON: {}", e.backup_path, e.json_path);
                self.toasts.push(ctx, "export", Level::Success, "Data exported", &text, 15.0);
                self.last_export = Some(e);
            }
            (Job::Export, Err(e)) => self.toasts.push(ctx, "export", Level::Error, "Could not export the data", &e, 15.0),
            (Job::Audit(_vin), Ok(v)) => self.audit = crate::api::parse::<AuditReply>(v).map(|a| a.records).unwrap_or_default(),
            (Job::Recover(spec), Ok(v)) => {
                let r: Recovery = crate::api::parse(v).unwrap_or_default();
                if !r.pending {
                    self.toasts.push(ctx, "handback", Level::Info, "Nothing to hand back",
                        &format!("{spec} had no interrupted write or test recorded."), 8.0);
                } else if r.handed_back {
                    let sent = if r.actions.is_empty() { String::new() } else { format!(" Sent: {}.", r.actions.join(", ")) };
                    self.toasts.push(ctx, "handback", Level::Success, "Control handed back to the car",
                        &format!("The interrupted operation was ended on {spec}.{sent}"), 15.0);
                } else {
                    let err = r.error.unwrap_or_else(|| "no requests were sent, and the backend did not say why".into());
                    self.toasts.push(ctx, "handback", Level::Error, "Control was not handed back",
                        &format!("{spec}: {err}. The operation is kept; fix the cause and connect again."), 30.0);
                }
            }
            (Job::Recover(spec), Err(e)) => self.toasts.push(ctx, "handback", Level::Error, "Could not hand control back",
                &format!("{spec}: {e}. The interrupted operation is kept until a hand-back succeeds."), 30.0),
            _ => {}
        }
    }

    /// Start each glance's poll once its system's catalog is here: only the groups it shows.
    fn start_live(&mut self) {
        for g in &mut self.glances {
            if g.live.is_some() { continue; }
            let Some(e) = self.ecus.iter().find(|e| g.def.types.contains(&e.system_type.as_str()) && !e.system.is_empty()).cloned() else { continue };
            let Some(cat) = self.catalogs.get(&e.system) else { continue };
            let mut dids: Vec<u32> = g.def.hero.iter().chain(g.def.gauges).chain(g.def.tiles).filter_map(|s| cat.find(s).map(|f| f.0)).collect();
            dids.sort_unstable();
            dids.dedup();
            g.live = Some(Live::new(e, dids));
        }
    }

    /// A new ECU set invalidates every live poll and the open system view: the next frames rebuild them.
    fn reset_live(&mut self) {
        for g in &mut self.glances {
            g.live = None;
        }
        self.sysui = None;
    }

    pub(super) fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.ev.try_recv() {
            match ev {
                Event::Backend { event, data } => self.on_event(ctx, &event, data),
                Event::Exit(why) => {
                    // a write or test that was in flight when the backend died has to be handed back before the car is
                    // forgotten: the reply to `recover` shows whether it worked, and the backend starts a fresh process
                    let writing = self.write_running();
                    let actuating = self.act.as_ref().is_some_and(|a| a.active);
                    let spec = self.settings.as_ref().map(|s| s.last.adapter.clone()).filter(|s| !s.is_empty()).or_else(|| Some(self.adapter.clone()));
                    // before the car is forgotten, as the contract asks: the adapter spec is all `recover` needs
                    if writing.is_some() || actuating {
                        if let Some(spec) = spec.filter(|s| !s.is_empty()) {
                            self.job(Job::Recover(spec.clone()), "recover", json!({ "adapter": spec }));
                        }
                    }
                    self.link = Link::Idle;
                    self.forget();
                    self.toasts.push(ctx, "exit", Level::Error, "Backend stopped", &why, 12.0);
                }
            }
        }
        while let Ok((sys, c)) = self.catalog_rx.try_recv() { self.catalogs.insert(sys, std::sync::Arc::new(c)); self.start_live(); }
        let mut done = Vec::new();
        for (i, (_, rx)) in self.jobs.iter().enumerate() {
            match rx.try_recv() {
                Ok(r) => done.push((i, r)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => done.push((i, Err("backend stopped".into()))),
                Err(_) => {}
            }
        }
        for (i, r) in done.into_iter().rev() {
            let (kind, _) = self.jobs.remove(i);
            self.finish(ctx, kind, r);
        }
        if self.link == Link::Connected && self.status_at.elapsed() > Duration::from_secs(2)
            && !self.jobs.iter().any(|(k, _)| matches!(k, Job::Status)) {
            self.status_at = Instant::now();
            self.job(Job::Status, "status", json!({}));
        }
        for g in &mut self.glances { if let Some(l) = &mut g.live { l.pump(&self.be, Duration::from_millis(250)); } }
        self.sync_system();
        self.remember();
        if let Some(s) = &mut self.sysui {
            s.pump();
            if s.tab == Tab::Live { s.live.live.pump(&self.be, Duration::from_millis(200)); }
        }
    }

    fn on_event(&mut self, ctx: &egui::Context, event: &str, data: serde_json::Value) {
        match event {
            "car" => {
                let off = data["state"] == "off";
                self.toasts.push(ctx, "car", Level::Info, if off { "Car is off: session saved" } else { "Car is on: new session" },
                    if off { "Watching for the ignition to come back." } else { "" }, 6.0);
            }
            "actuator" => if let Ok(a) = crate::api::parse::<ActStatus>(data) {
                if !a.active && !a.reason.is_empty() && self.act.as_ref().is_some_and(|x| x.active) {
                    let bad = !a.errors.is_empty();
                    self.toasts.push(ctx, "act-end", if bad { Level::Error } else { Level::Info }, &format!("{} ended", a.name),
                        &format!("Control is back with the car: {}.{}", a.reason, if bad { format!(" {}", a.errors.join("; ")) } else { String::new() }), 10.0);
                }
                self.act = Some(a);
            },
            "error" => self.toasts.push(ctx, "backend-error", Level::Error, "Background read failed", data["message"].as_str().unwrap_or(""), 8.0),
            "codes" => {
                let (a, s) = (data["address"].as_u64().unwrap_or(0) as u32, data["system"].as_str().unwrap_or(""));
                if let Some(e) = self.ecus.iter().find(|e| e.address == a && e.system == s).cloned() {
                    let Ok(list) = crate::api::parse::<Vec<Dtc>>(data["dtcs"].clone()) else { return };
                    let before = self.dtcs.get(&e.key()).and_then(|d| d.dtcs.clone());
                    let added: Vec<String> = before.map(|b| list.iter().filter(|d| !b.iter().any(|x| x.code == d.code && x.failure_type == d.failure_type))
                        .map(Dtc::full).collect()).unwrap_or_default();
                    if !added.is_empty() {
                        let name = crate::systems::info(&e.system_type, &e.display).name;
                        self.toasts.push(ctx, &format!("code-{}", e.key()), Level::Warn, &format!("New code in {name}"), &added.join(", "), 10.0);
                    }
                    // a read or clear of ours may still be in flight: its reply settles `loading`. The monitor pushes only
                    // the code list: the last read's self-tests-not-run records and status bits stay until the next read.
                    let d = self.dtcs.entry(e.key()).or_default();
                    d.dtcs = Some(list);
                    d.error = None;
                }
            }
            "session" if data["state"] == "started" => {
                self.dtcs.clear();
                if let Ok(s) = crate::api::parse::<ScanResult>(data) { self.ecus = s.ecus; }
                self.reset_live();
                self.start_live();
                for e in self.ecus.clone() { self.read_dtcs(&e); }
            }
            _ => {}
        }
    }

}
