---
feature: rust-rewrite
stage: design
complexity: complex
generated_by: spec-dev
generated_at: 2026-07-26T09:22:01+08:00
version: 2
status: awaiting-approval
---

# 技术设计: Quick Share Rust 全量重写

## 1. 设计目标与原则

本设计将 Quick Share 从 Python HTTP 分享工具演进为 Rust 跨平台 CLI 局域网传输工具，同时保留安全的传统 Web 分享能力。

核心原则：

1. **单一二进制**：核心运行不依赖 Python、Node.js 或桌面 GUI。
2. **渐进替换**：Rust 实现先与 Python 版本并存，通过验收后再移除旧实现，避免一次性切换失去回滚能力。
3. **安全默认**：设备直传必须加密；未知设备只能提交待确认请求；任何明文 Web 服务都必须显式启用。
4. **流式与有界资源**：禁止完整缓存大文件；所有队列、请求体和并发均有硬上限和背压。
5. **协议与业务分离**：协议数据类型、文件安全、传输状态机、网络服务、Web 服务和平台适配分层。
6. **失败可恢复**：接收内容先进入目标文件系统内的 staging 区，完整性验证通过后再提交。
7. **可测试性优先**：核心状态机不依赖真实网络；平台和网络边界使用 trait 隔离。

## 2. 关键架构决策

### ADR-000：正式实施前先执行可行性 Spike

以下五项在 Planning 阶段只有候选方案，没有足够证据固定生产实现：

1. 设备身份协议：自签名 mTLS、TLS + 应用层签名、Noise XX；
2. mDNS 在三平台、多网卡、防火墙和 VPN 环境中的实际行为；
3. 传统 Web 自签名 HTTPS 在桌面与移动浏览器中的可接受性；
4. 分块恢复日志在随机崩溃和状态损坏下的一致性；
5. Linux X11/Wayland/headless 与单文件剪贴板能力的兼容矩阵。

Execution 的第一个批次必须只做隔离原型和实验报告，不创建正式产品 workspace。每项输出可复现命令、结果、限制和 ADR。任何 P0 Spike 未通过时触发 Go/No-Go 闸门，暂停正式重写，由用户选择修改需求、接受降级方案或终止项目。

Spike 代码放在 `spikes/`，默认视为可丢弃实验，不得直接复制到生产 crate；只有经过后续 TDD 和安全评审的设计才能进入正式实现。

### ADR-001：自有协议，不承诺 LocalSend 互操作

本项目借鉴 LocalSend 的体验，但首版使用独立的 Quick Share Protocol v1（QSP/1），不声称兼容 LocalSend 客户端。

原因：

- 需求包含 CLI 自动 Web 回退、断点续传、符号链接和自有可信设备模型；
- 直接绑定第三方协议会限制恢复状态、错误模型和安全策略；
- 后续可通过独立兼容层支持 LocalSend，而不污染核心领域模型。

### ADR-002（暂定）：mDNS/DNS-SD 发现 + 加密直传

- 发现层候选为 mDNS/DNS-SD，服务类型 `_quickshare._tcp.local.`；
- mDNS 的“零响应”无法区分确实无设备和组播被静默丢弃，这是明确的产品限制；
- 如果 SP-002 证明三平台可接受，则保留 mDNS，并提供 `--peer` 作为确定性降级路径；
- SP-001 建议设备直传采用 Noise XX；QSP 控制消息和固定大小数据块使用有界 Noise frame；
- JSON 可用于低频控制消息，数据块使用明确 binary frame，所有 frame 有长度和类型上限；
- HTTP/2 不再作为设备直传默认承载，但传统 Web 继续使用 HTTPS/HTTP；
- 如果 Batch 0 不批准 ADR-009，本文 QSP transport 必须回到设计阶段，不得边实施边决定。

Noise XX 更贴合无预共享信任的首次配对，且双机原型已验证 SAS、pinning 和 transport。代价是需要自行实现严格有界、可取消、可恢复的 framing，因此 QSP 状态机和 fuzz 测试仍是 P0。

### ADR-003（目标属性，机制待定）：首次配对与可信设备

无论 SP-001 最终选择 mTLS、应用层签名还是 Noise XX，都必须满足：

