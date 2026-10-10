//! MCP stdio 服务。

use crate::state::AppState;
use avr_core::error::AvrError;
use avr_core::frames::{CapturedFrame, FrameDelivery};
use avr_core::media::MediaKey;
use avr_core::pipeline::{
    analyze, load_result, resolve_item_cache, AnalyzeOptions, PipelineContext, Source,
};
use avr_mcp::index::{frame_ref, parse_frame_uri, read_transcript, summary_index, FrameRef};
pub use avr_mcp::payload::{compact_mcp_payload, full_mcp_payload};
use rmcp::transport::stdio;
use rmcp::{
    handler::server::router::tool::ToolRouter,
    handler::server::tool::Parameters,
    model::{
        AnnotateAble, CallToolResult, Content, Implementation, ListResourceTemplatesResult,
        PaginatedRequestParam, ProtocolVersion, RawResourceTemplate, ReadResourceRequestParam,
        ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo,
    },
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router, RoleServer, ServerHandler, ServiceExt,
};
use serde::Deserialize;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
struct AvrMcpServer {
    ctx: Arc<PipelineContext>,
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AnalyzeMediaParams {
    source: String,
    /// compact（默认）| index（摘要索引，正文用 read_transcript 按需读取）| full
    #[serde(default = "default_compact")]
    response_mode: String,
    #[serde(default = "default_max_inline")]
    max_inline_chars: usize,
}

fn default_compact() -> String {
    "compact".into()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReadTranscriptParams {
    media_key: String,
    /// 起始秒（含），默认 0
    start: Option<f64>,
    /// 结束秒（不含），默认到结尾
    end: Option<f64>,
    /// 上一页返回的 nextCursor
    cursor: Option<String>,
    /// 单页最多字符数，默认 4000
    max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetFrameParams {
    media_key: String,
    /// 帧编号（analyze 结果 frames[].n）
    n: usize,
    width: Option<u32>,
}

fn error_json(code: &str, message: impl std::fmt::Display, hint: &str) -> CallToolResult {
    CallToolResult::success(vec![Content::text(
        serde_json::json!({ "error": true, "code": code, "message": message.to_string(), "hint": hint })
            .to_string(),
    )])
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
    fn new(state: AppState) -> Self {
        Self {
            ctx: Arc::new(state.pipeline_ctx()),
            state: Arc::new(state),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "从 B站/抖音 URL 或本地文件提取转写。默认 compact；response_mode=index 只返回摘要索引（分段时间范围、一句话概要、帧编号），正文再用 read_transcript 按需读取"
    )]
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
        let (_, cache) = match resolve_item_cache(&self.ctx, &p.media_key) {
            Ok(v) => v,
            Err(e) => return Ok(error_json(e.code.as_str(), e.message, &e.hint)),
        };
        let loaded = avr_core::cache::load_cached_transcript(&cache, None)
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
        let (_, cache) = match resolve_item_cache(&self.ctx, &p.media_key) {
            Ok(v) => v,
            Err(e) => return Ok(error_json(e.code.as_str(), e.message, &e.hint)),
        };
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

    #[tool(
        description = "按时间范围读取转写原文（只读）。返回 segments 与 nextCursor；有 nextCursor 时带上它再调用取下一页"
    )]
    async fn read_transcript(
        &self,
        Parameters(p): Parameters<ReadTranscriptParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let key = match MediaKey::parse(&p.media_key) {
            Ok(k) => k,
            Err(e) => return Ok(error_json(e.code.as_str(), e.message, &e.hint)),
        };
        let result = match load_result(&self.ctx, &key) {
            Ok(Some(r)) => r,
            Ok(None) => {
                return Ok(error_json(
                    "CACHE_ERROR",
                    "未找到缓存",
                    "先调用 analyze_media。",
                ))
            }
            Err(e) => return Ok(error_json(e.code.as_str(), e.message, &e.hint)),
        };
        match read_transcript(
            &result,
            p.start.unwrap_or(0.0),
            p.end.unwrap_or(f64::MAX),
            p.cursor.as_deref(),
            p.max_chars
                .unwrap_or(avr_mcp::index::DEFAULT_PAGE_CHARS)
                .max(1),
        ) {
            Ok(page) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string(&page).unwrap_or_default(),
            )])),
            Err(e) => Ok(error_json(
                "INVALID_ARGUMENT",
                e,
                "检查 cursor / 时间范围。",
            )),
        }
    }

    #[tool(
        description = "按编号取关键帧（只读）。返回帧信息与 JPEG 图片；也可直接读取 resource avr://job/{media_key}/frame/{n}"
    )]
    async fn get_frame(
        &self,
        Parameters(p): Parameters<GetFrameParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match self.capture_frame(&p.media_key, p.n, p.width).await {
            Ok((fref, frame)) => {
                let info = serde_json::json!({
                    "n": fref.n,
                    "time": fref.time,
                    "resource_uri": fref.resource_uri,
                    "path": frame.path,
                    "file_uri": frame.file_uri,
                });
                let mut content = vec![Content::text(info.to_string())];
                if let Some(b64) = frame.base64 {
                    content.push(Content::image(b64, "image/jpeg"));
                }
                Ok(CallToolResult::success(content))
            }
            Err(msg) => Ok(error_json(
                "FRAME_UNAVAILABLE",
                msg,
                "确认 media_key 与帧编号。",
            )),
        }
    }

    /// 截取第 n 帧（缓存的视频源或本地原文件）。
    async fn capture_frame(
        &self,
        media_key: &str,
        n: usize,
        width: Option<u32>,
    ) -> Result<(FrameRef, CapturedFrame), String> {
        let key = MediaKey::parse(media_key).map_err(|e| e.format_user())?;
        let result = load_result(&self.ctx, &key)
            .map_err(|e| e.format_user())?
            .ok_or_else(|| "未找到缓存，先调用 analyze_media".to_string())?;
        let fref = frame_ref(&result, n).map_err(|e| e.to_string())?;
        let cache = PathBuf::from(&result.cache_dir);
        let source = ["source.mp4", "visual_proxy.mp4", "clip.mp4"]
            .iter()
            .map(|f| cache.join(f))
            .chain(std::iter::once(PathBuf::from(&result.video.url)))
            .find(|p| p.is_file())
            .ok_or_else(|| "没有可用的视频画面源".to_string())?;
        let ffmpeg = avr_core::ffmpeg::FfmpegContext::resolve(&self.ctx.root, &self.ctx.config)
            .map_err(|e| e.format_user())?;
        let frames = avr_core::frames::extract_frames(
            &ffmpeg,
            &source,
            &[fref.time],
            &cache.join("frames"),
            width.unwrap_or(1280),
            FrameDelivery::Both,
        )
        .await
        .map_err(|e| e.format_user())?;
        let frame = frames
            .into_iter()
            .next()
            .ok_or_else(|| "截帧失败".to_string())?;
        Ok((fref, frame))
    }

    async fn analyze_impl(&self, p: AnalyzeMediaParams) -> Result<CallToolResult, rmcp::ErrorData> {
        let result = analyze(
            &self.ctx,
            &Source::parse(&p.source),
            &AnalyzeOptions::default(),
            self.state.deps(),
        )
        .await;

        match result {
            Ok(r) => {
                let json = match p.response_mode.as_str() {
                    "full" => full_mcp_payload(&r),
                    "index" => summary_index(&r),
                    _ => compact_mcp_payload(&r, p.max_inline_chars),
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
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation {
                name: "scene-packet".into(),
                version: avr_core::VERSION.into(),
            },
            instructions: Some(
                "视频文本提取工具。analyze_media 默认返回 compact；指定 response_mode=index 时只返回摘要索引，再用 read_transcript 按时间范围取原文，用 get_frame 或 resource 取关键帧。不做总结。".into(),
            ),
        }
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, rmcp::ErrorData> {
        Ok(ListResourceTemplatesResult {
            next_cursor: None,
            resource_templates: vec![RawResourceTemplate {
                uri_template: "avr://job/{media_key}/frame/{n}".into(),
                name: "关键帧".into(),
                description: Some("analyze_media 结果 frames[] 中的关键帧（JPEG）".into()),
                mime_type: Some("image/jpeg".into()),
            }
            .no_annotation()],
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::ErrorData> {
        let (key, n) = parse_frame_uri(&request.uri)
            .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
        let (_, frame) = self
            .capture_frame(key.as_str(), n, None)
            .await
            .map_err(|e| rmcp::ErrorData::resource_not_found(e, None))?;
        let blob = frame
            .base64
            .ok_or_else(|| rmcp::ErrorData::internal_error("截帧无数据", None))?;
        Ok(ReadResourceResult {
            contents: vec![ResourceContents::BlobResourceContents {
                uri: request.uri,
                mime_type: Some("image/jpeg".into()),
                blob,
            }],
        })
    }
}

pub fn run_mcp() -> Result<(), AvrError> {
    let server = AvrMcpServer::new(AppState::load()?);

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
