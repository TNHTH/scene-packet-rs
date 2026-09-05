# AI Video Reader v2

单 `avr.exe`：无参数开 GUI、`--mcp` 启 MCP stdio、子命令走 CLI。

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
