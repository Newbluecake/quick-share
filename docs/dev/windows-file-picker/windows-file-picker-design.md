---
feature: windows-file-picker
stage: design
complexity: complex
generated_by: spec-dev
generated_at: 2026-07-27T09:54:11Z
version: 1
status: approved
---

# 技术设计: Windows 远程文件与文件夹选择

## 1. 设计摘要

本功能在现有 Quick Share 2 Rust 架构上增加一个 Windows 桌面代理模式和一组 QSP/1.1 控制消息：

- Windows 运行 `quick-share agent`，作为可驻留托盘的双向传输端；
- Linux 执行 `quick-share receive --request [--peer ...]` 时，先启动本地接收器，再通过加密连接请求 Windows 选择文件或文件夹；
- Windows 选择完成后，使用第二条 Noise XX 连接回连 Linux，并复用现有 `TransferSender`、`TransferOffer`、分块、恢复和完整性校验链路；
- Linux 向 Windows 发送时仍使用现有 `quick-share send ...`，Windows agent 在收到 offer 后弹出保存目录确认窗口；
- 接收目录以稳定 `DeviceId` 为键持久化；目标目录和冲突决策只保存在接收端，不上协议、不写远端日志；
- 通过目标规划表支持覆盖、跳过、重命名和“应用到全部”，并保持 staging、原子提交与路径安全不变量。

不采用“在同一 Noise 会话中动态反转发送/接收角色”的方案。回连方案可以复用已经验收的 sender/receiver 状态机，减少对高风险数据面和恢复协议的改动。

## 2. 现状与关键缺口

当前实现已经具备：

- Noise XX 身份握手、SAS、完整静态密钥 pinning；
- mDNS 发现和显式 `--peer`；
- `OfferManager`、`ReceiverService`、`TransferSender`；
- 文件、目录、符号链接、分块、重试、取消和恢复；
- 固定接收根目录与全局 `ConflictPolicy`；
- Windows 平台目录、ACL 和网络诊断。

当前不具备：

1. 接收端主动请求远端选择源内容的协议消息；
2. 一个进程同时作为接收器和按请求启动发送器的 agent 编排；
3. 每个 transfer 独立接收根目录；
4. 每设备保存目录偏好；
5. 每个冲突项的独立决策；
6. Windows 原生选择器、托盘和 UI 线程代理。

现有 `ReceiverService` 在构造时固定 `output_root`，现有 `OfferManager` 对可信设备可能在 prompt 之前自动接受。因此 Windows agent 不能直接套用当前 `run_receive`，需要引入 transfer 目标路由，并在 agent 模式中强制“先完成本地目录/冲突交互，再授权数据传输”。

## 3. 架构决策

### ADR-001：新增显式 `agent` 模式，不改变终端 receiver 默认行为

新增：

```text
quick-share agent [--port PORT] [--bind LAN_OR_ADDRESS]
```

第一阶段仅在 Windows 上可用。它负责：

- 常驻监听并发布 mDNS；
- 接收入站 transfer offer，弹出保存目录交互；
- 接收远端源选择请求，弹出文件/文件夹选择器，再回连发送；
- 维护托盘、UI 串行化、退出和后台提示。

现有 `quick-share receive` 继续保持终端语义，避免 Windows GUI 依赖改变 Linux/macOS 构建和自动化脚本行为。非 Windows 平台执行 `agent` 时返回稳定、清晰的“不支持”错误；Linux agent/原生选择器后续可在相同 trait 和协议上实现。

agent 不自动安装为开机启动项，也不注册 Windows Service。用户必须在交互式桌面会话中启动它。

### ADR-002：Linux 使用显式远程请求模式

新增：

```text
quick-share receive --request
quick-share receive --request --peer <DEVICE_ID|HOST:PORT>
```

行为：

- `--request` 将 receive 切换为一次性远程选择模式；
- 未指定 `--peer` 时扫描支持 QSP/1.1 远程选择的 agent，并在交互式终端中选择；
- 指定 `--peer` 时只连接目标设备；
- Linux 先绑定并启动本地 receiver，再发送选择请求；
- `--output` 继续决定 Linux 保存目录；
- 远程选择模式隐含 one-shot，不能与持久被动接收语义混用；
- 非交互环境必须显式提供 `--peer`，不能猜测目标设备。

现有不带 `--request` 的 receive 行为完全不变。

### ADR-003：远程选择采用“两条认证连接 + 同身份回连”

