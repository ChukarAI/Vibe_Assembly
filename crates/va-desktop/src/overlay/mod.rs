pub mod model;
pub mod raster;

#[cfg(windows)]
mod win32;
#[cfg(not(windows))]
#[path = "win32_stub.rs"]
mod win32;

pub use win32::run_overlay_demo;
