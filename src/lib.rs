//! Keyboard lighting control for ASUS TUF laptops on Linux.
//!
//! The protocol was recovered by reversing Armoury Crate's laptop HAL (AacNBDTHal) and the
//! machine's ACPI tables; see docs/REVERSING.md.

pub mod color;
pub mod config;
pub mod effects;
pub mod engine;
pub mod hw;
pub mod ipc;
pub mod sensors;

pub use color::Rgb;