流程：

```text
Linux requester                         Windows agent
      │ 启动临时 receiver                     │ 常驻 agent receiver
      │ callback port ready                    │
      │── Noise XX + INFO ───────────────────→│
      │── SOURCE_SELECTION_REQUEST ──────────→│
      │                                       │ 身份分类/授权/选择源
      │                                       │ 构建 manifest 和 TransferPlan
      │←─ SOURCE_SELECTION_RESPONSE(ready) ──│
      │                                       │
      │←════ 第二条 Noise XX 连接（回连）══════│
      │←─ OFFER_CREATE(initiatedBy=request) ─│
      │── 自动 AcceptOnce（精确预授权匹配）──→│
      │←─ CHUNK_DATA / COMPLETE ─────────────│
      │ 完整性校验、原子提交                    │
```

回连地址规则：

- 请求只携带非零 callback port；
- callback IP 必须取第一条认证连接的对端源 IP，不接受请求方提供任意目标 IP；
- Windows 回连时固定第一条握手获得的 Linux 完整静态公钥；
- Linux 只接受来自同一 Windows `DeviceId`、同一完整静态公钥、同一 request ID 和 response 中 transfer ID 的 callback offer；
- 预授权有短有效期且只能消费一次。

这避免反射攻击，也避免重新设计双工复用、角色反转和同会话并发响应。

### ADR-004：QSP/1.1 通过动态 INFO 响应保持 QSP/1.0 兼容

新增协议版本 `ProtocolVersion::V1_1`，新增能力 `Capability::RemoteSelection`，新增消息类型：

| 编号 | 消息 | 方向 | 用途 |
|---:|---|---|---|
| 11 | `SourceSelectionRequest` | requester → agent | 请求对端选择源文件/文件夹 |
| 12 | `SourceSelectionResponse` | agent → requester | 返回 ready/cancelled/rejected/busy/unavailable/failed |

兼容策略：

1. mDNS TXT 继续只发布 QSP/1.0 已知 capability，避免旧客户端因严格 capability parser 丢弃 agent；
2. 新客户端发送 INFO 时只包含基础 capability；
3. agent 根据对端 INFO 版本动态构造响应：
   - 对端为 1.0：响应版本降级为 1.0，且不发送 `remoteSelection`；
   - 对端为 1.1+：响应 1.1，并包含 `remoteSelection`；
4. 客户端只有在 INFO 响应同时满足 minor >= 1 且包含 `remoteSelection` 时，才发送消息 11；
5. 普通 `send`/`receive` 的现有消息序列和 JSON 在 1.0 协商下保持字节结构兼容。

`TransferOffer` 增加可选字段：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub initiated_by: Option<RequestId>;
```

普通发送为 `None` 且不序列化；远程选择 callback 必须为第一条请求的 request ID。

### ADR-005：远程选择请求必须在传输前完成身份授权

`SourceSelectionRequest` 建议结构：

```rust
pub struct SourceSelectionRequest {
    pub requester: DeviceInfo,
    pub callback_port: u16,
}

