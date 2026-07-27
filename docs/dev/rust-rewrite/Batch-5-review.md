# Batch 5 两阶段评审：T-011 至 T-013

> 日期：2026-07-26
> 范围：授权后分块接收、安全提交、有界并发发送、进度/取消、断点续传、幂等与有限重试
> 结论：**通过，允许进入 Batch 6 人工闸门；无未解决 P0/P1**

## 1. 评审范围

- `crates/quick-share-protocol/src/lib.rs`
- `crates/quick-share-transfer/src/lib.rs`
- `crates/quick-share-transfer/src/network.rs`
- `crates/quick-share-transfer/src/noise.rs`
- `crates/quick-share-transfer/src/receiver.rs`
- `crates/quick-share-transfer/src/sender.rs`
- protocol/noise/receiver/sender/store/resume tests
- workspace/fuzz dependencies、lockfile 和 fuzz target

本批次交付可被 Batch 6 CLI/accept loop 接线的传输核心。真实 kernel TCP、authenticated Noise frame 和 receiver commit 已分别集成验证；最终 `send`/`receive` 命令、peer 选择、监听生命周期和终端渲染属于 T-014～T-016。

## 2. 阶段一：规范符合性评审

### 2.1 T-011：接收端分块上传、校验与完成提交

结论：**通过**。

- QSP/1 新增稳定的 `TRANSFER_STATUS`、`CHUNK_DATA`、`CHUNK_ACK`、`TRANSFER_COMPLETE` 和 `TRANSFER_CANCEL` 类型；安全敏感结构使用严格 unknown-field decode；
- manifest 的 file/text entry 携带最终 BLAKE3，chunk descriptor 携带 transfer、entry、index、offset、length 和 chunk BLAKE3；
- chunk 默认配置仍为 4 MiB，协议允许 256 KiB～16 MiB；单个 `CHUNK_DATA` fragment 最大 1 MiB，逻辑 Noise frame 最大 8 MiB，总 chunk 数最大 1,000,000；
- `ChunkData::validate` 在写入前检查非空 payload、fragment offset、checked end、descriptor length 和 `final_fragment` 一致性；
- 每次 status/chunk/complete/cancel 都重新校验 authorization 的 bearer、peer `DeviceId`、`TransferId`、permission 和 expiry；未经确认、错误/过期 token、跨 peer/transfer 请求均 fail closed；
- receiver 将 fragment 依序流入 `ChunkUpload`，不聚合完整文件；chunk 完成后先校验摘要、sync 数据，再原子更新 journal，最后才 ACK；
- 已验证相同 chunk 重放返回 duplicate ACK；同 index 的 length/digest 冲突、同 request ID descriptor 变化和乱序 fragment 均拒绝；
- 默认同时接收任务为 1、文件 stream 为 4；状态机和测试覆盖背压、断线释放 stream slot、暂停恢复及终态拒绝；
- receiver 提供子 `CancellationToken` 和有界 `ReceiverProgressEvent`；cancel 传播到活动任务，disconnect 清除请求局部 upload 并持久化 `Paused`；
- complete 要求无活动 upload、无 missing chunk、manifest digest 不变；所有完整文件摘要验证后才提交；错误摘要标记 Failed，其他可恢复提交错误保留 Paused；
- 文件最终提交使用 `cap-std::Dir` 根能力、hard-link/rename 和原子 journal，符号链接祖先竞态不能把 final commit 重定向到根外；
- 目录和符号链接提交意图持久化，可在崩溃后幂等完成；不支持原生链接时写入安全说明文件，不跟随或执行目标；
- handshake 和 encrypted record 的异步 I/O 均有单次操作 wall-clock deadline 和 cancellation；空/超长记录、慢速客户端、I/O、Noise 或 framing 错误都会 shutdown 并永久关闭 session；
- authenticated Noise `CHUNK_DATA → ACK → COMPLETE → ACK` 已通过端到端内存 transport 测试，kernel TCP encrypted application frame 在 Linux 和 Windows 均通过。

