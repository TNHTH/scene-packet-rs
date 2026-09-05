# AI Video Reader v2 交付与 GitHub 发布复盘

日期：2026-09-05

## 1. 目标与验收

| 目标 | 状态 |
|------|------|
| Rust 单 exe（GUI / CLI / MCP） | 完成 |
| E 盘外置工具链（ffmpeg、yt-dlp、whisper、模型） | 完成 |
| GPU Whisper（RTX 4060 + cuBLAS DLL） | 完成 |
| 抖音 URL 端到端 analyze（615s 无字幕 → ASR） | 完成 |
| 私有 GitHub 仓库发布 | 见下文执行记录 |

验收样例：`https://v.douyin.com/K4wMq-kGog4/` → `video_id=7674972844204100209`，duration=615s，source=asr。

## 2. 架构决策

- **数据根**：`E:\AIVideoReader`（`root.txt` 指针），与 exe 分离
- **工具外置**：不打包进 zip，由 `tools/install-to-e-drive.py` 测速安装
- **抖音反爬**：Netscape cookies + 匹配 `user_agent`；浏览器 CDP 导出 document.cookie，合并保留 HttpOnly `ttwid`
- **ASR**：`whisper-cpp-cublas` preset，模型 `ggml-large-v3-turbo.bin`

## 3. 关键 Bug 链（试跑中修复）

| 现象 | 根因 | 修复 |
|------|------|------|
| `[IO_ERROR] 系统找不到指定的路径` | 写 `.lock` 前未创建缓存子目录 | `jobs.rs`：`create_dir_all` |
| yt-dlp 403 / Fresh cookies | Cookie 与 UA 不匹配 | `config.toml` 增加 `user_agent` |
| 音频下载失败 | yt-dlp 未指定 ffmpeg | `ytdlp.rs`：`--ffmpeg-location` |
| 重复 extract 触发限流 | metadata + subtitles 各调一次 yt-dlp | `extract_metadata_with_info` 复用 JSON |
| ASR panic（nested runtime） | `run_blocking` 在 async 内 | `engine.rs` 改 `run().await` |
| 输出 panic | UTF-8 字节截断 | `truncate_utf8` |

## 4. 抖音试跑结论

- 615 秒视频：元数据 ~3s，抽音 ~20s，GPU ASR ~50s
- 转写文件：`E:\AIVideoReader\cache\url-c9dc1c23fd088981\transcript.txt`
- **待优化**：Whisper SRT 解析后 `segment_count=1`（全文可用，分段需修 `timeline.rs`）

## 5. GitHub 发布步骤

```powershell
cd d:\cursor\file\项目\ai-video-reader-rs
git init -b main
git add .
git status
git commit -m "feat: AI Video Reader v2 初始交付"
gh repo create ai-video-reader-rs --private --source=. --remote=origin --push
```

执行结果见本地日志：`项目/.tmp/codex/2026-09-05/github-publish-log.md`（含 commit hash 与仓库 URL）。

## 6. 后续 TODO

- [ ] CLI：`avr cookies refresh-douyin`（浏览器 Cookie 导出产品化）
- [ ] 修复 Whisper SRT → 多 segment 解析
- [ ] CI：Windows `cargo test` + release 构建

## 7. 安全提醒

- 已删除含硬编码 Cookie 的 `tools/write-douyin-cookies.py`
- `.gitignore` 排除 `cookies*.txt`、`target/`、`.tmp/`
- 仓库为 **private**；仍勿在代码中写入会话令牌
