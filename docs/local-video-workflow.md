# 本地视频工作流

优化线的无界面入口是同一个 `avr` 可执行文件，不另拆 CLI。GUI、CLI 和 MCP 仍在这个入口里。

```text
avr transcribe <本地文件>
avr analyze <B站或抖音 URL>
avr doctor
avr mcp-config
avr --mcp
```

`test` 分支上的独立 `avr-cli` / `scene-packet packet` 没有并入。那条线会删掉本分支的 `avr-mcp`、fingerprint、keyframe、benchmark 和 ADR，而且它调用的 `transcribe_local` 在当前核心里不是公开接口。对应文件仍留在 `test` 和 `codex/scenepacket-video-reading`。

已并入的是 Windows 文件 URI 往返：空格和非 ASCII 路径经 `url::Url::from_file_path` 编码后再解析。非 Windows 的默认数据根可以落到 `XDG_DATA_HOME/ScenePacket` 或 `~/.local/share/ScenePacket`。

导出仍是 `avr-core` 的 Obsidian bundle，不是 `test` 分支的图文 packet。缓存、ASR、选帧和 MCP 按 `docs/adr/` 保持。