pub enum SourceSelectionStatus {
    Ready { transfer_id: TransferId },
    Cancelled,
    Rejected,
    Busy,
    UiUnavailable,
    Expired,
    Failed,
}
```

验证与状态规则：

- `requester.device_id` 必须匹配 Noise 握手静态公钥派生 ID；
- display name 继续视为不可信 UI 文本并使用现有长度/控制字符限制；
- callback port 必须非零；
- 每设备请求速率、全局 pending 数和 replay cache 有硬上限；
- 同一 `RequestId` 重试返回缓存终态，不重复弹窗、不重复启动 transfer；
- `Changed` 身份默认失败关闭，不允许按旧信任直接操作；
- Unknown 设备先显示 SAS 和“接受一次 / 接受并信任 / 拒绝”；
- Trusted 设备直接进入文件/文件夹选择窗口；
- 接受一次只为该 request 建立短期回连授权，不写入永久 trust store；
- “接受并信任”必须显式确认 SAS，复用现有 `TrustedDeviceStore`。

源选择与 manifest 构建完成后才返回 `Ready`。构建工作放入受限 blocking task，UI 线程不执行文件遍历和哈希。请求处理使用有界但足以覆盖人工选择和大文件 manifest 构建的超时；超时后不得在后台继续启动传输。

### ADR-006：每设备目录偏好与活动 transfer 绑定分开持久化

新增两个原子存储：

```text
<data-dir>/receive-destinations.toml
<data-dir>/receive-bindings.json
```

`ReceiveDestinationStore`：

```rust
struct DeviceDestination {
    device_id: DeviceId,
    path: PathBuf,
}
```

- 只为当前受信任设备保存；
- 以稳定 `DeviceId` 为键，不以设备名称/IP 为键；
- 应用重启后恢复；
- 设备名称变化不影响关联；
- key mismatch 不读取原目录；
- Unknown 的 AcceptOnce 不持久化路径；AcceptAndTrust 成功后才可保存；
- 无记录时使用 `AppDirs::download_dir()`。

`ReceiveBindingStore`：

```rust
struct ReceiveBinding {
    transfer_id: TransferId,
    sender_device_id: DeviceId,
    manifest_digest: [u8; 32],
    output_root: PathBuf,
    destination_plan: DestinationPlan,
}
```

- 在授予 upload authorization 前原子写入；
- 将活动 transfer 固定到原目标文件系统，确保 staging 和 resume 不跨目录；
- resume 必须匹配 sender、manifest digest 和原 output root；
- 原 root 不可访问时明确失败，不静默迁移已有 staging；
- transfer 完成并 cleanup 后删除绑定；暂停/断线时保留。

两个文件只包含本地路径和非秘密标识，不包含私钥、authorization proof 或传输内容。写入使用 `quick-share-platform::atomic_write`。

### ADR-007：以持久化 `DestinationPlan` 取代 agent 模式的单一冲突策略

现有 `ReceiverPolicy.conflict` 是 transfer 级单一策略，无法表达逐项选择。新增：

```rust
enum DestinationDisposition {
    Commit {
        relative_path: RelativePath,
        replace_existing: bool,
    },
    Skip,
}

struct DestinationPlan {
    entries: BTreeMap<EntryId, DestinationDisposition>,
}
```

规划步骤：

1. 在用户选定 output root 后，对完整 manifest 做路径和冲突预检；
2. 无冲突项默认映射原相对路径且 `replace_existing=false`；
3. 文件冲突支持 overwrite/skip/rename；
4. 目录冲突语义：
   - overwrite：合并已有目录，子项仍按各自决策；
   - skip：跳过该目录及全部后代；
   - rename：重命名目录根并同步重写所有后代相对路径；
5. “应用到全部”只作用于本次 transfer 后续同类冲突；
6. 计划在授权前完成并与 transfer binding 一起持久化；
7. commit 只能写入计划中的规范化相对路径。

TOCTOU 处理：

- 计划为 no-clobber 的目标如果在最终 commit 前被其他进程占用，commit 不自动改为覆盖；
- receiver 返回本地 `ConflictPending`，agent 再次唤起冲突交互并原子更新计划；
- server 在更新计划后重试 complete；
- sender 的 complete 等待使用专用人工交互超时；
- 路径祖先在规划和提交时都重新检查，符号链接祖先始终失败关闭。

现有终端 receiver 和 Web upload 继续使用原 `ConflictPolicy`；新的 planned commit API 与旧 API 并存，防止无关路径回归。

### ADR-008：Windows UI 使用 `rfd`，托盘/owner event loop 使用 `tray-icon-win` + `winit`

Windows 目标依赖候选：

- `rfd`：原生文件、文件夹和 message dialog；支持 `pick_files`、`pick_folder`、起始目录和 parent window；
- `tray-icon-win`：Windows-only 托盘图标和菜单；T-001 已排除跨平台 `tray-icon`，因为其 lockfile 引入旧 GTK3 审计 warning；
- `winit`：Win32 event loop、隐藏 owner window、跨线程 `EventLoopProxy` 和 request-user-attention；
- `embed-resource` 或等价构建方案：嵌入 Common Controls v6 manifest，使 rfd Windows TaskDialog 支持自定义按钮文本和现代样式；GNU `.rc` 必须使用数值资源类型 `24`，不能依赖未定义的 `RT_MANIFEST` token。

所有依赖仅在 `cfg(windows)` 下启用。项目自身仍保持 `#![forbid(unsafe_code)]`；依赖版本、许可证、MSRV、Windows MSVC/GNU 构建和 RustSec 必须在首个执行任务中验证。

`quick-share-platform` 新增无协议依赖的桌面接口：

