//! net80211 - FreeBSD net80211 stack ported to Redox OS
//!
//! This crate provides a Rust implementation of the FreeBSD net80211
//! 802.11 stack, ported to Redox OS with Rust.
//!
//! Original source: FreeBSD src/sys/net80211/
//! License: BSD-3-Clause (compatible with MIT)

#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod ieee80211;
pub mod link;
pub mod crypto;
pub mod ratectl;
pub mod regulatory;
pub mod getrandom;
// pub mod node;
// pub mod vap;
// pub mod input;
// pub mod output;
// pub mod proto;
// pub mod scan;
// pub mod auth;
// pub mod assoc;
// pub mod wme;
// pub mod htc;
// pub mod action;
// pub mod mesh;
// pub mod tdls;
// pub mod sona;
// pub mod dfs;
// pub mod wnm;
// pub mod rrm;
// pub mod fst;
// pub mod ome;
// pub mod mu_mimo;
// pub mod he;
// pub mod eht;

pub use ieee80211::*;

/// Initialize the 802.11 stack
pub fn init() {
    regulatory::init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init() {
        init();
    }
}