//! MCP 服务（rmcp）：工具与 resource 实现。desktop 只负责构造依赖并在 stdio 上启动；
//! 这里不依赖 Tauri，可在 Linux 上做进程内集成测试。

use crate::calllog::{self, CallLog};
use crate::index::{
    board_intervals, board_uri, frame_ref, parse_any_frame_uri, read_transcript, top_board_frames,
    FrameKind,
};
use crate::mode::{render, ResponseMode};
use avr_core::error::AvrError;
use avr_core::frames::CapturedFrame;
use avr_core::media::MediaKey;
use avr_core::pipeline::{
    analyze, load_result, resolve_item_cache, AnalyzeOptions, Deps, PipelineContext, Source,
};
use rmcp::transport::stdio;
use rmcp::{
    handler::server::router::tool::ToolRouter,
    handler::server::tool::{Parameters, ToolCallContext},
    model::{
        AnnotateAble, CallToolRequestParam, CallToolResult, Content, Implementation,
        ListResourceTemplatesResult, ListToolsResult, PaginatedRequestParam, ProtocolVersion,
        RawResourceTemplate, ReadResourceRequestParam, ReadResourceResult, ResourceContents,
        ServerCapabilities, ServerInfo,
    },
    schemars,
    service::RequestContext,
    tool, tool_router, RoleServer, ServerHandler, ServiceExt,
};
use serde::Deserialize;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AvrMcpServer {
    ctx: Arc<PipelineContext>,
    deps: Deps,
    call_log: CallLog,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AnalyzeMediaParams {
    source: String,
    /// compact（默认）| index（摘要索引，正文用 read_transcript 按需读取）| full |
    /// auto（转写估算 token ≤ 配置 mcp.auto_full_max_tokens 时 full，否则 index）
    #[serde(default)]
    response_mode: Option<String>,
    #[serde(default = "default_max_inline")]
    max_inline_chars: usize,
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
    /// 单个帧编号（analyze 结果 frames[].n，board=true 时为 board[].n）
    n: Option<usize>,
    /// 一次取多张（与 n 合并），单次最多 keyframes.max_frames_per_call（默认 4）张
    ns: Option<Vec<usize>>,
    /// true = 取板书 / 幻灯片帧（索引 board[]）；默认 false = 分段关键帧
    #[serde(default)]
    board: bool,
    /// 图片长边像素，默认 keyframes.image_long_side（1280）
    width: Option<u32>,
}

/// 请求的帧编号列表：合并 n 与 ns、去重保序，并检查数量上限。
fn requested_frames(
    n: Option<usize>,
    ns: Option<Vec<usize>>,
    max: usize,
) -> Result<Vec<usize>, String> {
    let mut v: Vec<usize> = n.into_iter().chain(ns.unwrap_or_default()).collect();
    let mut seen = std::collections::HashSet::new();
    v.retain(|x| seen.insert(*x));
    if v.is_empty() {
        return Err("需要 n 或 ns".into());
    }
    if v.len() > max {
        return Err(format!("单次最多取 {max} 张，收到 {} 张", v.len()));
    }
    Ok(v)
}

/// 一张截好的帧及其元数据。
struct Captured {
    kind: FrameKind,
    n: usize,
    time: f64,
    resource_uri: String,
    frame: CapturedFrame,
}

impl Captured {
    fn info(&self) -> serde_json::Value {
        serde_json::json!({
            "n": self.n,
            "board": self.kind == FrameKind::Board,
            "time": self.time,
            "resource_uri": self.resource_uri,
            "path": self.frame.path,
            "file_uri": self.frame.file_uri,
        })
    }
}

/// 结果对应的视频画面源（缓存的视频或本地原文件）。
fn visual_source(result: &avr_core::models::AnalyzeResult) -> Option<PathBuf> {
    let cache = PathBuf::from(&result.cache_dir);
    ["source.mp4", "visual_proxy.mp4", "clip.mp4"]
        .iter()
        .map(|f| cache.join(f))
        .chain(std::iter::once(PathBuf::from(&result.video.url)))
        .find(|p| p.is_file())
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
    pub fn new(ctx: PipelineContext, deps: Deps) -> Self {
        let call_log = CallLog::from_config(&ctx.root.logs_dir(), &ctx.config.mcp);
        Self {
            ctx: Arc::new(ctx),
            deps,
            call_log,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "从 B站/抖音 URL 或本地文件提取转写。默认 compact；response_mode=index 只返回摘要索引（分段时间范围、一句话概要、帧编号），正文再用 read_transcript 按需读取；response_mode=auto 时短视频直接全文、长视频返回索引；full 返回全部"
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
                .unwrap_or(crate::index::DEFAULT_PAGE_CHARS)
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
        description = "按编号取图（只读）。board=true 取板书 / 幻灯片帧（索引 board[].n），否则取分段关键帧（frames[].n）；ns 一次最多 4 张。返回帧信息与 JPEG（长边默认 1280）；也可读 resource avr://job/{media_key}/frame/{n} 或 /board/{n}"
    )]
    async fn get_frame(
        &self,
        Parameters(p): Parameters<GetFrameParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kf = &self.ctx.config.keyframes;
        let ns = match requested_frames(p.n, p.ns, kf.max_frames_per_call) {
            Ok(v) => v,
            Err(msg) => {
                return Ok(error_json(
                    "INVALID_ARGUMENT",
                    msg,
                    "用 ns 传 1–4 个帧编号。",
                ))
            }
        };
        let kind = if p.board {
            FrameKind::Board
        } else {
            FrameKind::Section
        };
        let mut texts = Vec::new();
        let mut images = Vec::new();
        for n in ns {
            match self.capture_frame(&p.media_key, kind, n, p.width).await {
                Ok(c) => {
                    texts.push(c.info());
                    if let Some(b64) = c.frame.base64 {
                        images.push(Content::image(b64, "image/jpeg"));
                    }
                }
                Err(msg) => {
                    return Ok(error_json(
                        "FRAME_UNAVAILABLE",
                        msg,
                        "确认 media_key 与帧编号（board=true 时用索引 board[].n）。",
                    ))
                }
            }
        }
        // 单张时保持旧格式（顶层字段），多张时放进 frames 数组。
        let info = if texts.len() == 1 {
            texts.remove(0)
        } else {
            serde_json::json!({ "frames": texts })
        };
        let mut content = vec![Content::text(info.to_string())];
        content.extend(images);
        Ok(CallToolResult::success(content))
    }

    /// 读取（必要时计算并缓存）板书帧。
    async fn board_frames(
        &self,
        result: &avr_core::models::AnalyzeResult,
    ) -> Result<Vec<avr_core::keyframes::Keyframe>, String> {
        let video = visual_source(result).ok_or_else(|| "没有可用的视频画面源".to_string())?;
        let ffmpeg = avr_core::ffmpeg::FfmpegContext::resolve(&self.ctx.root, &self.ctx.config)
            .map_err(|e| e.format_user())?;
        avr_core::keyframes::load_or_compute(
            &ffmpeg,
            &video,
            &PathBuf::from(&result.cache_dir),
            &self.ctx.config.keyframes,
            result.transcript.as_ref(),
        )
        .await
        .map_err(|e| e.format_user())
    }

    /// 截取第 n 帧（分段关键帧或板书帧），按配置的长边与 JPEG 质量输出。
    async fn capture_frame(
        &self,
        media_key: &str,
        kind: FrameKind,
        n: usize,
        long_side: Option<u32>,
    ) -> Result<Captured, String> {
        let key = MediaKey::parse(media_key).map_err(|e| e.format_user())?;
        let result = load_result(&self.ctx, &key)
            .map_err(|e| e.format_user())?
            .ok_or_else(|| "未找到缓存，先调用 analyze_media".to_string())?;
        self.capture_from(&result, kind, n, long_side).await
    }

    async fn capture_from(
        &self,
        result: &avr_core::models::AnalyzeResult,
        kind: FrameKind,
        n: usize,
        long_side: Option<u32>,
    ) -> Result<Captured, String> {
        let (time, resource_uri) = match kind {
            FrameKind::Section => {
                let f = frame_ref(result, n).map_err(|e| e.to_string())?;
                (f.time, f.resource_uri)
            }
            FrameKind::Board => {
                let frames = self.board_frames(result).await?;
                let f = frames
                    .get(n)
                    .ok_or_else(|| format!("板书帧编号 {n} 超出范围（共 {} 张）", frames.len()))?;
                (f.time, board_uri(&result.media_key, n))
            }
        };
        let cache = PathBuf::from(&result.cache_dir);
        let source = visual_source(result).ok_or_else(|| "没有可用的视频画面源".to_string())?;
        let ffmpeg = avr_core::ffmpeg::FfmpegContext::resolve(&self.ctx.root, &self.ctx.config)
            .map_err(|e| e.format_user())?;
        let kf = &self.ctx.config.keyframes;
        let long = long_side.unwrap_or(kf.image_long_side).max(16);
        let name = match kind {
            FrameKind::Section => format!("f{n}_{long}_{}.jpg", kf.jpeg_quality),
            FrameKind::Board => format!("b{n}_{long}_{}.jpg", kf.jpeg_quality),
        };
        let path = cache.join("frames").join(name);
        avr_core::ffmpeg::capture_frame_spec(&ffmpeg, &source, time, &path, long, kf.jpeg_quality)
            .await
            .map_err(|e| e.format_user())?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        use base64::Engine as _;
        let frame = CapturedFrame {
            time,
            path: path.display().to_string(),
            file_uri: avr_core::paths::to_file_uri(&path),
            markdown: String::new(),
            width: long,
            height: 0,
            base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        };
        Ok(Captured {
            kind,
            n,
            time,
            resource_uri,
            frame,
        })
    }

    /// 索引模式加“板书区间”段落；转写稀少时附最重要的至多 `sparse_attach` 张板书图。
    /// 选帧失败（如无画面源）只记 `board_error`，不影响转写结果。
    async fn attach_board(
        &self,
        r: &avr_core::models::AnalyzeResult,
        mode: ResponseMode,
        json: &mut serde_json::Value,
    ) -> Vec<Content> {
        let kf = &self.ctx.config.keyframes;
        let duration = r.video.duration.max(
            r.transcript
                .as_ref()
                .and_then(|t| t.segments.last())
                .map(|s| s.end)
                .unwrap_or(0.0),
        );
        let sparse = avr_core::keyframes::is_sparse(r.transcript.as_ref(), duration, kf);
        if mode != ResponseMode::Index && !sparse {
            return Vec::new();
        }
        let Some(obj) = json.as_object_mut() else {
            return Vec::new();
        };
        let frames = match self.board_frames(r).await {
            Ok(f) => f,
            Err(e) => {
                obj.insert("board_error".into(), e.into());
                return Vec::new();
            }
        };
        if mode == ResponseMode::Index {
            obj.insert(
                "board".into(),
                serde_json::to_value(board_intervals(r, &frames)).unwrap_or_default(),
            );
        }
        let mut images = Vec::new();
        if sparse {
            let ns = top_board_frames(&frames, kf.sparse_attach.min(kf.max_frames_per_call));
            for &n in &ns {
                if let Ok(c) = self.capture_from(r, FrameKind::Board, n, None).await {
                    if let Some(b64) = c.frame.base64 {
                        images.push(Content::image(b64, "image/jpeg"));
                    }
                }
            }
            obj.insert("sparse_transcript".into(), true.into());
            obj.insert("board_images".into(), serde_json::json!(ns));
        }
        images
    }

    async fn analyze_impl(&self, p: AnalyzeMediaParams) -> Result<CallToolResult, rmcp::ErrorData> {
        let result = analyze(
            &self.ctx,
            &Source::parse(&p.source),
            &AnalyzeOptions::default(),
            &self.deps,
        )
        .await;

        match result {
            Ok(r) => {
                let (mut json, resolved) = render(
                    &r,
                    ResponseMode::parse(p.response_mode.as_deref()),
                    p.max_inline_chars,
                    self.ctx.config.mcp.auto_full_max_tokens,
                );
                if let Some(obj) = json.as_object_mut() {
                    obj.insert("response_mode".into(), resolved.as_str().into());
                }
                let images = self.attach_board(&r, resolved, &mut json).await;
                let mut content = vec![Content::text(
                    serde_json::to_string(&json).unwrap_or_default(),
                )];
                content.extend(images);
                Ok(CallToolResult::success(content))
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

impl ServerHandler for AvrMcpServer {
    // 不用 #[tool_handler]：手写 call_tool 以便每次调用记一行日志（stderr + logs/mcp-calls.jsonl）。
    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let tool = request.name.to_string();
        let args = request.arguments.clone();
        let tcc = ToolCallContext::new(self, request, context);
        let result = self.tool_router.call(tcc).await;
        let (texts, image_bytes): (Vec<&str>, usize) = match &result {
            Ok(r) => (
                r.content
                    .iter()
                    .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
                    .collect(),
                r.content
                    .iter()
                    .filter_map(|c| c.as_image().map(|i| i.data.len()))
                    .sum(),
            ),
            Err(_) => (Vec::new(), 0),
        };
        self.call_log.write(&calllog::record(
            &tool,
            args.as_ref(),
            &texts,
            image_bytes,
            result.is_ok(),
        ));
        result
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tool_router.list_all()))
    }

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
                "视频文本提取工具。analyze_media 默认返回 compact；指定 response_mode=index 时只返回摘要索引（含 board[] 板书区间），再用 read_transcript 按时间范围取原文，用 get_frame（board=true 取板书帧，ns 一次最多 4 张）或 resource 取图。转写稀少的视频会直接附上最重要的板书图。不做总结。".into(),
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
            resource_templates: vec![
                RawResourceTemplate {
                    uri_template: "avr://job/{media_key}/frame/{n}".into(),
                    name: "关键帧".into(),
                    description: Some("analyze_media 结果 frames[] 中的关键帧（JPEG）".into()),
                    mime_type: Some("image/jpeg".into()),
                }
                .no_annotation(),
                RawResourceTemplate {
                    uri_template: "avr://job/{media_key}/board/{n}".into(),
                    name: "板书帧".into(),
                    description: Some("索引 board[] 中的板书 / 幻灯片帧（JPEG）".into()),
                    mime_type: Some("image/jpeg".into()),
                }
                .no_annotation(),
            ],
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::ErrorData> {
        let (key, kind, n) = parse_any_frame_uri(&request.uri)
            .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
        let c = self
            .capture_frame(key.as_str(), kind, n, None)
            .await
            .map_err(|e| rmcp::ErrorData::resource_not_found(e, None))?;
        let blob = c
            .frame
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

/// 在 stdio 上运行 MCP 服务直到客户端断开。
pub fn run_stdio(ctx: PipelineContext, deps: Deps) -> Result<(), AvrError> {
    let server = AvrMcpServer::new(ctx, deps);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AvrError::io(e, "MCP 启动失败"))?
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

#[cfg(test)]
mod tests {
    use super::requested_frames;

    #[test]
    fn frame_request_limits() {
        assert_eq!(requested_frames(Some(1), None, 4).unwrap(), vec![1]);
        assert_eq!(
            requested_frames(Some(2), Some(vec![0, 2, 3]), 4).unwrap(),
            vec![2, 0, 3]
        );
        assert!(requested_frames(None, Some(vec![0, 1, 2, 3, 4]), 4).is_err());
        assert!(requested_frames(None, None, 4).is_err());
        assert!(requested_frames(None, Some(vec![]), 4).is_err());
    }
}
