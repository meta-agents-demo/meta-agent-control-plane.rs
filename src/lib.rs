#![forbid(unsafe_code)]

pub mod auth;
pub mod client;
pub mod config;
pub mod coordination;
pub mod daemon;
pub mod http;
pub mod metacognition;
pub mod metacognition_api;
pub mod metacognition_ui;
pub mod model;
pub mod openapi;
pub mod provider;
pub mod store;
pub mod tcp;
pub mod udp;
pub mod ui;
pub mod web;

pub use config::Config;
pub use daemon::Daemon;
