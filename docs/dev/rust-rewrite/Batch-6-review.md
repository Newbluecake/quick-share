# Batch 6 两阶段评审：T-014 至 T-016

> 日期：2026-07-27
> 范围：`receive`/`devices`、`send`/设备选择、严格 Web 回退、生产 TCP + Noise 编排、文本与剪贴板端到端传输
> 结论：**通过，允许进入 Batch 7 人工闸门；Batch 6 范围内无未解决 P0/P1**

## 1. 评审范围

- `crates/quick-share-cli/src/app.rs`
- `crates/quick-share-cli/src/devices.rs`
- `crates/quick-share-cli/src/orchestration.rs`
- `crates/quick-share-cli/src/terminal.rs`
- `crates/quick-share-platform/src/network.rs`
- `crates/quick-share-discovery/src/lib.rs`
- `crates/quick-share-protocol/src/lib.rs`
- `crates/quick-share-transfer/src/direct.rs`
- `crates/quick-share-transfer/src/offer.rs`
- `crates/quick-share-transfer/src/receiver.rs`
- `crates/quick-share-transfer/src/sender.rs`
- `crates/quick-share-transfer/src/text.rs`
- CLI/devices/orchestration/direct/protocol/receiver/sender/text tests

本批次首次把 Batch 2～5 的协议、身份、发现、offer、发送、接收和恢复核心接入真实 `quick-share send`/`receive` 进程。传统 Web HTTPS 服务仍由 T-017 交付；当前 `DeferredWeb` 只证明路由边界，选择 Web 后会明确报出尚未实现，而不会伪装成功或降级到 HTTP。

## 2. 阶段一：规范符合性评审

### 2.1 T-014：`receive`、`devices` 和接收终端体验

结论：**通过**。

- `receive` 已接入真实 TCP listener、Noise XX responder、INFO/能力协商、offer prompt、receiver service、进度 channel 和 mDNS advertisement；
- mDNS 只发布 listener 实际绑定的 LAN IP；loopback listener 不发布，并明确提示使用 `--peer host:port`；
- accept loop 使用 16-slot semaphore 和容量 16 的 session result channel；超过并发握手上限时丢弃连接并给出警告，不创建无界任务；
- offer prompt 使用 mutex 串行化，避免多个并发请求把终端输入互相串线；offer manager 继续执行 pending queue、rate limit、trusted auto 阈值和 owner isolation；
- unknown peer 显示设备、地址、清单、总大小、6 位 SAS 和 TOFU 状态；identity changed 明确阻止自动信任；
- 非 TTY unknown offer 默认拒绝；显式 `--yes` 最多产生 `AcceptOnce`，不能写入长期 trust；`AcceptAndTrust` 只有明确输入并匹配 SAS 后才带 `sas_verified = true`；
- trusted `auto`/`confirm` 使用 durable trust store 的完整 static key，而不是名称、IP、mDNS 短指纹或 SAS；
- `devices list/rename/remove` 支持 human/JSON 输出和 remove confirmation；只输出域分离短 fingerprint，完整 static key 和私密材料不会写入终端；
- trusted device 名称拒绝控制字符；设备身份和 trusted record 的 `Debug` 均隐藏 key；
- 第一次 Ctrl+C 取消 accept/session、等待有界 grace period并调用 `ReceiverService::pause_all`；第二次 Ctrl+C 明确 force exit 130；真实进程 SIGINT 已验证第一次 graceful exit 0；
- 默认接收目录来自系统 Downloads/config；`--output` 可覆盖目录。文本文件输出使用已记录的确定规则：现有目录或无扩展名新路径表示接收目录，新扩展名路径表示 one-shot 文本文件；文本文件必须配合 `--once`，且绝不覆盖已有目标；
- Windows 只读执行 `Get-NetConnectionProfile`。Public profile 会提示入站可能被阻止、建议用户仅在明确选择后为目标 profile/程序配置规则，并明确声明 Quick Share 没有修改 firewall；
- Windows 真机运行前后 firewall rule 集合 SHA-256 完全一致，证明诊断没有静默修改规则。

### 2.2 T-015：`send`、设备选择与严格 Web 回退

结论：**通过**。

