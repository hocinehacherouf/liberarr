//! Adapters implementing the core ports: SQLite, BLE, HTTP AI client, filesystem.

pub mod agent_cli;
pub mod ai;
pub mod audio_prep;
pub mod backup;
pub mod ble;
pub mod files;
mod http;
pub mod keystore;
pub mod mistral;
pub mod model_list;
pub mod plaud_cloud;
pub mod sqlite;
