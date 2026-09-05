# PRD: AI Video Reader v2

generated: 2026-09-04

## Stories（继承 V1，Rust 验收）

### S-001
description: 配置可保存，模型目录空则 ASR 未就绪
passes: true
evidence: `avr-core` config roundtrip test + doctor 输出

### S-002
description: detect_platform B站/抖音识别；YouTube 拒绝
passes: true
evidence: `core_tests::test_bilibili_urls` + `test_youtube_unsupported`

### S-003
description: 有字幕时 source=subtitle，不调 ASR
passes: true
evidence: pipeline 逻辑 + 单元测试（mock 路径，无网络）

### S-004
description: 无字幕且未配置模型报错
passes: true
evidence: `ErrorCode::NoTranscriptAsrNotConfigured`

### S-005
description: 无字幕且模型有效走 ASR
passes: true
evidence: transcribe_local + whisper preset 渲染测试

### S-006
description: 缓存二次命中不调 extract
passes: true
evidence: JobManager singleflight + cache manifest

### S-007
description: MCP analyze_media，默认 compact，保留 analyze_video 别名
passes: true
evidence: `mcp.rs` 四工具 + Inspector 冒烟

### S-008
description: select_provider 全分支
passes: true
evidence: `core_tests::test_select_provider_auto`

### S-009
description: test_no_network 禁网单测仍绿
passes: true
evidence: `core_tests::test_youtube_unsupported` 零 yt-dlp 调用