- `SendRoute::{Auto, ExplicitPeer, Web}` 是封闭状态；`ScanResult::is_definitive_empty()` 是自动 Web 回退的唯一入口；
- discovery error、partial empty、发现到 peer 后的拒绝、确认超时、身份错误、连接失败、完整性失败和传输失败均不会调用 Web adapter；
- `--peer` 跳过自动 discovery 回退路径；设备 ID 只在 discovery 中定位指定 peer，`host:port` 直接解析连接；`--web` 完全跳过 discovery；
- 自动发现一个 peer 时终端明确确认；多个 peer 必须交互选择，非交互或越界输入返回 usage error，不偷偷选第一个；
- `ClientConnector` 执行 TCP connect、固定 Noise XX handshake、可选完整 static-key pin、selected device ID 验证、INFO exchange 和 capability intersection；
- discovery 的 device ID/fingerprint 都必须与 Noise 认证后的 remote static identity 一致；可信 key 变化返回 identity error；
- `ServerContext` 在 INFO 前不接受 offer/data；offer 必须符合协商后的 Files/Directories/Symlinks/Text/Resume 能力；
- accepted offer 的 bearer 绑定 peer、transfer、manifest、permission 和 expiry。重连 socket 重新执行 Noise + INFO 后可直接使用原 bearer 查询 status 并恢复，不必重放已确认 offer；
- operation timeout 与用户确认 timeout 已分离：TCP/handshake 使用短 wall-clock deadline，offer 等待使用 offer policy 的 120 秒截止，客户端留出 130 秒响应窗口；
- protocol/authorization/resource/integrity/state error 通过严格 `ProtocolError` 和稳定 `ErrorCode` 回传，客户端映射为 identity/integrity/rejected/network 等稳定 CLI 错误；
- status/chunk/complete 取消时 sender 尽力发送 `TransferCancel`；远端 integrity error 不再被折叠成普通网络失败；
- 当前 Web adapter 明确返回“HTTPS server 由 T-017 交付”；默认不会启动 HTTP，也不会宣称传统分享已成功。

### 2.3 T-016：文本与剪贴板端到端传输

结论：**通过**。

- `--text` 和 `--clipboard` 与 paths 构成互斥 content input；空文本、Unicode 和 1 MiB 上限均使用 `TextPayload` 校验；
- 文本 offer 使用 `ContentKind::Text`、有界单 entry、最终 BLAKE3，并在 media type 中记录 `source=literal` 或 `source=clipboard`；
- `TransferSender::send_text` 直接从有界内存 bytes 分块进入 Noise transport，不创建明文临时源文件；
- receiver 只有在所有块 durable、missing 为空且完整摘要匹配后才通过 authenticated owner-only `completed_text` 读取文本；文本不会 commit 成普通 final file；
- 显式文本 `--output` 优先，使用 create-new/atomic no-clobber；无显式文件时按 native clipboard → stdout 降级；
- TTY stdout 使用 `SafeTerminalOutput` 转义 ESC、换行和其他控制字符；pipe stdout 保持原始 UTF-8 数据语义；
- 日志只输出来源、bytes、字符数和短 digest，不输出正文；`TextPayload::Debug` 隐藏正文；
- 文本不会自动执行、打开、解释或作为 shell/链接渲染；Linux 与 Windows 真机恶意 `$(touch ...)` payload 均原样传输且没有产生目标文件；
- 文本完成并交付后清理 receiver staging；交付失败不会伪造 clipboard/file 成功信息。

## 3. 阶段二：代码质量与安全评审

结论：**通过；Batch 6 范围内无未解决 P0/P1。**

### 3.1 状态边界与失败语义

- discovery 的 definitive empty/partial/error 不共享空 `Vec` 语义，自动回退条件可静态审查；
- 一旦 route 选为 direct，后续 adapter error 直接返回，不存在 catch-all Web fallback；
- server 只在 Noise static identity 和 INFO 完成后处理 offer/data；每个 transfer frame 仍由 receiver 做逐操作 bearer 校验；
- request/response 同时验证 request ID 和 message type；strict decoder 拒绝 unknown fields、控制字符和非法 authorization 状态；
- Noise/framing/network error 关闭当前 session；短暂网络失败由有限 reconnect/retry 恢复，fatal/integrity/identity/invalid-response 不重试；
- receiver disconnect guard 释放 request-local upload slot并保留 durable chunk/journal；成功、拒绝和取消具有显式 session outcome。

