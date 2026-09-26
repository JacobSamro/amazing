//! Origin-partitioned storage: Web Storage, IndexedDB and Cache API engines.
//!
//! Std-only engine; bindings adopt it through the driver.
#![forbid(unsafe_code)]
pub mod buckets;
pub mod cache;
pub mod core;
pub mod idb;
pub mod locks;
pub mod manager;
#[cfg(test)]
mod tests;
pub mod web;
