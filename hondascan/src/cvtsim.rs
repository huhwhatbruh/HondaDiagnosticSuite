//! A strict fake of the CVT inclination-sensor learn across both of HDS's paths, for `SimEcu::with_cvt_isl`. The read side
//! of `CVT_ISL` (README "CVT inclination-sensor learn") is stage one only; this models what HDS's full procedure
//! (`sub_af69`, EAT_SOL_TST 4382-4465) expects of the CVT module, so a stage-two runner can be tried before any car.
//!
//! What HDS shows about `CVT,SLPLMOD` (DID 0x2340 byte 29), from `cvt_isl_74_20.$CODE` (4030-4121), `sub_b50c` (4530-4622) and
//! `sub_abf4` (4270-4380); nothing below was observed on a car:
//! - the learn starts from SLPLMOD 0-3 (the count-0 compare) or, for the second of two stages, 16;
//! - a finished learn reads 2 or 18 (`sub_abf4` accepts both), a reported error 3 or 19 with the code in SLPLERR (byte 30);
//! - after the ignition off/on HDS sends, a one-stage learn must read 0 (count 2), the first of two stages must read 16
//!   (count 18, then `0x1149` again as count 34) and the second must read 0 (count 35).
//!
//! Single stage: `0x1148` -> 2 -> ignition cycle -> 0. Two stages: `0x1149` -> 18 -> ignition cycle -> 16, then `0x1149` -> 2
//! -> ignition cycle -> 0. HDS accepts 2 or 18 for any finished learn, so which of the two the fake reports (18 after the
//! first of two stages, 2 otherwise) is an unconfirmed choice that only exercises both. The correction learn (`0x114a`,
//! `0x114b`) is not modelled: this car's `CALSLPS0` is 0, so HDS would not offer it. What happens to SLPLMOD 3 or 19 at an
//! ignition cycle is not in HDS's code; the fake leaves it unchanged.
use std::time::{Duration, Instant};

const ONE_STAGE: u16 = 0x1148;
const TWO_STAGE: u16 = 0x1149;

pub struct CvtIsl {
    /// Seconds from the start request to the finished state; 0 finishes at once.
    pub wait_s: f64,
    /// Some(code): the next learn ends in HDS's error state with this SLPLERR: SLPLMOD 19 for the first of two stages, 3 otherwise.
    pub fail_with: Option<u8>,
    slplmod: u8,
    slplerr: u8,
    running: Option<(Instant, u8)>, // (start, SLPLMOD it ends in)
}

impl CvtIsl {
    pub fn new(wait_s: f64) -> CvtIsl {
        CvtIsl { wait_s, fail_with: None, slplmod: 0, slplerr: 0, running: None }
    }

    fn settle(&mut self) {
        let Some((t, done)) = self.running else { return };
        if t.elapsed() < Duration::from_secs_f64(self.wait_s) {
            return;
        }
        self.running = None;
        match self.fail_with.take() {
            Some(code) => (self.slplmod, self.slplerr) = (done + 1, code),
            None => self.slplmod = done,
        }
    }

    /// (SLPLMOD, SLPLERR) as the DID shows them now.
    pub fn state(&mut self) -> (u8, u8) {
        self.settle();
        (self.slplmod, self.slplerr)
    }

    /// A RoutineControl request: the positive reply, or the negative response code. Strict like the other fakes: extended session only, exactly `31 01 <id>`, the engine stopped,
    /// no second start while one runs, and a start only from a state HDS would start it from. There is no stop (HDS sends none).
    pub fn control(&mut self, req: &[u8], extended: bool, engine_running: bool) -> Result<Vec<u8>, u8> {
        if req.len() < 4 {
            return Err(0x13);
        }
        if !extended {
            return Err(0x7F);
        }
        let rid = (req[2] as u16) << 8 | req[3] as u16;
        if rid != ONE_STAGE && rid != TWO_STAGE {
            return Err(0x31);
        }
        if req.len() != 4 {
            return Err(0x13);
        }
        if req[1] != 0x01 {
            return Err(0x12);
        }
        self.settle();
        if engine_running {
            return Err(0x22);
        }
        if self.running.is_some() {
            return Err(0x24);
        }
        let from_start = self.slplmod <= 3;
        let ok = if rid == ONE_STAGE { from_start } else { from_start || self.slplmod == 16 };
        if !ok {
            return Err(0x22);
        }
        let done = if rid == ONE_STAGE || self.slplmod == 16 { 2 } else { 18 };
        self.running = Some((Instant::now(), done));
        Ok(vec![0x71, 0x01, req[2], req[3]])
    }

    /// HDS's ignition off/on: a finished learn is committed (2 -> 0, 18 -> 16); one still running is abandoned.
    pub fn ignition_cycle(&mut self) {
        self.settle();
        self.running = None;
        match self.slplmod {
            2 => self.slplmod = 0,
            18 => self.slplmod = 16,
            _ => {}
        }
    }
}
