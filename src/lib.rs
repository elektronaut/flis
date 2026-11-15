//! snags records what slowed down development, so agents can look up known
//! problems and close them once fixed. See the README for the commands.

pub mod cli;
pub mod clock;
pub mod commands;
pub mod context;
pub mod db;
pub mod error;
pub mod import;
pub mod mask;
pub mod model;
pub mod output;
pub mod rank;
pub mod search;
pub mod store;
