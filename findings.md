# 视觉源 Fallback 实测结论

日期：2026-09-04

## 四级降级顺序

1. **本地 `source.mp4`** — 用户拖入或已下载的完整文件；ffmpeg 精确 seek，最可靠。
2. **`visual_proxy.mp4` 缓存** — 360p/480p 长期代理，适合 agent 连续多时间点截帧。
3. **yt-dlp `--download-sections` 片段** — 取 `[T-3s, T+3s]` 落地后再本地精确 seek。yt-dlp 不重编码时只能切到关键帧（issue #4216），**不能**指望片段边界精确。
4. **直链远程 seek** — 仅 progressive MP4 有效；HLS/DASH、签名 URL、短时效链接易失败。

## 失败判定

| 条件 | 处理 |
|------|------|
| 子进程超时 / 非零退出码 | 降级下一级 |
| 产物不存在或 0 字节 | 降级 |
| `blackdetect` / 平均亮度过低 | 标记黑帧，不返回 |

## Scene 检测

- 讲课类：阈值 0.4 几乎不触发 PPT 切换
- Vlog 类：阈值 0.3 仍可能爆出数百点
- **默认**：`scene_threshold=0.3`，`visual_hint_limit=50`，文本线索优先

## 抖音 Cookie

- `cookies_from_browser`（Edge/Chrome）为硬依赖场景
- 失败提示写入 `ErrorCode::CookiesRequired` hint

## MCP 截帧

- 默认 `delivery=path`：返回 native 绝对路径 + `file://` URI
- `inline`/`both` 显式开启；9 张 1280px JPEG base64 会暴涨上下文