### 2.2 T-012：发送端并发调度、进度与取消

结论：**通过**。

- `TransferPlan::from_offer` 将 accepted wire offer 与本地 immutable source manifest 按 entry ID、相对路径、kind、size 和最终摘要绑定；
- `TransferSender` 默认 4 个 worker 和容量 16 的 bounded queue；策略拒绝零值、超过 64 worker 或超过 1024 queue 的配置；
- producer 只入队 receiver bitmap 标记为 missing 的固定 chunk；10,000 个小文件不会创建等量 task，也不会同时打开等量文件；
- 每个 worker 一次只打开一个 source file，按 u64 offset 读取一个有界 chunk；大于 4 GiB 稀疏文件只读取缺失尾块；
- 文件读取、前后 source snapshot 复核和 BLAKE3 hashing 均在 blocking worker 中执行，不阻塞 async executor；
- regular files、多个文件、空文件和包含空目录的目录 manifest 均有测试；目录/链接只进入 metadata offer，不进入 payload worker queue；
- sender 在首次 status 前、每个 chunk 读取前后及 complete 前重新校验 source snapshot；变化后发送 `SourceChanged` cancel 且不重试；
- `ProgressTracker` 从 resume byte count 起步，只在 durable ACK 后增加 current/uploaded chunk；current/total 单调且不超过 total，提供速度和 ETA；
- sender/receiver 取消均终止 producer、worker、frame send 和 retry sleep；本地取消尽力发送 remote cancel；远端 Cancelled/Fatal 不重试；
- authorization 在 sender 内以 `Arc<AuthorizationProof>` 持有，每个 wire proof 析构时 zeroize；不再把长期 raw `[u8; 32]` bearer 副本分发给 worker。

### 2.3 T-013：断点续传、幂等请求和有限重试

结论：**通过**。

- `TransferStatusResponse` 返回按 entry 的 compact missing bitmap，并限制 entry 数、单 entry/总 chunk 数、bitmap 长度及尾部未使用 bit；
- durable state 同时绑定 sender `DeviceId`、`TransferId`、offer/manifest digest、chunk descriptor 和完成 digest；不同 sender 或 manifest 不能复用 staging；
- sender 根据 bitmap 仅重建缺失 work item；已完成 bytes 进入进度基线但不会重复计入 uploaded bytes；
- 每个 chunk 边界（0、1、2、全部完成）均验证 drop/reopen 后 missing 集合精确，随后可完成并得到原始 payload；
- sender process、receiver service 和 offer manager 同时重启后可从 durable journal 恢复，只发送缺失块；损坏 JSON 安全失败且不暴露 final file；
- 同一逻辑 chunk 的 request ID 在所有 fragment 和重试间保持稳定；丢 ACK 后 receiver 可用 descriptor/durable chunk 状态返回幂等 ACK；
- status、chunk 和 complete 均使用有限 retry；默认最多 3 个总 request attempt，指数退避、正抖动和最大 delay 均有硬限制；Fatal、Cancelled、source change 和 invalid response 不重试；
- receiver 提供 resumable list、授权 cancel 和本地 administrative cleanup API；网络错误不会自动删除可恢复 staging。

## 3. 阶段二：代码质量与安全评审

结论：**通过；无未解决 P0/P1。**

### 3.1 边界、内存与资源

- 所有 wire collection、bitmap、payload、chunk、manifest、logical frame、handshake 和 ciphertext 长度均有硬上限；分配前执行 checked conversion；
- sender 内存上界由 worker 数 × chunk size 加 bounded queue 控制；receiver 内存上界由最多 4 个 fragment stream、1 MiB frame 和全局 chunk/journal 限制控制；
- full file hashing 使用固定缓冲，receiver 不读取完整文件到内存；
- progress channel 使用 `try_send`，慢 UI 不会反向造成传输内存无界增长；最终结果仍由 `TransferSummary`/terminal status 返回；
- 断线清除 request-local upload 对象，不泄漏 file stream slot；staging 数据和已 sync journal 保留用于 resume。

