//! FTP/FTPS download engine: a small, Tokio-native FTP client (USER/PASS, FEAT, TYPE I, EPSV/PASV,
//! SIZE, MDTM, REST, RETR, LIST/MLSD, ABOR, AUTH TLS/PBSZ/PROT) and a `Transfer` implementation
//! with resume, pause and rate limiting.

pub mod classify;
pub mod client;
pub mod engine;
pub mod listing;

pub use client::FtpEntry;
pub use engine::FtpEngine;
