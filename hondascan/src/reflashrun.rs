//! Running one reflash (reflash.rs): VIN match, the unlock, the erase, the per-block download/transfer, the finish
//! routine and the guaranteed return to the default session on any error.
//!
//! STATUS: tested against the strict fake ECU only (`sim.rs`, `SimEcu::with_reflash`); never run on a real car.
//! CLAUDE.md "Writing functions": the old value (a full ROM read) is not read and stored here because no backup
//! and no confirmed read route exist yet for this PCM (README.md "Reflash groundwork") -- this runner does not
//! claim to meet that condition. **No CLI command exists for this yet**; when one is built, it must refuse every
//! adapter but the simulator until a read route exists and the owner asks for a car run. Every frame is still
//! logged (`confirm` below needs an
//! attached capture, the same as `writerun.rs`), and on any error the session is returned to the default one (01)
//! so the ECU is not left waiting in the programming session; an error after the erase routine has run says so
//! plainly, because the ECU may hold an incomplete image at that point and the only known recovery is to run the
//! same flash again from the start.
use crate::honda::HondaCan;
use crate::ident::{vin_of, VIN_DID};
use crate::reflash::{FlashBlock, FlashTarget};
use crate::uds::UdsError;
use serde_json::{json, Value};
use std::fmt;

#[derive(Debug)]
pub enum RunError {
    /// Refused before anything was sent to the ECU: a bad argument, a missing log, a VIN mismatch, a block outside
    /// the target's range.
    Refused(String),
    /// Something was sent and failed. `erased` is true once the erase routine has run: the ECU may hold an
    /// incomplete image, so the message says so and names the only known recovery (run the flash again).
    Failed { erased: bool, message: String },
    Uds(UdsError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RunError::Refused(s) => write!(f, "{s}"),
            RunError::Failed { message, .. } => write!(f, "{message}"),
            RunError::Uds(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<UdsError> for RunError {
    fn from(e: UdsError) -> Self {
        RunError::Uds(e)
    }
}

pub struct ReflashRun {
    pub address: u32,
    pub target: FlashTarget,
    pub blocks: Vec<FlashBlock>,
    blocks_total: usize,
    pub blocks_done: usize,
    pub errors: Vec<String>,
}

impl ReflashRun {
    pub fn new(address: u32, target: FlashTarget, blocks: Vec<FlashBlock>) -> ReflashRun {
        ReflashRun { address, target, blocks_total: blocks.len(), blocks, blocks_done: 0, errors: vec![] }
    }

    /// Run the whole sequence once. `vin` is the person's assertion of the car's VIN; the ECU's F190 answer must
    /// match it, the same rule `writerun.rs` enforces.
    pub fn run(&mut self, hc: &mut HondaCan, vin: &str, confirm: bool) -> Result<Value, RunError> {
        if !confirm {
            return Err(RunError::Refused("a reflash needs confirm: true".into()));
        }
        if !hc.capture.as_ref().is_some_and(|c| c.is_attached() && !c.capped()) {
            return Err(RunError::Refused("a reflash needs a frame log (--log FILE): every frame must be recorded, and this capture is not attached to a file or already hit its cap".into()));
        }
        if self.blocks.is_empty() {
            return Err(RunError::Refused("no blocks to flash".into()));
        }
        for b in &self.blocks {
            crate::reflash::check_bounds(&self.target, b.address, b.data.len() as u32).map_err(|e| RunError::Refused(e.0))?;
        }
        let vin = vin.trim().to_ascii_uppercase();
        if vin.is_empty() {
            return Err(RunError::Refused("a reflash needs the car's VIN: pass --vin VIN (scan prints it)".into()));
        }
        let reported = hc.client(self.address).read_did(VIN_DID)?;
        let reported = vin_of(&reported).ok_or_else(|| RunError::Refused("the ECU did not return a VIN; refusing a reflash without the VIN match".into()))?;
        if reported != vin {
            return Err(RunError::Refused(format!("VIN mismatch: the car reports {reported}, the command says {vin}")));
        }

        hc.note(&format!("reflash {} ({} block(s), {} bytes total): start", self.target.key, self.blocks.len(), self.blocks.iter().map(|b| b.data.len()).sum::<usize>()));
        hc.client(self.address).reflash_target = Some(self.target);
        let r = self.sequence(hc);
        hc.client(self.address).reflash_target = None;
        r
    }

    fn sequence(&mut self, hc: &mut HondaCan) -> Result<Value, RunError> {
        let addr = self.address;
        if let Err(e) = hc.client(addr).security_access() {
            return Err(self.abort(hc, false, format!("SecurityAccess 27 01/02: {e}")));
        }
        if let Err(e) = hc.client(addr).session(0x02) {
            return Err(self.abort(hc, false, format!("session 10 02: {e}")));
        }
        if let Err((erased, e)) = self.erase(hc) {
            return Err(self.abort(hc, erased, format!("erase: {e}")));
        }
        let blocks = std::mem::take(&mut self.blocks);
        for (i, b) in blocks.iter().enumerate() {
            if let Err(e) = self.send_block(hc, b) {
                return Err(self.abort(hc, true, format!("block {i} (0x{:08X}, {} bytes): {e}", b.address, b.data.len())));
            }
            self.blocks_done += 1;
        }
        if let Err(e) = hc.client(addr).reflash_finish() {
            return Err(self.abort(hc, true, format!("finish 31 01 FF 01: {e}")));
        }
        if let Err(e) = hc.client(addr).session(0x01) {
            self.errors.push(format!("session 01: {e}"));
        }
        hc.note(&format!("reflash {}: all {} block(s) sent, finished", self.target.key, self.blocks_total));
        Ok(self.status(true))
    }

    /// The begin marker, the erase routine itself, then the end marker. `Err((erased, e))`: `erased` is true once
    /// the destructive erase routine has actually run, even when a later step (the end marker) is what failed --
    /// `abort` needs that distinction to name the right recovery (CLAUDE.md's writing rule).
    fn erase(&mut self, hc: &mut HondaCan) -> Result<(), (bool, UdsError)> {
        let addr = self.address;
        hc.client(addr).reflash_erase_marker(true).map_err(|e| (false, e))?;
        hc.client(addr).reflash_erase().map_err(|e| (false, e))?;
        hc.client(addr).reflash_erase_marker(false).map_err(|e| (true, e))?;
        Ok(())
    }

    /// RequestDownload, then TransferData chunks of at most the negotiated length (the block counter resets to 1
    /// per block and wraps `0xFF -> 0x00`, README.md "Reflash groundwork"), then RequestTransferExit.
    fn send_block(&mut self, hc: &mut HondaCan, b: &FlashBlock) -> Result<(), UdsError> {
        let addr = self.address;
        let max_block = hc.client(addr).request_download(b.address, b.data.len() as u32)? as usize;
        let mut counter: u8 = 1;
        let mut sent = 0usize;
        while sent < b.data.len() {
            let n = max_block.min(b.data.len() - sent);
            hc.client(addr).transfer_data(counter, &b.data[sent..sent + n])?;
            sent += n;
            counter = counter.wrapping_add(1);
        }
        hc.client(addr).request_transfer_exit()?;
        Ok(())
    }

    /// Return to the default session on any error, so the ECU is not left in the programming one. `erased`: the
    /// erase routine had already run, so the ECU may hold an incomplete image -- the message says so and names the
    /// only known recovery (CLAUDE.md requires a stated recovery path before any write; a verified ROM read before
    /// this reflash is the missing half of that, see the module doc).
    fn abort(&mut self, hc: &mut HondaCan, erased: bool, message: String) -> RunError {
        let message = if erased {
            format!("{message}; the ECU has been erased and may hold an incomplete program image -- do not power it off; the only known recovery is to run this flash again from the start")
        } else {
            message
        };
        hc.note(&format!("reflash {} aborted: {message}", self.target.key));
        if let Err(e) = hc.client(self.address).session(0x01) {
            self.errors.push(format!("session 01 (hand-back after abort): {e}"));
        }
        RunError::Failed { erased, message }
    }

    pub fn status(&self, ok: bool) -> Value {
        json!({
            "target": self.target.key,
            "ok": ok,
            "blocks_done": self.blocks_done,
            "blocks_total": self.blocks_total,
            "errors": self.errors,
        })
    }
}
