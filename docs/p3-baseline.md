# P3 输出质量基线

测试：`crates/avr-mcp/tests/baseline.rs`（P3-1）。

## 测试素材

- 视频：测试运行时用 ffmpeg lavfi 现生成，`testsrc2` 2s + 纯红 2s + 纯蓝 2s（两处硬切，场景检测稳定），配 440Hz `sine` 音轨，320x240@25fps，共 6s。不提交二进制文件。
- 转写：`FakeTools`（`avr-core` 的 `test-support` feature）返回固定的 60 段中文转写，含“如图”“幻灯片”等视觉线索。
- 时间轴与关键帧：走真实 ffmpeg（`detect_scenes` + `suggest_visual_hints` + `extract_frames`）。
- 找不到 ffmpeg 时跳过；`AVR_REQUIRE_FFMPEG=1` 时找不到就失败。CI 的 Linux job 用 apt 安装 ffmpeg 并设置该变量，日志中出现 `ffmpeg baseline: RAN` 表示真实执行。

## 快照（insta，`tests/snapshots/`）

| 快照 | 内容 | 忽略（redaction） |
|---|---|---|
| `baseline__transcript` | 完整 `AnalyzeResult`（转写分段、平台、截断标记） | 路径、file URI、cache_dir、media_key |
| `baseline__timeline` | SRT 条目数、前两条 SRT、文本线索数 | — |
| `baseline__keyframes` | 只记结构：帧数在 1..=8、检测到 ≥2 处切换、每帧时间落在某个分段内、文件存在、字段列表 | 不记具体帧时间与图片哈希 |

## Payload 体积基线（P3-1，改动前）

token 数用 tiktoken `o200k_base` 统计，仅作参考：
`cargo test -p avr-mcp --test baseline -- --ignored --nocapture token_stats`。

| payload | 字节 | tokens (o200k_base) |
|---|---|---|
| compact（`max_inline_chars=3000`，analyze_media 默认返回） | 4134 | 1192 |
| full（`response_mode=full`） | 11480 | 3478 |

字节数随临时目录路径长度有几个字节的浮动。
