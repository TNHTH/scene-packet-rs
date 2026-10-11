# Changelog

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。优化工作在 `optimize` 分支进行，清单见 `docs/optimization-plan.md`。

## [Unreleased] — optimize 分支

### P0-1 工具链与 CI（已验收）
- `63d3dea` 工作区设 `rust-version = "1.88"`，新增 `rust-toolchain.toml`（stable + rustfmt + clippy）与 `.github/workflows/ci.yml`。
- `7bf4acc` 全工作区 `cargo fmt`，仅格式变更。
- `f2ee961` desktop job 先构建 `desktop/ui` 生成 `ui/dist`；core job 在 1.88（MSRV）与 stable 双版本运行。
- 验收：审查确认 fmt 干净、MSRV 真正验证、Windows CI 不再因缺少 `ui/dist` 失败。

### P0-2 clippy 清零与 panic 点（已验收）
- `de7a8be` `frames.rs` 用 `total_cmp` 替代 `partial_cmp().unwrap()`，新增回归测试 `nan_inputs_do_not_panic`。
- `timeline.rs`/`media.rs`/`ffmpeg.rs` 正则改为 `LazyLock` 静态编译一次。
- `process.rs`、`ytdlp.rs`、`root.rs` 去除 unwrap；删除未使用的 `YtDlpJson` 等。
- 结果：avr-core `clippy --all-targets -D warnings` 零警告，测试 15 → 16 全部通过；Tauri crate 依赖 Windows CI 验证。

### P0-3 任务锁（已验收）
- P0-3 `d77a44d` 任务锁改用 fs4 建议锁 + RAII 守卫，修复等待方误报成功，测试 16 → 21。已验收：1.88 下 21 个测试全过，`fs4 0.13` 满足 MSRV（见 ADR 0004）。

### P0-4 平台识别（已验收）
- P0-4 `5d7af10` 新增 `Platform::Local` 与 `parse()`，本地转写不再标成 Bilibili，旧缓存兜底兼容，测试 21 → 22。已验收（P0 全部关闭）。

- P1 架构设计 `docs/architecture/p1-design.md`（架构师）。

### P1 架构改造（已验收，见 ADR 0005）
- `34ea752` P1-1 新增 `Source` 与统一入口 `analyze(ctx, src, opts, deps)`；`Deps` 字段私有，`AnalyzeOptions` `#[non_exhaustive]` + Default；附 `docs/architecture/p1-design.md`。
- `8920c25` P1-2 删除 `shared_job_manager`，GUI 用 `.manage(AppState)`，CLI/MCP 用 `AppState::load()`。
- `2a67672` P1-3 `ToolId`、`Tools` trait、`RealTools`/`FakeTools`，新增 2 个无需外部程序的流水线测试。
- `7db85d6` 修复 1.88 clippy 的 `uninlined_format_args`。
- `a366f8e` P1-4 `AvrError`/`ErrorCode` `#[non_exhaustive]`，`#[source]` 保留原因链；desktop 新增可序列化 `CommandError`。
- `f17a0e7` P1-5 MCP payload 拼装移到 `desktop/src-tauri/src/mcp.rs`。
- `316dd46` + `00c75a4` 删除旧接口 `analyze_url`/`transcribe_local`。
- 结果：测试 22 → 29，1.88 与 stable 下 clippy 零警告，CI 三个 job 全绿。

**行为变化**：本地文件缺 ffmpeg 时到抽音频才报错；GUI 返回的错误改为对象（前端读 `e.display`）。
**未验证**：`App.svelte` 与 GUI 实际行为需在 Windows 上手动运行确认。

### 进行中
### P2 缓存与流水线去重（待验收）
- `2b99f09` P2-3 `CACHE_SCHEMA = 2` 与 `AsrFingerprint` 写入 `metadata.json`，格式版本或 ASR 模型/版本不符即视为未命中（字幕缓存不受影响）；`Tools::version()`，`RealTools` 用可执行文件"大小:修改时间"；音频先写临时文件再改名。
- `15fa74a` P2-2 两条路径共用 `transcribe_and_save`，FakeTools 覆盖 ASR、复用音频、字幕三种情况；暂用 `#[allow(clippy::too_many_arguments)]`，P3-2 收成 `StageCtx`。
- 结果：测试 29 → 32，1.88 与 stable 下 clippy 零警告，CI 全绿。
- 行为变化：升级后旧缓存全部失效一次；已有 `audio.wav` 会复用；本地文件先检查 ASR 配置再跑 ffmpeg；本地文件也发 asr 进度事件。
- ~~P2-1 入口去重~~：审查核对后划掉，三个入口不存在真正重复，加 `entry` 模块反而增加耦合。

### P2-3 修复：ASR 内容指纹（已验收）
- `d3aa22d` 模型与 whisper 改用内容指纹（大小 + 头尾各 1MB 的 xxh3），不含路径；回归测试覆盖换模型、touch、复制三种情况。
- `f1d5946` 修复偶发失败：修改时间距今不到 2 秒的文件不写入指纹缓存。

### P3（部分验收）
- `d83d16c` P3-1 基线（已验收）：新建 `crates/avr-mcp`，payload 拼装改为纯函数；ffmpeg 现生成 6 秒测试视频 + FakeTools 固定转写；insta 快照三份，关键帧只记结构；CI 设 `AVR_REQUIRE_FFMPEG=1`。
- `eebb055` P3-2 按需读取（打回）：摘要索引、`read_transcript`（cursor 分页）、`get_frame`、帧 resource；`StageCtx` 去掉 allow；往返测试逐字段一致；首次返回体积的对比仅来自 6 秒合成视频，待效果集上的真实视频验证，不作结论。
  - 打回原因 1：路径穿越只在 `load_result` 校验，`list_visual_hints`、`extract_frames` 未校验，可读写缓存目录外文件。修复：私有字段的 `MediaKey::parse` + `MediaKey::dir(&root)`，所有拼路径处只接受 `MediaKey`，各补穿越测试。
  - 打回原因 2：未过 P3-0 准入就把默认切到索引模式。修复：默认改回 `compact`，`index` 显式指定才生效。
- 未完成：短视频直接返回全文的开关、MCP 工具调用日志、rmcp 升级（暂缓，等效果集证明索引模式不掉分）、`avr-mcp` 进程内 MCP 集成测试。

### 进行中
- P3-0 效果集：Tears of Steel、Sintel（CC BY），中文为中国新闻网《周建平谈中国空间站：怎么建？有何用？》（Wikimedia Commons，CC BY 3.0，3 分 37 秒新闻采访；限制：新闻采访语音，未覆盖口语化 UP 主风格与方言）；中新社视频许可核实后可替换）；按固定网址与哈希下载，不进仓库，`NOTICE` 注明来源。
- P3-2 MCP 按需读取：`analyze` 只返回摘要索引，新增按时间范围取转写、按编号取关键帧两个只读工具；参数收成 `StageCtx`。