### 3.2 完整性、持久化与 TOCTOU

- ACK 顺序为 data write → chunk digest → data sync → journal atomic replace → ACK；完成顺序为 all chunks → full digest → persisted commit intent → capability-bound final operation → terminal journal；
- crash-before/after state replace、commit intent、final rename 和 metadata side effect 均有 fault/restart 测试；
- final path 在完整摘要验证前不可见；摘要失败、取消和不完整 transfer 均不会产生伪完整目标；
- conflict policy 在提交时重新判断；已有目标不被静默覆盖，overwrite 必须显式；
- final operation 的 path traversal 使用 capability root，而不是仅依赖词法 `starts_with`；测试用 raced symlink ancestor 证明无法逃逸；
- Windows 不支持普通 `File::open(directory).sync_all()`，因此仅在 Unix fsync final parent；Windows 依赖文件/状态 FlushFileBuffers 和原子 rename，其行为已由真机 crash/commit 测试覆盖。

### 3.3 Network、重放与敏感数据

- Noise suite 仍固定，不引入新密码原语；`NetworkSession` 只负责 bounded length-prefix I/O、deadline、cancellation、ordered `SecureChannel` 和 fail-closed shutdown；
- deadline 覆盖完整 handshake packet、record read/write 和完整 logical-frame reassembly，而不是每读一个 byte 重置，慢速客户端无法无限延长；
- stateful Noise nonce/order 防 ciphertext replay；request ID 和 durable chunk descriptor 防语义重放；
- bearer Debug 始终脱敏，protocol proof、offer token 及 sender worker 共享 proof 在 Drop 时清零；payload Debug 不输出正文或 bytes；
- network errors 不包含本地绝对路径、token、完整静态公钥或 payload 内容。

### 3.4 评审中发现并已修复

| 级别 | 发现 | 修复与回归 |
|---|---|---|
| P1 | 初版只有同步 Noise record helper，未来慢 socket 可无限阻塞，framing error 后 close 语义不集中 | 新增 async handshake/record wall-clock deadline、cancellation、shutdown 和 closed state；慢 header、超长 record、kernel TCP 测试通过 |
| P1 | final commit 最初使用普通路径检查，仍存在 ancestor symlink TOCTOU 窗口 | final create/link/rename 改为 `cap-std::Dir` 根能力；raced ancestor 测试证明无法写出根目录 |
| P1 | Windows 对目录执行普通 file sync 导致完成阶段 PermissionDenied/NotFound | parent directory fsync 限定 Unix；Windows 文件/journal flush 和原子提交真机回归通过 |
| P1 | Windows 存储 `docs/data.bin` 相对 symlink 后无法解引用（error 123） | Windows 创建时仅转换路径分隔符，并规范化比较 stored target；真机 link read-through 通过 |
| P1 | sender 最初将 raw bearer `[u8; 32]` 复制给每个 worker | 改为共享 `Arc<AuthorizationProof>`，wire clone 均有 Drop zeroize |
| P2 | chunk retry 最初重新生成 request ID，弱化 request-level idempotency | request ID 移到 retry loop 外，同一 chunk 所有重试保持稳定；并发 retry 回归验证 |
| P2 | chunk BLAKE3 最初在 async worker 上直接执行 | 读取、snapshot 复核和 hashing 一并放入 `spawn_blocking` |
| P2 | receiver 最初只有状态取消，没有可传播 token 和有界进度事件 | 每个 active transfer 增加 child cancellation token、恢复 byte 基线和 bounded progress event |
| P2 | 恢复测试只覆盖一个中断点 | 增加每个 chunk boundary、双进程重启、different sender、manifest mismatch 和 corrupt state 测试 |

## 4. 验证结果

