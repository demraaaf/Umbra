//! Umbra's recording engine: screen capture, hardware encoding and clip writing.

pub mod aac;
pub mod albums;
pub mod align;
pub mod apps;
pub mod audio;
pub mod autoclip;
pub mod benchmark;
pub mod buffer;
mod burn;
pub mod burnin;
pub mod camera;
pub mod capture;
pub mod cleanup;
pub mod clock;
pub mod controller;
pub mod cs2;
pub mod d3d;
pub mod defrag;
pub mod devices;
pub mod disk;
pub mod dshow;
pub mod encode;
pub mod estimate;
pub mod export;
pub mod gamedb;
pub mod gameload;
pub mod games;
pub mod hdr;
pub mod hotkey;
pub mod input;
pub mod inputdraw;
pub mod inputfile;
pub mod inputguard;
pub mod inputrules;
pub mod library;
pub mod loadtest;
pub mod mux;
pub mod naming;
pub mod others;
pub mod process;
pub mod processing;
pub mod recorder;
pub mod recordings;
pub mod render;
pub mod scale;
pub mod settings;
pub mod share;
pub mod snapshot;
pub mod sound;
pub mod spill;
pub mod startup;
pub mod timer;
pub mod unmix;
pub mod usage;
pub mod util;
pub mod waveform;
pub mod wnd;

pub use ffmpeg_next::Packet;

pub fn ffmpeg_version() -> String {
    ffmpeg_next::init().ok();
    unsafe {
        std::ffi::CStr::from_ptr(ffmpeg_next::ffi::av_version_info())
            .to_string_lossy()
            .into_owned()
    }
}

/// Keeps FFmpeg's own messages to errors. Without this it prints a warning for every clip with
/// named tracks ("UDTA parsing failed retrying raw"), which is FFmpeg complaining about the
/// way it writes track names itself; the names still read back correctly.
pub fn quiet_ffmpeg_logs() {
    ffmpeg_next::init().ok();
    ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Error);
}
