//! MCP stdio 服务。

use avr_core::error::AvrError;
use avr_core::pipeline::{
    analyze_url, compact_mcp_payload, full_mcp_payload, transcribe_local, PipelineContext,
};
use avr_core::root::resolve_data_root;
use rmcp::transport::stdio;
use rmcp::{
    handler::server::router::tool::ToolRouter,
    handler::server::tool::Parameters,
    model::{
        CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    schemars, tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use serde::Deserialize;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
struct AvrMcpServer {
    ctx: Arc<PipelineContext>,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AnalyzeMediaParams {
    source: String,
    #[serde(default = "default_compact")]
    response_mode: String,
    #[serde(default = "default_max_inline")]
    max_inline_chars: usize,
}

fn default_compact() -> String {
    "compact".into()
}
fn default_max_inline() -> usize {
    3000
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListHintsParams {
    media_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExtractFramesParams {
    media_key: String,
    times: Vec<f64>,
    #[serde(default = "default_delivery")]
    delivery: String,
    width: Option<u32>,
}

fn default_delivery() -> String {
    "path".into()
}

#[tool_router]
impl AvrMcpServer {
    fn new(ctx: PipelineContext) -> Self {
        Self {
            ctx: Arc::new(ctx),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "从 B站/抖音 URL 或本地文件提取字幕或语音转写文本")]
    async fn analyze_media(
        &self,
        Parameters(p): Parameters<AnalyzeMediaParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.analyze_impl(p).await
    }

    #[tool(description = "analyze_video 别名")]
    async fn analyze_video(
        &self,
        Parameters(p): Parameters<AnalyzeMediaParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.analyze_impl(p).await
    }

    #[tool(description = "列出视觉候选时间点")]
    async fn list_visual_hints(
        &self,
        Parameters(p): Parameters<ListHintsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let cache = avr_core::config::effective_cache_dir(&self.ctx.root, &self.ctx.config)
            .join(&p.media_key);
        let loaded = avr_core::cache::load_cached_transcript(&cache)
            .map_err(|e| rmcp::ErrorData::internal_error(e.message, None))?;
        if loaded.is_none() {
            return Ok(CallToolResult::success(vec![Content::text(
                r#"{"error":true,"message":"未找到缓存"}"#,
            )]));
        }
        let (_, transcript, _) = loaded.unwrap();
        let ffmpeg = avr_core::ffmpeg::FfmpegContext::resolve(&self.ctx.root, &self.ctx.config)
            .map_err(|e| rmcp::ErrorData::internal_error(e.message, None))?;
        let video = cache.join("visual_proxy.mp4");
        let scenes = if video.exists() {
            avr_core::frames::detect_scene_hints(&ffmpeg, &video, &self.ctx.config)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };
        let hints = avr_core::frames::suggest_visual_hints(
            &transcript,
            &scenes,
            self.ctx.config.ui.visual_hint_limit,
        );
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&hints).unwrap_or_default(),
        )]))
    }

    #[tool(description = "按时间戳截帧，默认 delivery=path")]
    async fn extract_frames(
        &self,
        Parameters(p): Parameters<ExtractFramesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let cache = avr_core::config::effective_cache_dir(&self.ctx.root, &self.ctx.config)
            .join(&p.media_key);
        let ffmpeg = avr_core::ffmpeg::FfmpegContext::resolve(&self.ctx.root, &self.ctx.config)
            .map_err(|e| rmcp::ErrorData::internal_error(e.message, None))?;
        let video = cache.join("visual_proxy.mp4");
        if !video.exists() {
            return Ok(CallToolResult::success(vec![Content::text(
                r#"{"error":true,"message":"无视觉源"}"#,
            )]));
        }
        let delivery = match p.delivery.as_str() {
            "inline" => avr_core::frames::FrameDelivery::Inline,
            "both" => avr_core::frames::FrameDelivery::Both,
            _ => avr_core::frames::FrameDelivery::Path,
        };
        let frames = avr_core::frames::extract_frames(
            &ffmpeg,
            &video,
            &p.times,
            &cache.join("frames"),
            p.width.unwrap_or(1280),
            delivery,
        )
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(e.message, None))?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&frames).unwrap_or_default(),
        )]))
    }

    async fn analyze_impl(&self, p: AnalyzeMediaParams) -> Result<CallToolResult, rmcp::ErrorData> {
        let result = if p.source.starts_with("http://") || p.source.starts_with("https://") {
            analyze_url(&self.ctx, &p.source).await
        } else {
            transcribe_local(&self.ctx, PathBuf::from(&p.source).as_path()).await
        };

        match result {
            Ok(r) => {
                let json = if p.response_mode == "full" {
                    full_mcp_payload(&r)
                } else {
                    compact_mcp_payload(&r, p.max_inline_chars)
                };
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string(&json).unwrap_or_default(),
                )]))
            }
            Err(e) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::json!({
                    "error": true,
                    "code": e.code.as_str(),
                    "message": e.message,
                    "hint": e.hint,
                })
                .to_string(),
            )])),
        }
    }
}

#[tool_handler]
impl ServerHandler for AvrMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "scene-packet".into(),
                version: avr_core::VERSION.into(),
            },
            instructions: Some("视频文本提取工具。只返回 transcript，不做总结。".into()),
        }
    }
}

pub fn run_mcp() -> Result<(), AvrError> {
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_ref().and_then(|p| p.parent());
    let root = resolve_data_root(exe_dir)?;
    let ctx = PipelineContext::load(root)?;
    let server = AvrMcpServer::new(ctx);

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let service = server.serve(stdio()).await.map_err(|e| {
                AvrError::new(
                    avr_core::ErrorCode::ProcessFailed,
                    e.to_string(),
                    "MCP 启动失败",
                )
            })?;
            service.waiting().await.map_err(|e| {
                AvrError::new(
                    avr_core::ErrorCode::ProcessFailed,
                    e.to_string(),
                    "MCP 运行异常",
                )
            })?;
            Ok(())
        })
}
