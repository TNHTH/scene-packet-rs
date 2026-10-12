//! ScenePacket 核心库：视频上下文提取、缓存、工具编排。

pub mod asr;
pub mod cache;
pub mod config;
pub mod error;
pub mod export;
pub mod extract;
pub mod ffmpeg;
pub mod fingerprint;
pub mod frames;
pub mod jobs;
pub mod keyframes;
pub mod media;
pub mod models;
pub mod packet;
pub mod paths;
pub mod pipeline;
pub mod process;
pub mod root;
pub mod timeline;
pub mod tools;

pub const VERSION: &str = "2.0.0";

pub use error::{unsupported_url, AvrError, ErrorCode};

pub mod storage;
