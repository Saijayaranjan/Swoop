//! Site Grabber: a bounded, robots-aware HTML crawler that discovers downloadable files.
//!
//! The crawler streams every page through `lol_html` (bounded memory), normalises and dedups
//! URLs, respects scope/depth/page limits and per-host politeness, and classifies discovered
//! links into files (by extension/MIME) and pages (crawled further). Discovered files can be
//! probed with `HEAD` for size and MIME so the UI can filter before downloading anything.

pub mod classify;
pub mod crawler;
pub mod extract;
pub mod robots;
pub mod types;
pub mod urlnorm;

pub use crawler::{Crawler, Session};
pub use types::{GrabberFile, GrabberOptions, GrabberSession};
