# 本地视频工作流

无界面入口是 `scene-packet`。桌面程序 `avr` 仍保留 Tauri CLI 和 MCP；协议实现在 `avr-mcp`，`scene-packet --mcp` 只负责启动它。

```text
scene-packet transcribe <本地文件>
scene-packet analyze <B站或抖音 URL>
scene-packet doctor
scene-packet mcp-config
scene-packet --mcp
scene-packet frames <文件> --output <目录> --times 1,2
scene-packet packet <文件> --output <目录>
```

`packet` 写出 `manifest.json`、`README.md` 和内嵌图片的 `index.html`。截图高度来自探测结果；当前 `extract_frames` 不回填高度。语音转写走现有 `pipeline::analyze`，不另写一套 ASR。

`avr-mcp`、`fingerprint.rs`、`keyframes.rs`、`bench/effect/` 和 `docs/adr/` 留在这条 main 上。`media_key` 仍是文件头 4MB 的缓存键；整文件变化用 `content_fingerprint`。

Windows 文件 URI：空格和非 ASCII 路径经 `url::Url::from_file_path` 编码后再解析。非 Windows 的默认数据根可以落到 `XDG_DATA_HOME/ScenePacket` 或 `~/.local/share/ScenePacket`。

导出仍包含 `avr-core` 的 Obsidian bundle。缓存、ASR、选帧和 MCP 按 `docs/adr/` 保持。