```rust
trait DesktopInteraction: Send + Sync {
    fn authorize_peer(&self, request: AuthorizationDialog) -> Result<AuthorizationChoice, DesktopError>;
    fn choose_send_source(&self, request: SourceDialog) -> Result<SourceChoice, DesktopError>;
    fn confirm_receive_directory(&self, request: ReceiveDirectoryDialog) -> Result<DirectoryChoice, DesktopError>;
    fn resolve_conflict(&self, request: ConflictDialog) -> Result<ConflictChoice, DesktopError>;
    fn notify(&self, notification: DesktopNotification) -> Result<(), DesktopError>;
}
```

`rfd` message dialog最多承载三项自定义按钮，因此冲突交互采用两步原生小窗口：

1. “覆盖 / 跳过 / 重命名”（关闭窗口即取消 transfer）；
2. “仅此项 / 应用到全部 / 取消 transfer”。

文件选择入口使用“选择文件 / 选择文件夹 / 取消”；文件选择支持多选，文件夹选择返回单个根目录。保存目录窗口使用“接收 / 更改目录 / 取消”。

相关调研入口：

- rfd crate 与 Windows 自定义 TaskDialog：https://docs.rs/rfd
- rfd `FileDialog`：https://docs.rs/rfd/latest/rfd/struct.FileDialog.html
- rfd `MessageDialog`：https://docs.rs/rfd/latest/rfd/struct.MessageDialog.html
- tray-icon-win 平台 event loop 约束：https://docs.rs/tray-icon-win/latest/tray_icon_win/
- winit event loop：https://docs.rs/winit/latest/winit/event_loop/

### ADR-009：主线程运行 Windows event loop，Tokio 网络运行时放入后台线程

当前 binary 使用 `#[tokio::main]`。为满足 Windows tray/event loop 和 owner window 生命周期，入口改为同步 main：

```text
main thread (Windows agent only)
  ├─ winit event loop
  ├─ hidden owner window + tray icon
  └─ rfd blocking dialogs (严格串行)

network thread
  ├─ Tokio runtime
  ├─ Noise listener / discovery
  ├─ selection manager / callback sender
  └─ 通过 bounded channel + oneshot 请求 UI
```

非 agent 命令仍在当前线程创建 Tokio runtime 并执行原 `app::run`，保持命令行为。

UI broker 规则：

- 同时最多一个模态工作流；
- 新请求返回 Busy 或进入有界队列，绝不叠加选择器；
- 网络线程通过 `EventLoopProxy` 唤醒 UI，不直接调用 rfd；
- 每个 UI 请求有 request ID、deadline 和 oneshot response；
- deadline 到达或 agent 退出时关闭逻辑请求并返回 Expired/UiUnavailable；
- 对话框设置 hidden owner window 为 parent；无法抢占前台时显示 owner、请求用户注意并保持托盘提示；
- 不尝试穿透锁屏或 Session 0。agent 初始化无法建立交互 event loop 时直接失败；锁屏期间未响应的请求按确认超时结束。

托盘菜单第一阶段只要求：状态、显示/唤起、退出。不开启开机自启、Shell 扩展或系统服务。

## 4. 模块设计

### 4.1 `quick-share-protocol`

修改：

- `ProtocolVersion::V1_1`；
- `Capability::RemoteSelection`；
- `MessageType` 11/12；
- `SourceSelectionRequest/Response/Status`；
- `TransferOffer.initiated_by`；
- 新增长度、callback port、状态字段组合验证；
- 为 QSP/1.0 fixtures 增加序列化兼容断言。

### 4.2 `quick-share-core`

新增建议模块：

- `receive_preferences.rs`：`ReceiveDestinationStore`；
- `receive_binding.rs`：活动 transfer → output root / plan 绑定；
- `destination_plan.rs`：manifest 冲突扫描、目录前缀重写和计划验证。

不把本地绝对路径加入 wire model。

### 4.3 `quick-share-platform`

新增建议模块：

- `desktop.rs`：平台无关 DTO、trait、错误；
- `desktop/windows.rs`：rfd 实现；
- `tray/windows.rs`：tray/owner window/event proxy runtime。

非 Windows 提供明确 Unsupported stub，保证 workspace 在 Linux/macOS 编译。

### 4.4 `quick-share-transfer`

新增建议模块：

- `selection.rs`：请求状态机、速率限制、replay/幂等、短期 callback 授权；
- `receiver_router.rs`：按 transfer ID 路由到独立目标 receiver；
- `expected_offer.rs`：requester 对 callback offer 的一次性精确预授权。