- 每个设备具有持久化身份密钥；
- 首次连接双方显示由握手 transcript 派生的短验证码（SAS）；
- “接受并信任”必须要求明确验证 SAS 或输入一次性配对码；
- 未知设备只能提交有界 offer，不得在确认前上传数据；
- 可信设备固定身份公钥指纹，变化时不自动更新信任；
- “接受一次”只授权当前 transfer；
- 重放、证书/公钥替换和中间人测试必须失败关闭。

如果用户不比较 SAS、没有预共享密钥、没有公共 CA 且没有中心账户，首次连接只能达到 TOFU，不能宣称强身份认证。最终 UI 和安全声明必须如实表达这一限制。

任何自定义 verifier、canonical request 签名或 Noise framing 都属于高风险实现区，必须集中封装并独立安全评审。

### ADR-004：分块传输 + 接收端日志实现恢复

- 默认数据块大小 4 MiB，协议允许在受限范围内协商；
- 每个块携带序号、长度和 BLAKE3 摘要；
- 接收端只在块摘要正确后记录为已完成；
- 恢复时由接收端返回缺失块集合，发送端只补传缺失块；
- 完成后接收端重新流式计算整个文件 BLAKE3，并与发送端最终摘要比对；
- resume 权限绑定原发送设备身份和 transfer ID。

该方案允许并发和乱序块，同时避免为了创建 offer 预读整个大文件。代价是完成阶段会额外读取一次目标文件，属于可接受的完整性成本，后续可通过有序流优化。

### ADR-005：文件系统日志，不引入数据库

首版不引入 SQLite。状态按职责分开：

- `config.toml`：用户偏好；
- `identity/`：获选身份协议的设备私钥、公钥及必要证书；
- `trusted-devices.toml`：可信设备；
- `state/transfers/<id>.json`：可恢复传输日志；
- 目标目录内隐藏 staging 目录：接收中的数据块与临时文件。

所有状态文件采用“同目录临时文件 + fsync + 原子 rename”更新。写入由进程内锁串行化；未来出现多进程共享需求时再评估数据库。

### ADR-006：Cargo workspace 分层，但只发布一个主程序

workspace 用于信息隐藏、独立测试和限制依赖扩散，最终只发布 `quick-share` 一个可执行文件。`sc`、`rc` 是同一二进制的链接或副本，程序通过 argv[0] 选择快捷入口。

### ADR-007：Rust 与 Python 侧向迁移

实施期间保留现有 `src/`、`tests/`、`setup.py` 等 Python 实现。Rust 达到功能、安全和三平台验收后，在单独切换任务中：

1. 将 Rust 二进制设为唯一发布产物；
2. 更新安装脚本和 README；
3. 归档迁移说明；
4. 删除 Python 运行时文件；
5. 按项目版本清单同步版本与 CHANGELOG，或在删除旧版本文件后将 Cargo 元数据设为唯一版本源。

### ADR-008：传统 Web 默认自签名 HTTPS，并允许用户证书

SP-003 已在 Linux 双机和 Windows 浏览器完成验证。用户选择安全策略 D：

- 传统 Web 默认使用自签名 HTTPS；
- 终端必须明确说明浏览器会显示不受信任警告；
- 用户可以配置自己的受信任证书和私钥；
- HTTP 仍只能通过显式 `--allow-http` 或等价显式配置启用；
- HTTP 模式必须显示醒目风险提示；
- 自动回退不得静默选择 HTTP；
- 无论 TLS 信任方式如何，分享 URL 继续使用高熵 token、有效期和次数限制。

该选择接受“首次浏览器访问有证书警告”的体验成本，以保持默认链路加密。测试证书不得被建议安装为系统 CA。

### ADR-009（等待 Batch 0 闸门批准）：设备直传采用 Noise XX

SP-001 的 Linux↔Linux、Windows↔Linux 原生双机验证表明，`Noise_XX_25519_ChaChaPoly_BLAKE2s` 能直接满足双方首次未知 static key、SAS、持久 pinning 和加密 transport 的目标。

建议选择：

- 设备直传安全层使用 Noise XX；
- 每台设备持久化 X25519 static key；
- 首次配对显示 handshake hash 派生的 6 位 SAS；
- 长期信任绑定完整 static public key，不绑定 6 位 SAS；
- QSP 控制和数据消息通过有界 Noise frame 承载；
- Web 模式继续使用 rustls HTTPS，不与 Noise 混用；
- mDNS 只发布不可信的发现提示和 identity fingerprint，最终身份由 Noise 握手确认。

