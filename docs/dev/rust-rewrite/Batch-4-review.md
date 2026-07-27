# Batch 4 两阶段评审：T-009 至 T-010

> 日期：2026-07-26
> 范围：Noise XX、SAS、完整静态公钥 pinning、QSP 有界 frame、peer authentication、offer/确认/授权状态机
> 结论：**通过，允许进入下一人工闸门；`snow` 正式审计仍是发布硬门**

## 1. 评审范围

- `crates/quick-share-protocol/src/lib.rs`
- `crates/quick-share-core/src/identity.rs`
- `crates/quick-share-transfer/src/noise.rs`
- `crates/quick-share-transfer/src/auth.rs`
- `crates/quick-share-transfer/src/offer.rs`
- 对应 protocol/auth/noise/offer tests
- `fuzz/`、workspace dependency/lockfile 和 CI 变更

本批次建立安全连接与授权基础，但尚不负责真实 socket accept/connect、分块上传、receiver commit 或 CLI 接线；这些分别属于 T-011、T-012 和 T-016。

## 2. 阶段一：规范符合性评审

### T-009：Noise XX、SAS 和可信设备 pinning

结论：**通过**。

- Noise suite 是唯一编译期常量 `Noise_XX_25519_ChaChaPoly_BLAKE2s`；没有运行时 pattern/cipher/hash 协商，也没有自定义握手协议；
- `snow` 精确锁定为 `=0.10.0`，adapter 集中在 `noise.rs`，正式 crate 继续 `#![forbid(unsafe_code)]`；
- XX 三消息状态机显式限制读写顺序、4 KiB handshake packet 和绝对 deadline；过期、乱序、重复、截断和超长均 fail closed；
- 握手完成后提取双方认证的完整 X25519 static public key；`DeviceId` 必须由该完整 key 派生，pin 比较使用 constant-time equality；
- `HandshakeEvidence` 包含 remote device ID、完整认证 key、key fingerprint、handshake transcript 派生的 6 位 SAS；Debug 隐藏完整 key/fingerprint；
- SAS 使用域分离 BLAKE3 从 Noise handshake hash 派生，只用于人工比较，不作为长期身份或 token；双端一致，终止型 MITM 两侧 SAS 不同；
- `PeerAuthContext` 明确区分 `Unknown`、`Trusted`、`Changed`；覆盖声明 ID 与 key 不符、已 pin key 替换、同名不同 key；
- trusted peer 的 UI 名称取本地 trust store，peer-supplied name 只用于校验 wire claim，避免远端静默改写可信显示名；
- Unknown/Changed 在授权前只能使用 INFO 和 `OFFER_CREATE`，不能查询状态或上传；
- transport 使用 Snow 有状态 nonce；重复 ciphertext、截断 ciphertext、错误 key 和非协同 rekey 均失败；
- 单个 ciphertext record 上限约 60 KiB plaintext + 16-byte tag；单个逻辑 frame 最大 8 MiB、最多 256 segments；重组只允许一个 frame，拒绝交错、乱序、重复、不一致 segment count/length 和非法尾段；
- 网络长度前缀在分配前检查：handshake 4 KiB、ciphertext record 60 KiB 级别；
- INFO/OFFER control codec 同时绑定稳定 `MessageType`、严格 JSON unknown-field policy 和 payload 语义验证；设备名控制字符、超长 name 和超限 offer 不会越过 secure control boundary；
- 私钥临时副本使用 `Zeroizing`，Noise handshake/transport 不暴露 cryptographic Debug；测试确认错误和 Debug 不包含持久私钥。

### T-010：offer、确认和授权状态机

结论：**通过**。