修改：

- `direct.rs`：动态 INFO 响应和消息 11/12 dispatch；
- `offer.rs`：OfferView 暴露受限的 `initiated_by`；
- `receiver.rs` / `TransferStore`：planned commit、冲突待决和绑定恢复；
- 保持 chunk wire、authorization proof 和 data frame 不变。

### 4.5 `quick-share-cli`

新增建议模块：

- `agent.rs`：Windows agent 生命周期和双向编排；
- `remote_receive.rs`：Linux `receive --request` 编排；
- `desktop_prompt.rs`：OfferView/selection/conflict 与平台 DTO 映射。

修改：

- CLI intent：`AgentIntent`、receive `request` 和 `peer`；
- `main.rs`：同步入口 + Windows agent event loop 特例；
- `app.rs`：复用 `prepare_path_transfer` 和 `ProductionDirect`，将其拆成可供 agent callback 调用的服务；
- agent 的 incoming offer policy 始终要求本地目录工作流，不使用 trusted auto-accept；
- 普通 terminal receiver 继续遵守 `receive.trusted_policy`。

## 5. 主要交互流程

### 5.1 Linux 请求 Windows 发送

1. Linux 解析 `receive --request`，绑定 receiver，得到 callback port；
2. 发现/连接 Windows agent，完成 Noise 和 INFO 1.1 协商；
3. Unknown Windows 时 Linux 继续使用现有 TOFU/SAS 确认；
4. Linux 创建一次性 expected callback，发送 selection request；
5. Windows 对 requester 做完整静态身份分类：
   - Trusted：直接显示源选择小窗口；
   - Unknown：授权后显示源选择；
   - Changed：拒绝；
6. Windows 用户选择多个文件或一个文件夹；
7. agent 在 blocking worker 中构建 offer/plan，返回 `Ready { transfer_id }`；
8. agent 回连第一条连接的源 IP + callback port，并固定 Linux 静态公钥；
9. Linux 只对精确匹配 expected callback 的 offer 自动 AcceptOnce；
10. 复用现有 sender/receiver 完成传输；取消属于成功的正常终止状态。

### 5.2 Linux 发送到 Windows

1. Linux 继续使用现有 `send paths... --peer ...`；
2. Windows agent 收到 authenticated offer；
3. Trusted sender 直接显示保存目录确认；Unknown 先授权；Changed 拒绝；
4. 从 `ReceiveDestinationStore` 读取该 trusted `DeviceId` 的目录，无记录使用 Downloads；
5. 用户确认或更改目录；
6. core 验证目录存在、可写、无不安全条件，并生成 conflict plan；
7. binding 原子持久化成功后，才调用 `OfferManager::decide(AcceptOnce/AcceptAndTrust)`；
8. 数据进入选定文件系统内 staging；
9. final commit 按 plan 执行；竞态冲突重新询问；
10. 完成后清理 staging 和 active binding，保留每设备目录偏好。

## 6. 错误与取消模型

| 本地结果 | 远端稳定结果 | 进程语义 |
|---|---|---|
| 用户关闭/取消选择 | `Cancelled` | 正常取消，exit 0 |
| 用户明确拒绝未知设备 | `Rejected` | 远端 rejected，exit 4 |
| 身份变化/回连 key 不匹配 | protocol unauthorized / identity | exit 5 |
| agent UI 已占用 | `Busy` | 可重试，不弹第二窗口 |
| 无交互桌面/UI broker 不可用 | `UiUnavailable` | 清晰错误，不开始传输 |
| 请求确认超时 | `Expired` | 不在后台继续选择/发送 |
| 源路径失效或不可读 | `Failed` | 两端显示源准备失败 |
| 目标目录不可写/失效 | reject before authorization | 不授予 upload token |
| 冲突处理取消 | transfer cancel | 清理未完成临时文件，不提交剩余项 |
| callback 连接失败 | peer unavailable | Windows 显示失败，Linux 等待有界超时 |

日志只记录 request/transfer ID、设备 ID、状态和脱敏错误。默认不记录完整本地路径列表。

## 7. 安全不变量

