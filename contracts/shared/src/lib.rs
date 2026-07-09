#![no_std]

//! Common types shared across Astraguard contracts: admin/oracle access
//! control and a timelocked admin-handover flow, so admin changes take
//! effect only after a delay users can see and react to.

pub mod access;
pub mod timelock;
pub mod ttl;