约束：`snow 0.10.0` README 明确说明没有正式安全审计。正式发布前必须进行专项安全评审、RustSec 审计、framing/replay/fuzz 测试；Spike 代码不得复制为生产实现。如果安全评审不通过，回退到重新评估 rustls + 应用签名，而不是降低认证要求。

## 3. 总体架构

以下为目标分层，不代表安全承载已经定案。图中的 discovery/transfer 接口保持稳定；TLS/HTTP2、应用签名或 Noise 的具体 adapter 由 Batch 0 ADR 决定。

```text
┌──────────────────────── quick-share-cli ────────────────────────┐
│ clap 命令、argv[0] 快捷入口、自动模式编排、终端交互、退出码      │
└───────┬──────────────┬──────────────┬──────────────┬────────────┘
        │              │              │              │
        ▼              ▼              ▼              ▼
┌──────────────┐ ┌──────────────┐ ┌─────────────┐ ┌──────────────┐
│ qs-discovery │ │ qs-transfer  │ │   qs-web    │ │ qs-platform  │
│ mDNS/DNS-SD  │ │ sender/recv  │ │ Axum/Web UI │ │ dirs/clip/fs │
└──────┬───────┘ └──────┬───────┘ └──────┬──────┘ └──────┬───────┘
       │                │                │               │
       └──────────┬─────┴──────────┬─────┴───────────────┘
                  ▼                ▼
           ┌──────────────┐ ┌──────────────┐
           │ qs-protocol  │ │   qs-core    │
           │ wire types   │ │ manifest/fs  │
           │ versioning   │ │ state/trust  │
           └──────────────┘ └──────────────┘
```

建议目录：

```text
Cargo.toml
Cargo.lock
rust-toolchain.toml
crates/
├── quick-share-cli/       # 唯一发布的 binary crate
├── quick-share-core/      # 领域模型、配置、路径、manifest、状态机
├── quick-share-protocol/  # QSP/1 wire types、版本和错误码
├── quick-share-discovery/ # mDNS 广播与扫描
├── quick-share-transfer/  # 安全连接、offer、上传、恢复、重试
├── quick-share-web/       # 传统 Web 服务和嵌入式前端
├── quick-share-update/    # 固定源、签名校验、自更新和回滚
└── quick-share-platform/  # 平台目录、权限、剪贴板、信号和存储
crates/quick-share-web/assets/
├── index.html
├── app.js
└── app.css
tests/
├── fixtures/
├── protocol/
├── integration/
└── security/
```

## 4. 模块职责与接口

### 4.1 `quick-share-cli`

职责：

- 定义 `send`、`receive`、`serve`、`devices`、`config`、`update`；
- 根据 `argv[0]` 将 `sc` 映射为 `send`，将 `rc` 映射为 `receive`；
- 解析 CLI、环境变量和 TOML 配置；
- 编排自动发现与 Web 回退；
- 处理终端确认、设备选择、进度条和 Ctrl+C；
- 将领域错误映射为稳定退出码。

自动模式伪代码：

```rust
async fn auto_send(request: SendRequest) -> Result<ExitStatus> {
    let peers = discovery.scan(config.discovery.timeout).await?;
    if peers.is_empty() {
        return web.serve(request.into_catalog()).await;
    }

    let peer = terminal.select_peer(peers)?;
    // 从此处开始，拒绝/超时/失败均直接返回，不再回退 Web。
    transfer.send(peer, request).await
}
```

建议退出码：

| 退出码 | 含义 |
|---:|---|
| 0 | 成功或用户正常取消 |
| 2 | 参数或配置错误 |
| 3 | 强制指定的设备不可用 |
| 4 | 对方拒绝或确认超时 |
| 5 | 身份、证书或授权失败 |
| 6 | 本地文件系统错误 |
| 7 | 网络或发现错误 |
| 8 | 完整性校验失败 |
| 9 | 更新失败 |

### 4.2 `quick-share-core`

核心类型：