- 实现严格 `INFO_REQUEST`、`INFO_RESPONSE`、`OFFER_CREATE`、`OFFER_STATUS` request/response wire 类型及稳定 message code；
- `OfferManager` 在保存前重新执行 protocol offer validation、portable `RelativePath` validation、sender identity binding 和资源上限；
- offer 保存不可变 manifest snapshot，确认后 caller 修改原对象不会改变授权内容；
- view model 仅提供本地可信显示名、transfer、条目、总大小、SAS、trust/change 状态和安全相对路径；
- 支持 AcceptOnce、AcceptAndTrust、Reject、Timeout 和 oneshot resolution；所有常规 API 操作都会先推进超时，过期 pending 不会长期占用队列；
- AcceptAndTrust 仅对 Unknown/Changed 且 `sas_verified = true` 生效，并通过 `TrustedDeviceStore::trust_peer` 原子持久化完整认证 key；
- trusted auto/confirm policy 在 offer 创建时重新读取 trust store，信任撤销后的陈旧 `PeerAuthContext` 不能自动通过；
- trusted auto 同时受 entry count 和 total byte 阈值限制；Changed 永远不能 auto accept；
- 随机 256-bit bearer token 由 OS CSPRNG 生成，manager 只保存域分离 BLAKE3 hash；token Debug 固定 `[REDACTED]`，Drop zeroize 原始 bytes；
- authorization 同时绑定 token、peer `DeviceId`、`TransferId`、manifest snapshot、permission 和 expiry；任一不匹配均统一返回 Unauthorized；
- 未确认阶段没有 token，伪造 token、跨 peer、跨 transfer、错误 permission、过期 token 均不能获得上传能力；
- offer/status owner isolation、transfer ID replay cache、每设备滑动窗口 rate limit、全局 pending queue 和 replay-entry 上限均已实现；
- Reject/Timeout 唤醒等待方且不生成授权；终态和过期 grant 有界清理。

## 3. 阶段二：代码质量与独立安全评审

结论：**通过；当前实现无未解决 P0/P1，保留发布级外部依赖风险。**

### 3.1 密码学与身份边界

- 使用标准 Noise XX 和库实现的 X25519/ChaChaPoly/BLAKE2s，不复制 Spike，不自行实现 DH/AEAD/handshake；
- 应用层 BLAKE3 仅用于域分离 SAS、identity/fingerprint 和 bearer hash，不替代 Noise authentication；
- 长期 trust 的判定源是完整 authenticated static key；mDNS name/IP/fingerprint 仍只是未验证 hint；
- 首次连接若用户不比较 SAS，安全声明仍只能是 TOFU/opportunistic authentication；实现没有把 6 位 SAS 当作长期 identity；
- 任意 pin mismatch、claim mismatch 或 trust-name collision 都进入 Changed/错误路径，不会降级成 Trusted。

### 3.2 Framing、内存与拒绝服务

- 所有外部长度均在分配前检查，整数运算使用 checked conversion/addition；
- exact 8 MiB 逻辑 payload 通过，8 MiB + 1 byte 被拒；
- fragment count 根据 total length 重新计算，非尾 segment 必须为固定最大长度，尾段必须精确匹配 remainder；
- reassembly 不支持 frame interleaving，异常后清空状态并返回错误；调用方在 T-011 必须把任何 Noise/framing error 作为关闭 session 的条件；
- offer queue、rate history、replay set、terminal retention 和 grant retention 均有显式上限或有效期。

### 3.3 TOCTOU、重放和授权

- offer snapshot 在 manager 内部 clone 后固定，grant 返回同一 snapshot；
- trusted auto 在决策点重新读取 durable trust store，而不是只信任握手时的 context；
- token 采用 hash-at-rest 和 constant-time compare；owner、transfer、permission、expiry 逐项检查；
- Noise transport replay 由 nonce/order 阻止，offer 语义重放由 `TransferId` 阻止；未来 CHUNK/RESUME 的 request-level idempotency 仍由 T-011/T-012 实现。

### 3.4 评审中发现并已修复

| 级别 | 发现 | 修复与回归 |
|---|---|---|
| P1 | control frame 最初只做严格 JSON decode，未强制调用 Info/Offer semantic bounds | 引入 `ControlPayload`，encode/decode 两侧均验证；增加控制字符 name 和 invalid INFO 测试 |
| P1 | pending offer 若没有外部显式调用 expire，可能持续占用队列 | 所有 manager API 的 purge 阶段自动推进 deadline 并唤醒 resolution |
| P2 | trusted peer 若改变网络声明名，UI 可能显示远端新名称 | 分离 `claimed_name` 与本地 `display_name`；wire 校验使用前者，UI 使用 trust store 名称 |
| P2 | control encode API 最初允许调用者传入与 Rust payload 类型不一致的 `MessageType` | `ControlPayload::MESSAGE_TYPE` 在 encode/decode 两侧强制绑定 |
| P2 | fuzz 最初只覆盖长度前缀和 INFO JSON | 增加 authenticated plaintext segment parser fuzz entry，覆盖 segment header/count/length/reassembly前置校验 |

## 4. 验证结果

