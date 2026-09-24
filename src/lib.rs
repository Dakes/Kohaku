//! Kohaku, a tiny self-hosted bug report inbox.
//!
//! The binary is a thin wrapper around [`cli::main`]; everything else lives in this
//! library so integration tests reach it directly.

#![forbid(unsafe_code)]

#[cfg(all(feature = "dev", not(debug_assertions)))]
compile_error!("the dev feature is in a release build: build releases with default features only");

pub mod admin;
pub mod assets;
pub mod audit;
pub mod auth;
pub mod cli;
pub mod config;
pub mod db;
pub mod dev;
pub mod healthcheck;
pub mod http;
pub mod jobs;
pub mod keys;
pub mod limits;
pub mod logging;
pub mod mail;
pub mod pages;
pub mod routing;
pub mod serve;
pub mod time;

#[cfg(test)]
pub mod test_support;