```rust
struct DeviceId(String);          // 获选身份方案公钥指纹的稳定编码
struct TransferId(Uuid);
struct EntryId(u32);

struct TransferManifest {
    transfer_id: TransferId,
    content_kind: ContentKind,
    entries: Vec<ManifestEntry>,
    total_bytes: u64,
    chunk_size: u32,
}

enum ManifestEntryKind {
    File,
    Directory,
    Symlink { target: String },
    Text { media_type: String },
}

struct ManifestEntry {
    id: EntryId,
    relative_path: RelativePath,
    kind: ManifestEntryKind,
    size: u64,
    source_snapshot: Option<SourceSnapshot>,
}
```

职责：

- 验证输入路径、重复顶层名称和源文件快照；
- 构造不含绝对接收路径的 manifest；
- 确保所有 `relative_path` 都是规范化相对路径；
- 检测 `..`、绝对路径、Windows 前缀、NUL、保留名称和符号链接逃逸；
- 定义冲突策略：`rename`（默认）、`ask`、`skip`、`overwrite`、`error`；
- 实现 transfer/entry/chunk 状态机；
- 原子持久化配置、信任列表和恢复日志。

状态机：

```text
Offered
  ├─ reject ───────────────→ Rejected
  ├─ timeout ──────────────→ Expired
  └─ accept ───────────────→ Accepted
Accepted ── first chunk ───→ Transferring
Transferring
  ├─ disconnect ───────────→ Paused
  ├─ cancel ───────────────→ Cancelled
  ├─ unrecoverable error ──→ Failed
  └─ all chunks received ──→ Verifying
Verifying
  ├─ digest mismatch ──────→ Failed
  └─ atomic commit ────────→ Completed
Paused ── authenticated resume ─→ Transferring
```

### 4.3 `quick-share-protocol`

ADR-009 下，QSP/1 使用 Noise 有界 frame，而不是设备直传 HTTP endpoint。下表中的方法/路径仅保留为命令语义名称，wire format 使用稳定 message type、request ID 和 payload length。协议类型必须添加 `deny_unknown_fields` 的位置需谨慎：安全敏感命令严格拒绝未知字段；能力扩展字段允许忽略。所有集合、字符串和请求体均有上限。

端点草案：

| 消息类型 | 语义名称 | 权限 | 用途 |
|---|---|---|---|
| `INFO_REQUEST/RESPONSE` | `info` | 任意完成安全握手的设备 | 版本和能力协商 |
| `OFFER_CREATE` | `offers.create` | 未知/可信设备 | 创建待确认发送请求 |
| `OFFER_STATUS` | `offers.status` | offer 所有者 | 查询接受/拒绝状态 |
| `TRANSFER_STATUS` | `transfers.status` | transfer 所有者 | 获取缺失块与恢复状态 |
| `CHUNK_DATA/ACK` | `transfers.chunk` | 已接受设备 | 上传一个数据块 |
| `TRANSFER_COMPLETE` | `transfers.complete` | transfer 所有者 | 提交最终摘要并触发校验 |
| `TRANSFER_CANCEL` | `transfers.cancel` | 任一相关方 | 取消任务 |

控制消息示例：

```json
{
  "protocolVersion": "1.0",
  "transferId": "0190...",
  "sender": {
    "deviceId": "qs_...",
    "name": "bluecake-linux"
  },
  "contentKind": "files",
  "chunkSize": 4194304,
  "totalBytes": 123456,
  "entries": [
    {
      "id": 1,
      "relativePath": "docs/readme.txt",
      "kind": "file",
      "size": 123456
    }
  ]
}
```

数据块请求必须携带：

- transfer、entry、chunk 标识；
- 精确偏移和长度；
- BLAKE3 块摘要；
- 幂等请求标识；
- 有界 `Content-Length`。

重复上传已验证的相同块返回幂等成功；同一块编号但摘要不同必须拒绝并记录安全事件。

协议版本采用 major/minor：major 不兼容时拒绝，minor 通过能力协商降级。错误响应使用稳定机器码和安全的人类说明，不返回本地绝对路径或内部堆栈。

### 4.4 `quick-share-discovery`

mDNS TXT 字段控制在 DNS-SD 限制内：

```text
id=<short-device-id>
name=<escaped-device-name>
ver=1
port=<u16>
fp=<identity-public-key-fingerprint>
caps=files,dirs,text,resume
```

