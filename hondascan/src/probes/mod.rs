//! The research tools that used to be Python scripts in hondascan/tools/: `drive_series` (a recording's data as a time series),
//! `abs_probe` (what makes the ABS/EPS/TPMS answer on the DLC) and `elm_addressing` (which header style an ELM clone accepts).
//! Each has a thin binary in src/bin/.
pub mod abs_probe;
pub mod diagh_session;
pub mod drive_series;
pub mod elm_addressing;
// the GD101 probes use termios2 and /proc/<pid>/mem of a Wine process: Linux only
#[cfg(target_os = "linux")]
pub mod gd101_hello;
#[cfg(target_os = "linux")]
pub mod gd101_pt_dump;
pub mod profile_from_capture;
