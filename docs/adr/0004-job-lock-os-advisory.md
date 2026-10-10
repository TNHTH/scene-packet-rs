# ADR 0004：任务锁改用操作系统建议锁 + RAII 守卫
状态：已采纳（2026-10-10，`d77a44d`，待验收）

背景：原实现靠 `.lock` 文件存在与否判断，进程崩溃后锁残留；等待方 `rx.await.unwrap_or(Ok(()))` 会把中断误报为成功。
决策：用 `fs4::try_lock_exclusive`（flock / LockFileEx），崩溃由操作系统释放；`FileLock` 与 `InFlight` 守卫在 Drop 时解锁并通知等待方；执行方中断时等待方收到 `JobCancelled`。
备选：`create_new` + PID/时间戳过期接管（需自行处理过期判断，放弃）。
后果：新增依赖 `fs4 0.13`，需 CI 1.88 路确认 MSRV。