安全规则：

- 广播内容全部视为不可信提示；
- 名称限制长度并移除控制字符；
- 发现结果只有在获选安全握手和 `/info` 能力协商后才成为可连接 peer；
- 按设备 ID、地址、端口去重；
- 默认忽略 loopback、未指定地址、明显虚拟接口，可配置包含；
- 禁止使用 `enable_addr_auto()` 直接发布所有地址；Linux/Windows Spike 已证明它会发布虚拟地址并遗漏主地址；
- 显式枚举可用 LAN 接口，并按获选 IP 注册服务；候选连接成功前不认定地址可达；
- 多网卡并行查询，扫描达到超时后返回当前结果；
- mDNS 不可用时提示 `--peer host:port`，自动模式不把“扫描错误”当作“扫描无结果”静默回退。

### 4.5 `quick-share-transfer`

接收服务由以下组件构成：

- `SecureIdentity`：加载或生成获选协议需要的长期身份材料；
- `PeerAuthenticator`：提取身份公钥指纹、执行 pinning 和 SAS；
- `OfferService`：创建、确认、拒绝和超时 offer；
- `TransferStore`：恢复日志、staging 和冲突策略；
- `ChunkReceiver`：流式接收、长度限制、摘要验证和落盘；
- `TransferSender`：并发块调度、重试、取消、进度事件；
- `RateAndResourceLimits`：连接数、并发任务、每设备 offer 速率、manifest 大小和块大小限制。

默认限制建议：

| 项目 | 默认值 | 说明 |
|---|---:|---|
| 发现超时 | 2 秒 | 可配置 |
| offer 确认超时 | 120 秒 | 到期自动拒绝 |
| 块大小 | 4 MiB | 协商范围 256 KiB–16 MiB |
| 并发文件流 | 4 | 根据磁盘性能可配置 |
| 同时接收任务 | 1 | 其余排队或拒绝 |
| 网络重试 | 3 次 | 指数退避并带抖动 |
| manifest | 8 MiB | 同时限制 entry 数量 |
| 设备名称 | 64 Unicode 标量 | 清理控制字符 |

staging 设计：

```text
<output>/.quick-share-staging/<transfer-id>/
├── manifest.json
├── state.json
└── files/
    └── <entry-id>.part
```

- staging 位于最终输出文件系统，保证最终 rename 不跨文件系统；
- `.part` 文件不得作为最终文件暴露；
- 每个块写入成功并同步必要元数据后，原子更新 `state.json`；
- 完整文件通过最终 BLAKE3 后按冲突策略提交；
- 目录和安全符号链接在提交阶段创建；
- 绝对或逃逸型符号链接默认不创建，要求额外确认或保存为带说明的安全降级文件。

### 4.6 `quick-share-web`

职责：

- 将明确选择的路径转换为只读 `ShareCatalog`；
- 提供嵌入式 HTML/CSS/JS 文件列表、预览、下载和上传；
- 提供单文件、目录浏览、多路径和 Download All；
- 提供下载/上传进度、次数限制、超时和会话计数；
- 生成二维码和 `curl`/`wget` 命令。

安全路由原则：

- URL 使用 catalog entry ID，不直接将用户路径拼入文件系统；
- 访问路径由预先构建的 catalog 或安全目录解析器映射；
- 每次访问再次确认规范化路径仍在分享根内；
- 禁止跟随未授权符号链接；
- 上传文件名只取安全 basename，并在目标目录内创建；
- 所有上传使用流式解析、总量限制、单文件限制和临时文件；
- CSRF/访问 token、CSP、`X-Content-Type-Options` 和安全缓存头默认启用；
- Web UI 不依赖 CDN，离线可用。

Web 传输采用 ADR-008：

1. 默认生成或复用局域网自签名证书，以 HTTPS 启动；
2. 用户提供的有效证书和私钥可配置；
3. 自签名浏览器警告必须在终端明确说明；
4. HTTP 只能由 `--allow-http` 或明确持久配置启用，并显示风险；
5. 分享 URL 包含至少 128 bit 随机访问 token，并受超时和次数限制；
6. 自动 Web 回退显示实际协议、监听接口和安全状态，不静默选择 HTTP。