```text
cargo fmt --all -- --check                                           PASS
cargo clippy --workspace --all-targets --all-features -- -D warnings PASS
cargo test --workspace                                              PASS
  91 tests listed; 90 default-pass + 1 true-host mDNS ignored
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu PASS
cargo check --manifest-path fuzz/Cargo.toml --locked                 PASS
cargo build --workspace --release --locked                           PASS
cargo audit                                                          PASS
pytest -q                                                            PASS (339 tests)
git diff --check                                                    PASS
```

专项安全测试覆盖：

- XX 双端 static key 持有证明与 encrypted transport；
- 双端 SAS 一致、MITM 两会话 SAS 不同、SAS 固定 6 位十进制；
- 正确 pin、错误 pin、key replacement、同名不同 key、声明 ID 与 key 不符；
- handshake deadline、顺序、重放、截断、超长；
- ciphertext replay、截断、超长、协同/非协同 rekey；
- exact 8 MiB frame、多段重组、乱序/交错/重复/长度不一致拒绝；
- offer replay、未确认上传、owner isolation、token/peer/transfer/permission/expiry binding；
- trusted auto/confirm、信任撤销、Changed 不 auto、resource threshold、rate/queue bounds；
- accept once/trust/reject/timeout、SAS gate 和不可变 manifest snapshot；
- Debug/error 私钥、完整 key、token redaction。

Fuzz：

- `cargo-fuzz 0.13.2`；
- 工具链固定 `nightly-2026-07-01`（最新 nightly 与当前 sanitizer/cargo-fuzz 组合不兼容）；
- target：length-prefixed ciphertext record、Noise segment parser、strict INFO control decode；
- 多轮本地 smoke 累计超过 300 万 inputs（单轮最高约 185 万），无 crash、panic、OOM 或 sanitizer finding；
- CI 新增固定 nightly、固定 cargo-fuzz 版本的 10 秒 smoke job。

Windows 11 x86_64 真机：

- Noise 12/12、auth 2/2、offer 7/7，全数通过；
- 测试目录已清理。

RustSec：扫描 223 个 lockfile dependencies，无已知 advisory。

Python 基线首次全量运行因既有 concurrent download stdout capture 偶发少捕获一条 completion log，结果 338/339；该单测隔离重跑通过，随后全量重跑 339/339 通过。未修改 Python 生产代码或测试。

## 5. 已知限制与后续硬门

| 级别 | 项目 | 后续处理 |
|---|---|---|
| Release Blocker | `snow 0.10.0` 没有可引用的正式第三方安全审计 | 依赖已精确锁定且 adapter 已专项审查/RustSec/fuzz；发布前 T-024 必须取得可接受审计证据、完成额外专家评审，或迁移到满足审计要求的实现 |
| P1（T-011） | 当前 handshake deadline 是状态机 deadline，不能自行中断未来阻塞 socket read | network driver 必须设置 wall-clock read/write timeout/cancellation，并在任何 Noise/framing error 后立即关闭 session |
| P1（T-011/T-012） | CHUNK/RESUME 尚未接入 authorization 和 request-level idempotency | 每个数据操作必须验证 grant identity/transfer/permission/expiry，并实现 chunk/request replay 语义 |
| P1（T-016） | `sas_verified` 目前是状态机输入，尚无最终终端交互接线 | UI 只能在用户明确比较并确认后传 true；文案必须说明未比较 SAS 的首次连接仅是 TOFU |
| P1（平台） | 当前正式模块已在 Linux/Windows 各自运行，但本批代码尚无 production socket 跨机链路 | T-011/T-013 后执行 Linux↔Windows 当前正式实现互操作；发布前补 macOS Intel/Apple Silicon |
| P2 | 每个 Noise connection 当前只允许单个非交错逻辑 frame 重组 | 有意限制以降低 DoS/状态复杂度；上层按 request 顺序发送，不并行交错 segment |
| P2 | fuzz corpus/artifacts 不提交仓库 | CI 每次从空 corpus 做 bounded smoke；发布前保留更长 fuzz session 的日志/摘要 |

## 6. 最终结论

T-009、T-010 满足 Batch 4 的安全连接、身份分类、SAS、完整 key pinning、offer confirmation 和授权目标。两阶段评审通过，可以停在人工闸门等待用户批准 Batch 5。

此结论**不是发布批准**：`snow` 正式审计、真实网络 driver timeout/close 语义、CHUNK/RESUME 授权接线、当前正式代码的 Linux↔Windows 跨机互操作和 macOS 真机矩阵仍是后续硬门。
