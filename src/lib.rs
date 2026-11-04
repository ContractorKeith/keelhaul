#![forbid(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

pub mod backend;
pub mod exit;
pub mod output;

#[cfg(test)]
mod vectors;