目录打包在 blocking worker 中生成，通过有界 channel 流式传给 HTTP body；必须限制同时打包数，客户端断开后及时取消。

### 4.7 `quick-share-platform`

trait 边界：

```rust
trait PlatformDirs { /* config/data/cache/downloads */ }
trait Clipboard { fn read_text(...); fn write_text(...); }
trait FilePermissions { /* private key/config permissions */ }
trait AtomicReplace { /* Unix/Windows executable replacement */ }
trait SignalSource { /* Ctrl+C and graceful shutdown */ }
```

- 使用平台标准配置、数据和缓存目录；
- Linux 无图形会话、Wayland/X11 不可用时，剪贴板返回可识别错误并回退 stdout；
- Windows 符号链接权限不足时执行安全降级；管理员账户实测可创建，非管理员仍必须失败降级测试；
- Unix 私钥文件至少为 `0600`；Windows 使用受保护 ACL，只允许当前用户、SYSTEM 和必要管理员主体；不得依赖宽松目录继承；
- Windows receiver/Web 启动时检测网络 profile 和入站可达性；Public profile 无规则时显示明确诊断；
- 程序不得静默关闭 Windows Firewall；安装器仅在用户明确同意时添加限定程序、端口/协议和 profile 的最小规则；
- updater 替换当前可执行文件时必须考虑 Windows 文件锁。

## 5. 配置设计

建议配置：

```toml
[device]
name = "bluecake-linux"

[receive]
output = "~/Downloads"
trusted_policy = "auto"       # auto | confirm
conflict = "rename"           # rename | ask | skip | overwrite | error

[discovery]
timeout_ms = 2000
include_virtual = false

[network]
port = 0                       # 0 = 自动选择
bind = "lan"

[transfer]
chunk_size = 4194304
concurrent_files = 4
max_receive_tasks = 1
retry_count = 3

[web]
timeout = "5m"
max_downloads = 10
upload = false
allow_http = false
```

环境变量使用统一前缀和双下划线层级，例如：

```text
QUICK_SHARE__RECEIVE__OUTPUT
QUICK_SHARE__DISCOVERY__TIMEOUT_MS
QUICK_SHARE__WEB__MAX_DOWNLOADS
```

合并顺序：内置默认值 → TOML → 环境变量 → CLI。身份私钥不能通过普通配置打印或 `config --show` 输出。

旧版 `~/.quick-share/config.json` 仅迁移非敏感、语义明确的数据；旧共享 secret 不自动转换为新设备信任关系。

## 6. 主要数据流

### 6.1 设备直传

```text
Receiver                         Sender
   │ rc 启动                        │ sc paths...
   │ 安全服务 + mDNS 广播            │ 扫描 2s
   │                                │ 发现 receiver
   │←──── 获选协议的身份握手 ───────│
   │←──── POST /offers(manifest) ──│
   │ 显示设备、清单、SAS             │ 显示 SAS / 等待
   │ 接受一次或接受并信任             │
   │──── offer=accepted ───────────→│
   │←──── PUT chunk 0..N (并发) ───│
   │ 校验块摘要 + 更新恢复日志         │
   │←──── POST /complete ──────────│
   │ 全文件 BLAKE3 + 原子提交          │
   │──── completed ────────────────→│
```

### 6.2 断点续传

```text
Sender reconnects with same device identity + transfer ID
  → GET /transfers/{id}/status
  → receiver verifies owner and returns completed/missing chunks
  → sender revalidates source snapshot
  → sender sends missing chunks only
  → final verification and commit
```

### 6.3 自动 Web 回退

```text
sc paths...
  → mDNS scan returns success + zero peers
  → build immutable ShareCatalog
  → start web server using SP-003 approved security policy
  → print explicit protocol, mode, interfaces, expiry, limits, URL, QR, curl/wget
  → stop on timeout / quota / Ctrl+C
```

扫描本身发生错误时必须报告发现错误并提供 `--web` 建议；不能把网络或权限故障伪装成“没有设备”。

## 7. 并发、取消与关停

