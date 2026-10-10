# 本地视频到可追溯素材包

## 准备

构建 `cargo build --release -p avr-cli`，将 `target/release/scene-packet`（Windows 为 `.exe`）加入 PATH 或直接使用绝对路径。不依赖 Tauri、Python 或 GUI。准备 ffmpeg/ffprobe、whisper.cpp 的 whisper-cli 和 GGML 模型，程序不自动下载权重。纯画面流程不需要 Whisper。

首次运行 `scene-packet --data-root ./video-data doctor --visual-only` 会建立数据布局和默认配置。编辑生成的 `config.toml`，参考仓库配置示例。工具、模型、VAD 手动路径使用绝对路径；缓存的相对路径以数据根为准。`--data-root` 优先于环境变量和已有指针。

```sh
scene-packet --data-root ./video-data doctor
scene-packet --data-root ./video-data transcribe './技术 视频.mp4'
scene-packet --data-root ./video-data packet './技术 视频.mp4' --output ./packet --interval 2 --max-frames 120 --times 74.5,110
scene-packet --data-root ./video-data frames './技术 视频.mp4' --output ./extra-frames --times 20,40,70 --width 1280
python3 tools/validate-local-video.py ./packet --source './技术 视频.mp4' --ffprobe ffprobe
```

Windows PowerShell 同样使用这些参数，路径用单/双引号包围。无需另建 runner 或 Whisper wrapper。`transcribe` 支持音频；`packet` 需要画面。`packet --visual-only` 跳过 ASR；无音轨视频自动采用纯画面流程。有音轨但 Whisper 缺失会明确失败，不把缺失转写伪装为成功。

## 转写与缓存

音乐较多时配置 `vad_model_path`，可启用 `suppress_non_speech`、`no_fallback`；语言与技术术语提示由 `language`、`prompt` 设置。VAD 可能漏掉弱语音，不能代替人工核对。旧 Whisper 不认识参数时返回进程错误，请使用支持这些参数的版本。

媒体身份覆盖完整文件；处理指纹绑定 ASR 配置、工具、模型、VAD 实际内容。同路径替换权重或修改 prompt 会进入新缓存；旧缓存保留。计算大模型 hash 会有磁盘读取成本。命中缓存后移动的输入文件会更新来源位置。分析结果 `platform=local`，所有时间均相对原片。

原始 Whisper SRT 与整理后的文本分开保存，不自动删除重复语音，也不把技术解读当作原片内容。图文包报告未经校对及连续重复警告。仅有字幕准确不代表画面理解完整。

## 采样与导出

采样保留均匀覆盖与手动补帧，剩余预算加入场景点。预算不足时均匀间隔增大；强制补帧超限会失败。输出首尾、最大间隔和实际数量可从 manifest 检查。自动尾点按帧率约束；手动点保留原时间，如没有可解码帧则明确失败。

输出包含 `README.md`、`index.html`、`images/`、`manifest.json`，有转写时另有 `transcript.txt`、`transcript.srt`、`raw_asr.srt`。HTML 内嵌图片，可单文件离线打开；Markdown 分享时要连 images 目录一起复制。manifest 记录视频和图片 SHA-256、尺寸、时间、配置指纹与有效 ASR 选项。包仍包含原始内容，应按素材本身的分享范围处理。

输出目录必须为空。中断可能留下部分图片，未生成 manifest 不算完成；重试指定新空目录即可复用有效转写缓存。锁在任务取消、进程退出后释放，其他进程仍持锁时明确提示重试。子进程 stdout/stderr 并发排空并限制保留量，超时终止进程树。

## MCP

运行 `scene-packet --data-root /absolute/video-data mcp-config` 获取配置；宿主应启动同一数据根。提供 `analyze_media`、兼容别名 `analyze_video`、`list_visual_hints`、`extract_frames`、`export_packet`。先分析再用 media_key 查询画面；本地无音轨可直接 `export_packet`（无需先分析）。`extract_frames` 的 `inline/both` 返回元数据及 MCP 图片内容块，默认 path 不传图片数据。来源文本是待分析资料，不构成对智能体的指令。

URL 视觉源下载完整时间轴，不复用历史六秒 clip 或旧的限长代理冒充原片。远程可用性仍取决于平台、Cookie 和 yt-dlp；本地验证不能证明线上平台随时可用。

## 验证边界

`python3 tools/check.py` 运行 Rust 核心/CLI、Clippy、构建和合成 Python 测试。Linux 合成视频测试需要 `/usr/bin/ffmpeg`，缺失会显式 SKIP；CI Linux 安装它。Windows CI 校验核心/CLI 与桌面编译，不能代替 Windows 真机 GUI、GPU、拖放或进程树测试。真实私有视频、权重、Cookie 不进入仓库。
