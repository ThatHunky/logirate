//! logirate: switch a Logitech mouse's polling rate per foreground game.
//!
//! Everything is done in the mouse's volatile "host mode" over HID++ 2.0.
//! Nothing is written to the mouse's flash: power-cycling or replugging the
//! mouse always brings back its own onboard profile.

pub mod config;
pub mod foreground;
pub mod hidpp;
pub mod watch;