- Tokio 作为异步运行时；
- 网络连接、传输任务、打包 worker 和进度事件均挂接同一取消树；
- 第一次 Ctrl+C 触发 graceful shutdown：停止接收新请求、通知活动任务取消、持久化可恢复状态；
- 第二次 Ctrl+C 允许强制退出；
- 使用 semaphore 限制连接、offer、传输、文件流和 ZIP worker；
- blocking 文件哈希和压缩使用受限 blocking pool，不占用异步 executor；
- 进度更新通过有界 channel 聚合，终端刷新频率限制在约 10–20 Hz；
- 客户端断开必须取消对应生产者，避免后台继续读取或压缩整个目录。

## 8. 安全与威胁模型摘要

| 威胁 | 缓解措施 |
|---|---|
| 伪造 mDNS 设备 | 广播不作为信任；安全握手指纹、SAS 和长期 pinning |
| 首次连接 MITM | 双端显示 SAS；信任前明确比较；后续指纹固定 |
| 重放 offer/chunk | 随机 transfer ID、短期 nonce、状态机和幂等键 |
| 陌生设备直接上传 | 未确认设备仅允许 `/info` 和 `/offers` |
| 路径遍历 | 强类型相对路径、catalog ID、canonical containment |
| 符号链接逃逸 | 默认不跟随；提交时再次验证；危险链接确认/降级 |
| 恶意超大 manifest | 请求体、entry 数量、路径长度和总大小上限 |
| 磁盘耗尽 | 接受前检查空间；staging 配额；写入错误立即暂停 |
| 同名覆盖 | 默认 rename；overwrite 必须显式配置或确认 |
| 部分文件伪装完整 | `.part` staging + 摘要校验 + 同文件系统 rename |
| Web URL 猜测 | 128 bit 随机 token + 超时/次数限制 |
| 恶意上传文件名 | basename 清洗、保留名检测、目标 containment |
| 更新供应链攻击 | 固定仓库、校验和、签名验证、原子替换与回滚 |
| 日志泄密 | 结构化脱敏；不记录 key、token、完整文本内容 |

在实现获选身份协议、路径解析、上传和 updater 前后都必须执行专项安全评审。

## 9. 依赖候选与版本策略

候选库：

| 能力 | 候选 |
|---|---|
| CLI | `clap` |
| 异步运行时 | `tokio`, `tokio-util` |
| HTTP | `axum`, `hyper`, `tower-http` |
| TLS | `rustls`, `tokio-rustls`, `rcgen` |
| mDNS | `mdns-sd` |
| 序列化 | `serde`, `serde_json`, `toml` |
| 摘要 | `blake3` |
| ID | `uuid` |
| 配置目录 | `directories` |
| 剪贴板 | `arboard`，失败时平台降级 |
| 终端 | `indicatif`, `dialoguer` 或轻量自实现 |
| 二维码 | `qrcode` |
| 日志 | `tracing`, `tracing-subscriber` |
| 临时文件 | `tempfile` |
| Web 客户端/更新 | `reqwest`（rustls feature） |
| 打包发布 | `cargo-dist` 或等价 GitHub Actions |

策略：

- Rust edition 2024；初始 MSRV 设为 1.92.0，并在 CI 固定验证；
- 依赖必须关闭不需要的默认 feature，优先纯 Rust TLS；
- `Cargo.lock` 纳入版本控制；
- 禁止 wildcard 版本；
- 在引入每个候选库前检查许可证、维护状态、MSRV、跨平台依赖和安全公告；
- TLS、更新和路径处理依赖变更必须单独审查。

调研依据：

- LocalSend 官方说明：https://localsend.org/
- LocalSend 协议参考：https://github.com/localsend/protocol
- Axum：https://docs.rs/axum/latest/axum/
- rustls：https://docs.rs/rustls/latest/rustls/
- rcgen：https://docs.rs/rcgen/latest/rcgen/
- mdns-sd：https://docs.rs/mdns-sd/latest/mdns_sd/
- BLAKE3：https://docs.rs/blake3/latest/blake3/
- directories：https://docs.rs/directories/latest/directories/struct.ProjectDirs.html
- arboard：https://docs.rs/arboard/latest/arboard/
- self_update：https://docs.rs/self_update/latest/self_update/

上述链接仅作为设计调研入口；实施时以锁定版本的 API 和安全公告为准。

## 10. 测试策略

### 10.1 单元测试

