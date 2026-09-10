# ScenePacket

把视频转换成可供智能体读取的上下文包。

ScenePacket 从本地文件、B 站和抖音视频中提取带时间轴的字幕、语音转写、
视觉线索、元数据和关键帧，并通过 GUI、CLI 和 MCP 提供给 Cursor、Codex 等工具使用。

产品副标题：**Turn video into agent-ready context.**

当前可执行文件仍叫 `avr.exe`，以兼容已有脚本和 MCP 配置；后续可在独立版本中提供
`scenepacket.exe` 别名。

建议 GitHub 仓库 slug 使用 `scene-packet-rs`。当前远端地址仍为
`TNHTH/ai-video-reader-rs`，远端仓库重命名与 remote URL 同步需单独执行。

## 体积说明

| 类别 | 典型大小 | 说明 |
|------|----------|------|
| **应用本体** | ≤25 MB | `avr.exe` + 便携包 `avr.portable` |
| **数据目录** | 100 MB ~ 2 GB+ | ffmpeg (~106 MB)、yt-dlp、whisper 模型 (1.5 GB+) |

第三方二进制**不随便携 zip 分发**；首次使用在「工具」页引导安装或手动指定路径。

## 启动方式

| 命令 | 作用 |
|------|------|
| `avr.exe` | Tauri GUI |
| `avr.exe --mcp` | stdio MCP（Cursor/Codex） |
| `avr.exe transcribe <path>` | 转写本地文件 |
| `avr.exe analyze <url>` | 分析 B站/抖音链接 |
| `avr.exe mcp-config` | 打印可粘贴的 MCP JSON |
| `avr.exe doctor` | 数据根与工具状态 |

## 构建

```powershell
cd 项目\ai-video-reader-rs
cargo build --release
cd desktop\ui && npm install && npm run build
```

发布包：

```powershell
.\tools\package-avr.ps1
```

产物：`dist/avr-portable.zip`（仅 `avr.exe` + `avr.portable`）

## 数据根

优先级：`AVR_DATA_ROOT` > exe 同目录 `avr.portable` > `%APPDATA%\AIVideoReader\root.txt`

其中 `AIVideoReader` 是旧版本数据目录名，ScenePacket 会继续读取它，避免升级时丢失已有
配置、模型和缓存；新安装仍可按本节路径规划使用。

```
<DataRoot>/
├─ .avr-root
├─ config.toml
├─ bin/ffmpeg/   bin/yt-dlp/   bin/whisper/
├─ models/
├─ cache/<media_key>/
├─ exports/
└─ logs/
```

## MCP 工具

- `analyze_media` / `analyze_video`（别名）— 默认 `response_mode=compact`
- `list_visual_hints` — 视觉候选时间点（Phase B）
- `extract_frames` — 截帧，默认 `delivery=path`

接入 Cursor：运行 `avr mcp-config`，将 JSON 合并到 `.cursor/mcp.json`。

## 竞品与参考项目

下面的项目覆盖视频摘要、转写、关键帧、视觉参考和 Agent/MCP 接入等方向，可作为
ScenePacket 后续产品设计和交互优化的参考。它们并非全部是直接竞品，定位以公开页面为准。

| 项目 | 链接 | 公开定位 / 可借鉴方向 |
|------|------|----------------------|
| Video Ref / ai-video-transcript-keyframes | [GitHub](https://github.com/tanzeeldevAi/ai-video-transcript-keyframes) | 将视频转换为带时间戳 transcript 与带标签关键帧联系表，面向 Claude、ChatGPT、Cursor。 |
| VideoContext App | [videocontext.app](https://videocontext.app) | 将屏幕录制转换为 AI-ready context；参考“录制→上下文”的产品表达。 |
| Eightify | [eightify.app](https://eightify.app) | AI 视频摘要和关键观点提取。 |
| Glasp YouTube Summary | [glasp.co/youtube-summary](https://glasp.co/youtube-summary) | YouTube 摘要、时间点和知识管理。 |
| Recall | [recall.it](https://www.recall.it) | 视频总结与个人知识库。 |
| FrameLore | [framelore.com](https://framelore.com) | AI 影视前期制作 / production intelligence 平台，强调视觉资产与决策上下文。 |
| SceneWeaver | [sceneweaver.ai](https://www.sceneweaver.ai) | 竖屏剧创作平台；“场景编织”可参考其创作流程，但需注意同名项目较多。 |
| VoxFrame | [GitHub](https://github.com/tayyabalitech/voxframe) | 多模态视频描述与叙事合成。 |
| VidLedger | [GitHub](https://github.com/DNAMcKnight/vidledger) | 视频内容追踪和组织；可参考内容索引与溯源。 |
| SceneLedger | [informedsoftware.co.nz/product/sceneledger](https://www.informedsoftware.co.nz/product/sceneledger) | 场景检查 / 记录 App；可参考“场景记录”信息结构。 |
| ReelScribe | [reelscribe.app](https://reelscribe.app) | Instagram Reels 转写服务。 |
| MediaScribe | [mediascribe.app](https://mediascribe.app) | YouTube transcript 提取；另有面向政府视频无障碍字幕的平台。 |

`FrameLoom`（已有多个 App、视频重构和影视项目）与 `VideoContext`（已有 [BBC VideoContext](https://github.com/bbc/VideoContext)、App 和 MCP 项目）均存在明显重名，不建议作为本项目名称。

ScenePacket 的差异点：本地优先，统一编排平台字幕、Whisper ASR、视觉线索和按需截帧，
先生成可复用的上下文包，再通过 MCP 交给智能体完成总结、问答或进一步分析。

## Windows / E 盘部署

工具与模型建议放在 E 盘固定目录，数据根指向 `E:\AIVideoReader`（通过首次向导或 `%APPDATA%\AIVideoReader\root.txt`）。

```powershell
# 1. 测速下载 ffmpeg / yt-dlp / whisper / 模型到 E 盘
python tools/install-to-e-drive.py

# 2. 启用 GPU Whisper（RTX + cuBLAS DLL，无需完整 CUDA Toolkit）
python tools/enable-whisper-gpu.py

# 3. 复制配置模板到数据根并编辑路径
copy config.toml.example E:\AIVideoReader\config.toml
```

`config.toml` 中路径请使用正斜杠（`E:/ffmpeg/...`），避免 TOML 将 `\f` 解析为转义。

## 抖音链接

抖音需 Netscape 格式 Cookie 文件，且 `user_agent` **必须与导出 Cookie 的浏览器 UA 一致**（Chrome/Edge/Cursor 各不相同）。

```powershell
# 浏览器打开视频页后，用 document.cookie 字符串刷新（保留 HttpOnly 的 ttwid）
python tools/refresh-douyin-cookies.py "<document.cookie 字符串>"

# 或从 JSON 文件读取
python tools/refresh-douyin-from-json.py cookie-refresh.json
```

Cookie 写入 `E:\AIVideoReader\cookies\douyin.txt`（已在 `.gitignore` 排除，**勿提交 Git**）。

```powershell
avr.exe doctor
avr.exe analyze "https://v.douyin.com/xxxx/"
```

## 开发

```powershell
cargo test -p avr-core
cargo clippy -- -D warnings
```

## Legacy

Python V1 保留于 `项目/ai-video-reader`，仅供回滚参考，不再作为主交付。
