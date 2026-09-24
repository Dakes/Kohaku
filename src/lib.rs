//! Kohaku, a tiny self-hosted bug report inbox.
//!
//! The binary is a thin wrapper around [`cli::main`]; everything else lives in this
//! library so integration tests reach it directly.

#![forbid(unsafe_code)]

#[cfg(all(feature = "dev", not(debug_assertions)))]
compile_error!("the dev feature is in a release build: build releases with default features only");

pub mod cli;
pub mod config;
pub mod keys;

#[cfg(test)]
pub mod test_support;