- CLI alias 和参数优先级；
- manifest 构造、路径规范化、冲突命名；
- transfer 状态机的合法与非法转换；
- chunk offset、长度、摘要和幂等行为；
- trusted device pinning 和证书变化；
- 配置原子写入与损坏恢复；
- Web catalog 路由和 token 校验。

### 10.2 属性测试与模糊测试

- 任意 URL 编码、Windows/Unix 路径和 Unicode 文件名不能逃出根目录；
- 任意 protocol JSON 不得导致 panic 或无界分配；
- multipart/upload 解析不得越界或接受目录穿越；
- transfer 状态机在随机事件序列下保持不变量；
- chunk 重排、重复、截断和摘要冲突保持幂等与一致性。

### 10.3 集成测试

- 同机不同端口模拟 sender/receiver；
- 未知设备接受一次、接受并信任、拒绝和超时；
- 可信设备自动接收和证书变化；
- 连接中断后恢复缺失块；
- 源文件中途变化；
- 空文件、目录、超大稀疏文件、Unicode、符号链接；
- 零 peer 自动 Web 回退，发现失败不回退；
- Web 下载、上传、次数限制、超时和客户端断开；
- updater 成功、校验失败和替换失败回滚。

### 10.4 跨平台与性能

CI 构建目标至少包括：

- Linux x86_64（必要时增加 musl 产物）；
- macOS x86_64、aarch64；
- Windows x86_64；
- 可行时增加 Linux/Windows aarch64 构建。

性能基准固定记录：

- 1 GiB、10 GiB 单文件；
- 10,000 个小文件；
- 1/2.5/10 Gbps loopback 或受控网络；
- 峰值 RSS、CPU、吞吐和恢复耗时；
- 并发 1、2、4、8 文件流。

验收关注“不人为限速”和资源有界，而不是对所有硬件承诺固定绝对速度。

## 11. 发布与迁移

### 11.1 发布产物

建议命名：

```text
quick-share-<version>-x86_64-unknown-linux-musl.tar.gz
quick-share-<version>-aarch64-apple-darwin.tar.gz
quick-share-<version>-x86_64-apple-darwin.tar.gz
quick-share-<version>-x86_64-pc-windows-msvc.zip
SHA256SUMS
SHA256SUMS.sig
```

`sc`、`rc` 由安装脚本创建链接或副本，不单独编译不同程序。

### 11.2 切换标准

只有同时满足以下条件才移除 Python 实现：

- Rust 核心流程和 Web 传统流程通过验收；
- 三平台 CI 和安装冒烟测试通过；
- 安全审查无未处理 P0/P1；
- 性能基准无明显退化；
- 更新和回滚路径在真实产物上验证；
- README、CHANGELOG、安装脚本和版本源完成切换。

建议 Rust 首个不兼容版本发布为 `2.0.0`，最终版本号仍需在发布流程中由维护者确认。

## 12. 设计风险与实施前闸门

| 风险 | 等级 | 处理 |
|---|---|---|
| 身份协议选择或 verifier 实现错误 | 高 | SP-001 对比 mTLS/应用签名/Noise；Go/No-Go；独立安全评审 |
| 浏览器自签名 HTTPS 体验差 | 高 | SP-003 桌面/移动实测后确定 HTTPS、HTTP 或本地 CA 产品策略 |
| mDNS 在企业网络/防火墙不可用 | 中 | `--peer` 强制地址；错误不伪装成零结果 |
| 多平台剪贴板依赖图形环境 | 中 | trait 隔离；stdout/file 降级 |
| 断点续传状态损坏 | 高 | 原子日志、幂等块、恢复属性测试 |
| 目录和符号链接跨平台语义不同 | 高 | manifest 明确类型；危险目标确认或安全降级 |
| 全量重写周期过长 | 高 | 侧向迁移、分阶段可运行里程碑、最后删除 Python |
| 单文件与 Linux GUI/clipboard 动态依赖冲突 | 中 | 核心无 GUI 依赖；剪贴板 feature 审计；musl 冒烟测试 |

Planning 闸门批准前不得创建 Cargo workspace、Spike 原型或产品代码。Planning 获批后只进入 Batch 0 可行性验证；Batch 0 的 Go/No-Go 闸门批准前不得创建正式产品 workspace 或执行 T-001 及后续任务。