1. 所有选择请求和 callback 都运行在 Noise XX 认证加密连接内；
2. mDNS 名称、IP、显示名称不能建立信任；
3. callback IP 只能是认证请求连接的源 IP，避免任意反射；
4. callback 必须固定第一条连接的完整静态公钥；
5. expected callback 绑定 request ID、transfer ID、DeviceId、公钥和过期时间，且只消费一次；
6. 未完成目录选择和 binding 持久化前不签发 upload authorization；
7. UI 返回的路径仍需 core 路径、权限和 capability root 校验；
8. DestinationPlan 只能包含 root 下的 normalized relative path；
9. no-clobber 计划不能因 TOCTOU 静默升级为 overwrite；
10. Changed identity 不继承 trusted 目录或直接弹出文件选择器；
11. UI 队列、pending request、manifest、replay cache 和 callback 等待均有硬上限；
12. 本地绝对保存路径不发送给远端。

## 8. 测试策略

### 8.1 协议与属性测试

- QSP/1.0 INFO/offer fixture 保持可解码和兼容序列化；
- 1.0 请求永远收不到新 capability；1.1 agent 才暴露 remote selection；
- selection request 的端口、字段组合、状态与大小边界；
- initiatedBy 缺失/错配/replay/过期；
- 任意 destination plan 不能产生绝对路径、`..`、Windows 保留名或 root 逃逸；
- 目录 rename/skip 对后代映射保持闭包。

### 8.2 单元测试

- 每设备目录持久化、名称变化、key mismatch、不保存 AcceptOnce；
- active binding 的 create/reopen/remove、损坏文件失败关闭；
- UI broker 单飞、Busy、deadline、channel close；
- conflict 两步选择和 apply-all；
- request manager rate/replay/idempotency；
- callback 地址只使用认证源 IP。

### 8.3 集成测试

使用 fake desktop + loopback Noise：

- Linux request → Windows 多文件选择 → callback → 完整接收；
- 文件夹递归和目录结构；
- Unknown AcceptOnce、AcceptAndTrust、Reject；
- Trusted 直接选择；Changed 拒绝；
- Linux send → Windows 目录确认/更改；
- 多设备目录隔离和重启恢复；
- duplicate selection request 不重复打开 UI/发送；
- UI Busy、取消、超时、源失效、目标不可写；
- overwrite/skip/rename/逐项/apply-all；
- commit 前竞态冲突；
- callback 错误、断线、取消和现有 resume 回归。

### 8.4 Windows 真机与 CI

- Windows MSVC 原生构建、test、Clippy；
- Windows GNU cross-check 不因资源嵌入退化；
- rfd 文件多选、文件夹选择、custom button、owner/focus；
- agent 后台/托盘、第二请求 Busy、托盘退出；
- Windows Defender Firewall 仍只读诊断，不自动修改；
- 真实 Linux ↔ Windows 双机两个主流程；
- 锁屏/无交互环境记录真实行为，不能把 hosted runner 当作桌面 UI 验收。

## 9. 发布与回滚

- 不改变 workspace 单一版本源；本功能开发阶段不执行版本 bump；
- 新协议保持 major 1，按 minor 1 协商；
- agent 为显式 opt-in 命令，旧 receive 路径可作为回滚；
- Windows UI 依赖仅 target-gated，若依赖审核或真机门失败，则不得发布 agent；
- 不移除现有 terminal receive、send、Web fallback 或 resume 路径；
- 最终 release 仍须遵守仓库 T-024/安全和真实主机门，本文不覆盖发布批准。

## 10. 已知风险与缓解

| 风险 | 等级 | 缓解 |
|---|---|---|
| rfd 自定义按钮依赖 Common Controls v6 manifest | 中 | 首任务做真实 Windows spike；嵌入 manifest；MSVC/GNU 双检查 |
| 托盘/event loop 与 Tokio 生命周期复杂 | 高 | 主线程单一 event loop；bounded broker；集成 shutdown 测试 |
| 多目标 receiver 改动 staging/resume | 高 | Router 包装现有 ReceiverService；active binding；现有 receiver 全量回归 |
| 逐项冲突计划破坏原子提交 | 高 | planned commit 独立 API；intent journal；故障注入和 TOCTOU 测试 |
| callback 被用于反射或身份替换 | 高 | IP 取认证 socket；固定完整 key；一次性 expected offer |
| 大文件 manifest 准备期间请求超时 | 中 | blocking worker、独立有界准备超时、取消传播 |
| Windows 锁屏不能立即显示 UI | 中 | 不穿透锁屏；deadline 后 Expired；托盘/user-attention；真机记录 |
| 新 capability 破坏 1.0 strict decoder | 高 | 动态 INFO 降级；1.0 fixture 和旧二进制互操作测试 |