### 3.2 资源与并发

- listener 连接、session result、progress、sender work queue、receiver task/file streams、offer queue 和 frame/chunk/text payload 全部有硬上限；
- prompt 串行、数据 session 有界并发；blocking manifest/hash/file I/O 和 terminal prompt 不阻塞 async reactor；
- progress channel 可丢弃中间刷新但不丢最终 transfer result；
- graceful shutdown 有界等待 session permit 归还，随后统一持久化 Paused，不无限等待慢 peer。

### 3.3 终端、敏感数据与平台安全

- 所有来自 peer、path、network diagnostic 和 backend error 的 human terminal 文本均通过 control-character escaping；
- private key、完整 static public key、authorization bearer 和 text body 的 Debug/Display 路径已审查为脱敏；
- Windows network diagnostic 只有读取命令，没有 firewall/profile 写命令、提权或自动规则变更；
- 文本输出文件拒绝覆盖；received text 永不执行；unknown non-TTY 默认拒绝；`--yes` 不创建长期信任。

### 3.4 评审中发现并已修复

| 级别 | 发现 | 修复与回归 |
|---|---|---|
| P1 | direct offer prompt 最初拥有独立于 `OfferManager` 的 confirmation timeout；两者漂移时，network timeout 到达后 `expire(now)` 可能尚未达到 manager deadline，客户端会继续等待 | 删除 direct 层重复配置，网络 prompt 直接读取 `OfferManager::confirmation_timeout()`；新增 20 ms slow-prompt Red 测试，确认立即 Expired/Rejected 且不返回授权；Linux/Windows 均通过 |
| P1 | 初版远端 receiver error 被统一折叠，发送端无法区分 integrity、unauthorized、resource limit 和 invalid state | 增加严格 `ProtocolError` codec 和稳定错误码，服务端按 receiver error 分类，客户端映射到稳定 CLI/transport 错误；corrupt digest 端到端传播为 Integrity |
| P1 | sender 在 status/chunk/complete 取消时可能只停止本地 worker，不通知接收端 | 在取消路径尽力发送 bearer-bound `TransferCancel`，receiver 保持明确 Cancelled/Paused 语义 |
| P2 | 连接和用户 offer 确认最初共用短 operation timeout，交互用户可能来不及确认 | TCP/handshake 约 10 秒；manager prompt 120 秒；client offer response 130 秒，且全部为 wall-clock deadline |
| P2 | mDNS 初版可能发布与 listener 实际绑定不一致的接口 | 新增 `start_for_ips`，只发布真实绑定 LAN IP；loopback 不发布并提示 explicit peer |
| P2 | `receive --output` 同时承担目录和文本文件，缺少稳定可解释的区分 | 固化并写入 help：现有目录/无扩展名新路径为目录，新扩展名路径为 one-shot 文本文件；新增分类测试，文本文件强制 `--once` 且 no-clobber |
| P2 | trusted record 的派生 `Debug` 会包含完整 public key | 改为自定义脱敏 `Debug`；devices human/JSON 只暴露短 fingerprint |
| P2 | Windows 真机最初“卡住”实际是 SSH→`cmd.exe` 引号导致发送进程未启动 | 改用 PowerShell 参数数组执行测试；确认协议没有卡死，并完成 Windows→Linux 真机文件/剪贴板传输 |

## 4. 验证结果

```text
cargo fmt --all -- --check                                           PASS
cargo clippy --workspace --all-targets --all-features -- -D warnings PASS
cargo test --workspace --all-targets --all-features                  PASS
  129 tests listed; 128 passed + 1 true-host mDNS ignored
cargo check --workspace --all-targets --all-features \
  --target x86_64-pc-windows-gnu                                    PASS
cargo check --manifest-path fuzz/Cargo.toml --locked                 PASS
cargo build --workspace --release                                   PASS
cargo audit                                                          PASS
  238 dependencies; no advisory
python -m pytest -q                                                  PASS (339 tests)
git diff --check                                                    PASS
```