```text
cargo fmt --all -- --check                                           PASS
cargo clippy --workspace --all-targets --all-features -- -D warnings PASS
cargo test --workspace --all-targets --all-features                  PASS
  114 tests listed; 113 passed + 1 true-host mDNS ignored
cargo check --workspace --all-targets --all-features \
  --target x86_64-pc-windows-gnu                                    PASS
cargo check --manifest-path fuzz/Cargo.toml --locked                 PASS
cargo build --workspace --release                                   PASS
cargo audit                                                          PASS
pytest -q                                                            PASS (339 tests)
git diff --check                                                    PASS
```

专项覆盖：

- 严格 message code/control codec、1 MiB fragment、8 MiB logical frame 和 malformed chunk；
- 未授权、过期、跨 peer/transfer/permission、重复/冲突 chunk；
- 慢 handshake/record、invalid framing、session close、kernel TCP encrypted frame；
- 目录、空文件、多文件、10,000 小文件、超过 4 GiB 稀疏文件；
- 4 worker/queue 背压、progress、Ctrl+C、peer cancel、source change；
- 每个 chunk boundary、sender/receiver 重启、different sender/manifest、corrupt state；
- data/journal/rename/metadata fault injection、摘要错误、capability TOCTOU；
- authenticated Noise chunk/ACK/complete/ACK 端到端路径。

Windows 11 x86_64 真机专项：

- Noise/network 14/14；
- receiver 4/4；
- resume 3/3；
- sender 9/9；
- store 12/12；
- 合计 42/42，包含 kernel TCP、Windows relative symlink、目录提交、4+ GiB offset 和 10,000 小文件。

Fuzz：

- 固定 `nightly-2026-07-01`、`cargo-fuzz 0.13.2`；
- target 同时覆盖 ciphertext record、authenticated segment parser、strict INFO control decode 和 strict `ChunkData` decode；
- 本批最终运行 1,489,089 inputs / 21 秒，RSS 约 526 MiB；无 crash、panic、OOM 或 sanitizer finding；
- corpus、artifacts 和 fuzz target build 目录已清理，仓库仅保留 lockfile、manifest 和 target source。

RustSec：扫描 237 个 lockfile dependencies，无已知 advisory。

## 5. 已知限制与后续硬门

| 级别 | 项目 | 后续处理 |
|---|---|---|
| Release Blocker | `snow 0.10.0` 仍无可引用的正式第三方安全审计 | T-024 前取得审计证据/额外专家评审，或迁移到满足发布标准的实现 |
| P1（Batch 6） | 本批已验证 kernel TCP 和 authenticated Noise transfer core，但尚无最终 CLI accept/connect/reconnect dispatcher | T-014/T-015 接线真实 listener、peer connection、request dispatch、session replacement 和终端生命周期；随后执行 Linux↔Windows CLI 跨机传输 |
| P1（平台） | macOS Intel/Apple Silicon 尚未真机运行 | 发布前完成 network、staging、symlink、clipboard 和 updater 矩阵 |
| P1（T-016） | 最终 SAS 确认 UI 尚未接线 | 只有用户明确比较后才能传 `sas_verified = true`；否则必须标注 TOFU |
| P2 | 目录 transfer 只保证每个文件原子，不保证整个目录树同时出现 | 保持设计中的明确语义；CLI 文案不得宣称目录任务整体原子 |
| P2 | Windows 非管理员且关闭 Developer Mode 的 symlink fallback 尚未在该配置真机验证 | Batch 6/发布矩阵验证说明文件降级，不得尝试提权或静默跟随链接 |
| P2 | progress channel 满时允许丢中间事件 | 最终 summary/status 不丢；Batch 6 renderer 应按最新事件刷新而非假设逐事件到达 |

## 6. 最终结论

T-011、T-012、T-013 满足 Batch 5 的授权数据面、流式完整性、安全提交、有界并发、进度/取消、断点续传、幂等和有限重试目标。两阶段评审通过，可以停在 Batch 6 人工闸门。

此结论不是发布批准，也不是 CLI 功能已完成：最终 listener/connector/dispatch 与 `send`/`receive` UX、严格 Web 回退和文本/剪贴板接线仍属于 T-014～T-016；`snow` 审计和 macOS 真机矩阵仍是发布硬门。
