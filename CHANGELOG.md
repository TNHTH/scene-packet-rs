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

### 进行中
- P0-4 `5d7af10` 新增 `Platform::Local` 与 `parse()`，本地转写不再标成 Bilibili，旧缓存兜底兼容，测试 21 → 22。已验收（P0 全部关闭）。
- P1-1～P1-5 按 ADR 0005 拆分进行（执行）。
- P1 架构设计 `docs/architecture/p1-design.md`（架构师）。
