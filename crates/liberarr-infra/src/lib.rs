//! Adapters implementing the core ports: SQLite, BLE, HTTP AI client, filesystem.

pub mod ai;
pub mod anthropic;
pub mod backup;
pub mod ble;
pub mod claude_cli;
pub mod files;
mod http;
pub mod keystore;
pub mod mistral;
pub mod model_list;
pub mod plaud_cloud;
pub mod sqlite;
