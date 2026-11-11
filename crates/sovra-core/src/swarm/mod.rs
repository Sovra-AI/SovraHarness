//! The sovereign enterprise swarm: a company's computers keep every document, model
//! and record on their own hardware, and still share an audit history and train
//! company-owned models together.
//!
//! Everything here is pure and deterministic. The pieces:
//!
//! - [`department`]: each business unit keeps custody of its memory; filtering
//!   happens before retrieval and before replication.
//! - [`ledger`] (over [`merkle`]): a shared, append-only, tamper-evident history of
//!   where every item lives and every model update, with witnesses.
//! - [`egress`]: nothing leaves a computer until a permitted person approves the
//!   exact bytes.
//! - [`learn`]: collective training without exporting data. `GaLore` low-rank
//!   projection, SCAFFOLD drift control, exact discrete-Gaussian differential
//!   privacy with a zCDP accountant, and pairwise-masked secure aggregation.
//!
//! Transport (iroh, QUIC, Ed25519) is `sovra-mesh`; signing and secure randomness
//! are behind ports implemented there. See `docs/swarm.md`.

pub mod audit;
pub mod department;
pub mod egress;
pub mod learn;
pub mod ledger;
pub mod merkle;

#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