Linux 生产 CLI：

- explicit `--peer` 文件传输：Noise/SAS/TOFU/offer/status/chunk/complete/final payload 全链通过；
- LAN mDNS 自动发现唯一 receiver：明确选择 encrypted direct，未调用 Web；
- definitive zero peer：约 2 秒后安全选择 HTTPS Web adapter，并明确返回 T-017 尚未实现；
- Unicode/空文本/恶意 shell 文本：显式输出字节一致，未执行正文；
- unknown non-TTY 且无 `--yes`：sender exit 4、receiver 安全拒绝、无 final；
- 真实 SIGINT：第一次 Ctrl+C 输出 graceful snapshot 文案并 exit 0。

Windows 11 x86_64 真机：

- 最新 direct 集成测试 3/3：TCP + Noise 文件传输、断线重连、Unicode/空文本、integrity error、confirmation timeout；
- orchestration 7/7、devices 2/2、platform 2/2；
- Windows 本机 production CLI loopback：Unicode 文件名文件传输和 Unicode literal text 输出均 byte-for-byte 一致，双方正常停止；
- Windows → Linux production CLI 文件传输：双方 exit 0、payload 一致；
- Windows native clipboard → Linux explicit text output：双方 exit 0，`你好🚀 $(touch ...)` byte-for-byte 一致，命令未执行；
- Public profile runtime diagnostic 输出可操作警告；运行前后 firewall rule 集合 SHA-256 一致；
- Linux → Windows 跨子网入站连接被当前 Public-profile firewall 阻止；Windows loopback listener/direct/CLI 均通过，因此判定为环境入站策略而非协议或 Windows binary 故障。产品按要求只提示，不自动修改 firewall。

Fuzz：

- 固定 `nightly-2026-07-01` 和 `cargo-fuzz 0.13.2`；
- `noise_records` 覆盖 ciphertext record、segment/control/chunk parser；
- 本批最终专项运行 823,291 inputs / 21 秒，RSS 约 538 MiB；无 crash、panic、OOM 或 sanitizer finding；
- fuzz workspace `--locked` 检查通过。

## 5. 已知限制与后续硬门

| 级别 | 项目 | 后续处理 |
|---|---|---|
| Release Blocker | `snow 0.10.0` 仍无可引用的正式第三方安全审计 | T-024 前取得审计证据/额外专家评审，或迁移到满足发布标准的实现 |
| T-017 | 真实传统 Web HTTPS server 尚未实现 | 下一批实现 catalog、流式下载和安全路径；当前 `DeferredWeb` 必须继续明确失败，不能伪装启动 |
| 平台门 | macOS Intel/Apple Silicon 尚未真机验证 | 发布前完成 listener、mDNS、staging、clipboard、symlink、installer/updater 矩阵 |
| 环境限制 | 当前 Windows 机器是 Public profile，跨子网入站被 firewall 阻止 | 保留只读诊断和用户明确操作提示；不得为了测试自动改 profile 或 firewall |
| 发布前 UX | core 已支持 sender/receiver state 重建和缺块恢复，production direct 支持同进程 socket reconnect；CLI 尚未提供跨 sender 命令调用的 pending-transfer 选择/恢复界面 | T-024 最终验收前补齐或明确设计为独立恢复命令；不得宣称任意发送端进程重启后会自动猜测并恢复旧任务 |
| 平台门 | Windows 非管理员且关闭 Developer Mode 的 symlink notice fallback 尚未在该配置真机验证 | 发布矩阵验证安全说明文件降级，不提权、不静默跟随 |

## 6. 最终结论

T-014、T-015、T-016 已满足 Batch 6 的接收终端、可信设备管理、真实 direct CLI、严格 Web 回退、文本/剪贴板、安全输出和 Windows 防火墙诊断目标。两阶段评审中发现的 confirmation timeout 双源问题和远端错误分类问题均已修复，并在 Linux/Windows 上回归通过。

Batch 6 可以关闭并停在 Batch 7 人工闸门。此结论不是发布批准：T-017 Web HTTPS、macOS 真机、`snow` 审计、完整发布矩阵和跨 CLI invocation 的恢复 UX 仍是后续硬门。
