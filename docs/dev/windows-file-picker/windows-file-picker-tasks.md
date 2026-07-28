---
feature: windows-file-picker
stage: tasks
complexity: complex
generated_by: spec-dev
generated_at: 2026-07-27T09:54:11Z
version: 1
status: approved
execution: batch
---

# 任务拆分: Windows 远程文件与文件夹选择

## 1. 执行规则

- 工作流：SDD normal，Planning 批量确认，Execution 按批次确认。
- 当前选择：在主工作区执行，不创建 Worktree。
- 开发方法：每个生产任务遵循 Red → Green → Refactor；不得先写实现再补测试。
- 安全要求：协议、身份、callback、路径和冲突提交属于高风险区域，每批完成后执行规范符合性与代码质量两阶段评审。
- 兼容要求：QSP/1.0、现有 `send`、被动 `receive`、Web、resume 和发布检查不得回退。
- 平台要求：Windows UI 依赖必须 target-gated；Linux/macOS workspace 构建不能被 GUI 依赖污染。
- 发布要求：本功能不自动 bump 版本、不创建 tag，不绕过仓库发布闸门。

## 2. 批次与依赖图

```text
Batch 0: T-001 Windows UI/托盘可行性验证
                    │ Go
                    ▼
Batch 1: T-002 QSP/1.1 协议 ─────────────┐
         T-003 目录偏好/绑定/目标计划 ────┼─ 可并行
         T-004 CLI 合同与平台抽象 ────────┘
                    │
                    ▼
Batch 2: T-005 多目标 receiver + planned commit
         T-006 selection 状态机 + direct dispatch
                    │
                    ▼
Batch 3: T-007 Windows dialog/tray/UI broker
         T-008 Windows agent 双向编排
         T-009 Linux remote receive 编排
                    │
                    ▼
Batch 4: T-010 端到端、安全、CI、文档与验收
```

关键依赖：

- T-001 是 Windows 技术方案 Go/No-Go 前置；失败时停止，不进入正式实现。
- T-005 依赖 T-002、T-003。
- T-006 依赖 T-002、T-004。
- T-007 依赖 T-001、T-004。
- T-008 依赖 T-003、T-005、T-006、T-007。
- T-009 依赖 T-002、T-004、T-005、T-006。
- T-010 依赖全部生产任务。

## 3. Batch 0：Windows 桌面技术可行性

### T-001：验证 rfd + Windows-only tray + winit + Windows manifest

**目标**：在不修改正式产品路径的前提下，验证候选依赖能满足原生选择器、自定义按钮、parent/focus、托盘、Tokio 跨线程和项目构建约束。

**预计文件**：

- `spikes/Cargo.toml`
- `spikes/windows-desktop/Cargo.toml`
- `spikes/windows-desktop/build.rs`
- `spikes/windows-desktop/windows-manifest.xml`
- `spikes/windows-desktop/src/main.rs`
- `docs/dev/windows-file-picker/windows-file-picker-spike.md`

**Red / 验证清单**：

1. 先定义可复现验收脚本或检查表，覆盖：
   - 自定义“选择文件 / 选择文件夹 / 取消”按钮；
   - 文件多选；
   - 文件夹选择；
   - 设置初始目录；
   - hidden owner window + parent；
   - 后台时 request-user-attention；
   - tray icon、菜单、退出；
   - Tokio 后台线程通过 bounded channel 唤醒 UI；
   - UI 同时只处理一个请求。
2. 未嵌入 Common Controls v6 manifest 时记录 custom button 的失败/退化证据。

**Green**：

- 选择最小可行版本组合；
- 嵌入 Windows manifest；
- Windows 11 MSVC 真机完成清单；
- Linux 上主 workspace 不因 spike 增加系统 GUI 依赖；
- 验证 `#![forbid(unsafe_code)]` 可保留在项目源码中。

**质量与安全检查**：

- `cargo tree` 记录依赖面；
- `cargo deny check` / RustSec；
- 许可证、MSRV 1.92、维护状态；
- Windows MSVC 原生 build；
- Windows GNU cross-check；
- 关闭应用时没有遗留网络/UI 线程。

**完成定义**：

- 形成明确 Go/No-Go 结论；
- 若 custom button、event loop、cross-build、许可证或安全审计任一 P0 不满足，停止并回到设计阶段；
- spike 代码不得直接复制为生产实现，正式任务仍需 TDD。

**关联验收**：F-015、F-016、F-017。

## 4. Batch 1：协议、持久化与公共合同

### T-002：扩展 QSP/1.1 远程选择协议并保持 1.0 兼容

**目标**：增加 selection 控制消息、可选 offer correlation 和动态 INFO capability，不改变 QSP/1.0 普通传输。

**预计文件**：

- `crates/quick-share-protocol/src/lib.rs`
- `crates/quick-share-protocol/tests/protocol.rs`
- `crates/quick-share-discovery/src/lib.rs`
- `crates/quick-share-discovery/tests/discovery.rs`
- `crates/quick-share-transfer/src/noise.rs`
- `crates/quick-share-transfer/tests/noise.rs`

**Red**：

- 1.1 request/response round-trip 测试；
- callback port 0、非法字段组合、未知状态、超大字段拒绝；
- `Ready` 必须有 transfer ID，其他状态不得携带；
- `TransferOffer.initiated_by` correlation 测试；
- 1.0 INFO 请求断言响应不含 remote-selection capability；
- 1.0 普通 offer fixture 序列化不出现新字段；
- mDNS capability 保持旧 parser 可读。

**Green**：

- 增加 `ProtocolVersion::V1_1`；
- 增加 `Capability::RemoteSelection`；
- 增加 MessageType 11/12 和 strict DTO；
- 增加动态 INFO 响应辅助函数；
- 普通 offer `initiated_by=None` 时跳过序列化。

**Refactor / 评审重点**：

- 所有字符串、集合和序列化大小有界；
- 旧 peer 不会收到它无法解析的新 capability；
- 未协商 1.1 时发送 selection message 必须失败关闭；
- 更新协议 fixture 而非只做同版本自测。

**完成定义**：协议、发现、Noise control codec 测试通过；QSP/1.0 fixtures 无回归。

**关联验收**：F-008、F-009、F-010、F-011、F-018、F-019。

### T-003：实现每设备目录偏好、活动绑定和 DestinationPlan

**目标**：提供与协议无关的本地持久化和可验证目标计划。

**预计文件**：

- `crates/quick-share-core/src/lib.rs`
- `crates/quick-share-core/src/receive_preferences.rs`
- `crates/quick-share-core/src/receive_binding.rs`
- `crates/quick-share-core/src/destination_plan.rs`
- `crates/quick-share-core/tests/receive_targets.rs`
- `crates/quick-share-platform/src/lib.rs`

**Red**：

- 设备 A/B 目录隔离、重启恢复、设备改名保持；
- Unknown AcceptOnce 不持久化；受信任设备才持久化；
- key mismatch 不复用目录；
- 无记录回退 `AppDirs::download_dir()`；
- 无效/损坏 store 安全失败；
- binding 匹配 transfer/sender/manifest/root；
- binding cleanup 与 paused 保留；
- 文件 conflict overwrite/skip/rename；
- 目录 overwrite 合并、skip 子树、rename 前缀重写；
- Windows 大小写、Unicode、保留名、绝对路径、`..` 和 symlink ancestor；
- apply-all 只影响后续同类冲突；
- 属性测试保证 plan 的所有 commit path 都是 root 下规范相对路径。

**Green**：

- 原子版本化 store；
- 稳定 `DeviceId` 主键；
- `DestinationDisposition` / `DestinationPlan`；
- manifest 预检和目录后代重写；
- plan 更新必须重新完整验证。

**Refactor / 评审重点**：

- 不持久化 authorization proof、私钥或远端不需要的内容；
- `Debug` 不批量打印路径；
- 存储并发写入不丢失其他设备记录；
- 路径检查不使用不安全字符串前缀判断。

**完成定义**：core 单元/属性测试通过，原有 config/path/manifest 测试不变。

**关联验收**：F-006、F-007、F-010、F-013、F-014、F-018。

### T-004：增加 CLI 合同与平台无关 DesktopInteraction 抽象

**目标**：先固定用户入口和可测试 UI DTO，不接入真实 Windows 实现。

**预计文件**：

- `crates/quick-share-cli/src/lib.rs`
- `crates/quick-share-cli/tests/cli.rs`
- `crates/quick-share-cli/tests/contract.rs`
- `crates/quick-share-platform/src/lib.rs`
- `crates/quick-share-platform/src/desktop.rs`
- `crates/quick-share-platform/tests/desktop.rs`

**Red**：

- `quick-share agent` 解析；
- `receive --request` 和 `--peer` 组合；
- remote request 模式隐含 one-shot；
- 非交互 `--request` 无 peer 时拒绝；
- 与不兼容 receive 参数的冲突测试；
- non-Windows agent 返回稳定 unsupported；
- `SourceChoice`、`DirectoryChoice`、`AuthorizationChoice`、`ConflictChoice` 的取消和错误映射。

**Green**：

- 新增 `AgentIntent`；
- 扩展 `ReceiveIntent`；
- 新增平台无关 desktop DTO/trait 和 Unsupported stub；
- 保持现有 argv[0] `rc`/`sc` 和退出码。

**Refactor / 评审重点**：

- CLI parser 不依赖 Windows crate；
- 路径与设备名称在 UI DTO 中明确标记为本地/不可信；
- 取消映射到 `AppError::Cancelled`，不是 filesystem/network error。

**完成定义**：CLI/contract/platform 测试通过；帮助文本清晰说明 agent 第一阶段仅 Windows。

**关联验收**：F-003、F-004、F-020。

## 5. Batch 2：接收路由与远程选择状态机

### T-005：实现多目标 ReceiverRouter 和 planned commit

**目标**：让一个 agent 对每个 transfer 使用独立 output root 和 DestinationPlan，同时保持 staging/resume/原子提交不变量。

**预计文件**：

- `crates/quick-share-transfer/src/lib.rs`
- `crates/quick-share-transfer/src/receiver.rs`
- `crates/quick-share-transfer/src/receiver_router.rs`
- `crates/quick-share-transfer/tests/receiver.rs`
- `crates/quick-share-transfer/tests/store.rs`
- `crates/quick-share-transfer/tests/resume.rs`

**Red**：

- 两个 transfer 同时路由到不同 root；
- 全局 max_receive_tasks 不能因多个 root 绕过；
- binding 在 authorization 之前存在；
- sender/manifest/root 不匹配拒绝 reopen；
- planned overwrite/skip/rename 的 file commit；
- 目录 merge/skip/rename 后代；
- commit 前新冲突返回 `ConflictPending`，不静默覆盖；
- 更新 plan 后重试 complete；
- crash 注入在 intent/rename/journal 各点可恢复；
- pause/restart 使用原 root；原 root 丢失明确失败；
- cleanup 删除 staging + active binding；
- 现有固定-root ReceiverService 测试继续通过。

**Green**：

- `ReceiverRouter` 按 transfer ID 创建/重开 receiver；
- agent 无 default target，terminal receiver 保留 default target；
- `TransferStore` 增加 planned batch commit API；
- receiver 暴露冲突预检/计划更新；
- active binding 驱动 resume 路由。

**Refactor / 评审重点**：

- 不在持有 std mutex 时等待 UI；
- 不在 output root 之间移动 staging；
- no-clobber 与 overwrite intent 区分；
- capability-based directory/file operations继续检查 symlink ancestor。

**完成定义**：receiver/store/resume 全量通过，故障注入和属性测试无回归。

**关联验收**：F-004、F-005、F-007、F-013、F-014、F-018、F-019。

### T-006：实现 SelectionManager、expected callback 和 direct dispatch

**目标**：实现有界、幂等、身份绑定的远程选择控制面，不接真实 GUI。

**预计文件**：

- `crates/quick-share-transfer/src/auth.rs`
- `crates/quick-share-transfer/src/direct.rs`
- `crates/quick-share-transfer/src/selection.rs`
- `crates/quick-share-transfer/src/expected_offer.rs`
- `crates/quick-share-transfer/tests/auth.rs`
- `crates/quick-share-transfer/tests/direct.rs`
- `crates/quick-share-transfer/tests/selection.rs`

**Red**：

- trusted/unknown/changed 请求分类；
- Unknown AcceptOnce 与显式 AcceptAndTrust；
- Changed 默认拒绝；
- 每设备 rate limit、全局 pending、replay cache；
- 同 request ID 幂等且只调用一次 fake selector；
- 同时第二个 UI 请求 Busy；
- timeout/取消后不能延迟启动 callback；
- callback IP 必须等于认证连接源 IP；
- callback 完整 key mismatch、DeviceId mismatch、request mismatch、transfer mismatch、过期和 replay 全部拒绝；
- expected callback 精确匹配后只消费一次；
- 未协商 remote selection 时消息 11 关闭或返回兼容错误；
- 普通 offer 路径不受影响。

**Green**：

- `SelectionManager` 状态机；
- 一次性 callback grant；
- dynamic INFO；
- direct server/client selection exchange；
- OfferView/callback 检查所需的最小 correlation 字段。

**Refactor / 评审重点**：

- 不接受 wire callback IP；
- trust 只绑定完整静态公钥；
- 缓存终态不缓存 secret；
- 错误响应不泄露本地路径或内部堆栈；
- selection control timeout 与普通 chunk timeout 分开。

**完成定义**：selection/direct/auth 集成测试通过；Noise fuzz target 覆盖新 message types。

**关联验收**：F-008、F-009、F-010、F-011、F-012、F-016、F-018。

## 6. Batch 3：Windows UI 与产品编排

### T-007：实现 Windows 原生 dialogs、owner window、tray 和 UI broker

**目标**：将 T-001 结论以可测试生产代码接入 `quick-share-platform`。

**预计文件**：

- `Cargo.toml`
- `Cargo.lock`
- `crates/quick-share-platform/Cargo.toml`
- `crates/quick-share-platform/src/desktop/windows.rs`
- `crates/quick-share-platform/src/tray.rs`
- `crates/quick-share-platform/tests/desktop.rs`
- `crates/quick-share-cli/build.rs`
- `crates/quick-share-cli/windows-manifest.xml`

**Red**：

- fake backend 验证所有 button/result 映射；
- close/X 对每个 dialog 都映射 Cancelled；
- file 多选为空不算成功；
- directory invalid/unwritable 返回可区分错误；
- conflict 两步流程和 apply-all；
- UI broker 单飞、Busy、deadline、event loop exit；
- bounded channel 满时不阻塞网络线程；
- tray Exit 触发 graceful cancellation。

**Green**：

- rfd 文件/文件夹/message dialogs；
- hidden owner window + parent；
- tray menu；
- event loop proxy；
- request-user-attention；
- Common Controls v6 manifest。

**Refactor / 评审重点**：

- 所有真实 dialog 调用仅发生在 owner event-loop thread；
- 网络/runtime 线程不持有窗口对象；
- target dependencies 只在 Windows；
- 项目源码继续 forbid unsafe；
- 锁屏/无 UI 时按 deadline 失败，不尝试越权显示。

**完成定义**：fake 测试、Windows 真机 UI 清单、MSVC/GNU build、cargo-deny/RustSec 通过。

**关联验收**：F-001、F-002、F-003、F-004、F-005、F-013、F-014、F-015、F-016、F-017。

### T-008：实现 Windows agent 双向编排

**目标**：把 listener、offer prompt、目录偏好、冲突 planner、source selector 和 callback sender 组合为可驻留 agent。

**预计文件**：

- `crates/quick-share-cli/src/main.rs`
- `crates/quick-share-cli/src/app.rs`
- `crates/quick-share-cli/src/agent.rs`
- `crates/quick-share-cli/src/desktop_prompt.rs`
- `crates/quick-share-cli/tests/agent.rs`
- `crates/quick-share-cli/tests/orchestration.rs`

**Red**：

- trusted incoming offer 直接显示目录确认但不静默接受；
- unknown 先授权再目录确认；changed 拒绝；
- 默认 per-device dir、change dir、invalid history；
- 仅 trusted 或显式 AcceptAndTrust 后保存目录；
- binding 失败时不得授权；
- current conflict + race conflict UI；
- trusted source request 直接源选择；unknown 先授权；
- 文件多选/文件夹路径进入 existing ManifestBuilder；
- selector 取消返回 Cancelled 且不启动 sender；
- callback 使用认证 source IP、port 和 pinned key；
- callback preparation/transfer 失败两端获得有界结果；
- agent shutdown 取消 listener、UI、callback sender 并 pause receiver；
- 第二请求 Busy，不叠加窗口。

**Green**：

- 同步 main 对 agent 特例运行 Windows event loop；
- Tokio runtime 后台线程；
- agent listener 广告 QSP/1.1；
- desktop prompt adapter；
- callback outbound 使用已抽取的 production sender service；
- tray 状态与退出。

**Refactor / 评审重点**：

- agent policy 不复用 trusted auto-accept；
- 目录选择、plan、binding、offer grant 顺序不可颠倒；
- UI 取消是正常状态；
- manifest/hash 在 blocking worker；
- 不持久化 remote-selected source path；进程内自动重试仍复用现有 sender policy。

**完成定义**：agent orchestration fake 测试通过，Windows 本机可作为 sender/receiver 双向运行。

**关联验收**：F-001 至 F-018。

### T-009：实现 Linux `receive --request` 与 expected callback 接收

**目标**：让 Linux 用户从 CLI 选择 agent、请求 Windows 选择源，并安全接收精确 callback transfer。

**预计文件**：

- `crates/quick-share-cli/src/app.rs`
- `crates/quick-share-cli/src/remote_receive.rs`
- `crates/quick-share-cli/src/orchestration.rs`
- `crates/quick-share-cli/src/terminal.rs`
- `crates/quick-share-cli/tests/remote_receive.rs`
- `crates/quick-share-cli/tests/orchestration.rs`

**Red**：

- receiver 必须先 bind，再发送 request；
- auto discovery 只列出协商后支持 remoteSelection 的 peer；
- explicit device/address 路径；
- non-interactive 无 peer 失败；
- unknown Windows 的 TOFU/SAS；
- selection Ready 注册 expected transfer；
- Cancelled exit 0；Rejected/Busy/Unavailable/Expired/Failed 映射稳定错误；
- callback offer 必须精确匹配 request/transfer/DeviceId/key；
- 非预期 offer 不能因 trusted auto policy 混入本次 one-shot；
- request/callback timeout 后关闭 listener并清理临时 expectation；
- `--output`、目录结构和 existing receiver cleanup；
- 普通被动 receive 不变。

**Green**：

- `RemoteReceiveOrchestrator`；
- discovery/explicit target 选择；
- callback listener + selection client 并发；
- expected callback prompt；
- 用户可读终端状态。

**Refactor / 评审重点**：

- 扫描失败不伪装为零设备；
- 连接 selection peer 后不回退 Web；
- 回调等待有界；
- one-shot 完成、取消、失败都释放 listener 和 mDNS registration。

**完成定义**：Linux loopback fake-agent 测试通过；现有 receive/orchestration 测试全部通过。

**关联验收**：F-001、F-002、F-003、F-018、F-020。

## 7. Batch 4：整体验收与发布质量

### T-010：端到端、安全、跨平台 CI、文档和验收报告

**目标**：证明两个首期流程、全部边界和回归门满足 requirements。

**预计文件**：

- `crates/quick-share-cli/tests/remote_picker_e2e.rs`
- `crates/quick-share-transfer/tests/selection.rs`
- `crates/quick-share-core/tests/receive_targets.rs`
- `fuzz/fuzz_targets/noise_records.rs` 或新增 selection fuzz target
- `.github/workflows/ci.yml`
- `README.md`
- `README.zh-CN.md`
- `docs/migration-v2.md`（仅在确有用户行为变化时）
- `docs/dev/windows-file-picker/windows-file-picker-acceptance-report.md`

**自动化验证**：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu
cargo deny check
cargo audit --locked
git diff --check
```

并执行：

- QSP/1.0 compatibility fixture；
- selection control fuzz；
- 路径/DestinationPlan 属性测试；
- receiver crash/resume/fault matrix；
- Windows MSVC release build；
- 现有 release contract tests。

**真实主机验收**：

1. Linux `receive --request` → Windows 多文件 → Linux；
2. Linux `receive --request` → Windows 文件夹 → Linux，目录结构一致；
3. Linux `send` → Windows 默认 per-device 目录；
4. Windows 更改目录、重启 agent、按设备恢复；
5. unknown/trusted/changed identity；
6. overwrite/skip/rename/逐项/apply-all；
7. 后台/tray/focus、Busy、取消、超时；
8. 目标不可写、历史目录失效、源在选择后删除；
9. duplicate request 不重复弹窗/传输；
10. Windows Public profile 诊断保持只读；
11. 已有 direct transfer resume 不回归。

**文档**：

- Windows 启动 agent 的命令、托盘行为和交互式桌面限制；
- Linux remote receive 命令示例；
- 已配对/未配对/SAS 行为；
- 每设备保存目录和冲突处理；
- 防火墙和无 UI 环境排错；
- 第一阶段不含 Linux/macOS picker、开机自启和 Windows Service。

**完成定义**：

- requirements F-001 至 F-020 均有测试或真实主机证据；
- 两阶段评审无未解决 P0/P1；
- acceptance report 给出 PASS / CONDITIONAL PASS / FAIL；
- 未获得发布批准前不 bump 版本、不打 tag、不创建 release。

## 8. 验收映射

| 验收项 | 主要任务 |
|---|---|
| F-001 多文件选择 | T-007、T-008、T-009、T-010 |
| F-002 文件夹选择 | T-003、T-007、T-008、T-010 |
| F-003 发送选择取消 | T-004、T-007、T-008 |
| F-004 接收目录确认 | T-003、T-005、T-007、T-008 |
| F-005 更改目录 | T-003、T-007、T-008 |
| F-006 按设备记忆 | T-003、T-008、T-010 |
| F-007 历史目录失效 | T-003、T-005、T-008 |
| F-008 已配对快速交互 | T-002、T-006、T-008 |
| F-009 未配对授权 | T-002、T-006、T-008 |
| F-010 身份变化隔离 | T-003、T-006、T-008 |
| F-011 请求防重 | T-002、T-006 |
| F-012 防骚扰 | T-006、T-008 |
| F-013 冲突逐项 | T-003、T-005、T-007、T-008 |
| F-014 apply-all | T-003、T-005、T-007 |
| F-015 后台唤起 | T-001、T-007、T-010 |
| F-016 UI Busy | T-006、T-007、T-008 |
| F-017 无交互桌面 | T-001、T-007、T-010 |
| F-018 错误反馈 | T-002、T-005、T-006、T-008、T-009 |
| F-019 现有回归 | T-002、T-005、T-010 |
| F-020 第一阶段边界 | T-004、T-009、T-010 |

## 9. 规划估算

| 批次 | 任务 | 复杂度 | 可并行性 |
|---|---:|---|---|
| Batch 0 | 1 | 中 | 否，前置闸门 |
| Batch 1 | 3 | 高 | T-002/T-003/T-004 可并行 |
| Batch 2 | 2 | 高 | 初期可并行，集成时串行 |
| Batch 3 | 3 | 很高 | T-007 与 T-009 部分并行；T-008 后集成 |
| Batch 4 | 1 | 高 | 自动检查可并行，真机流程串行 |

总计 10 个任务。最大风险集中在 T-005（多目标与 planned commit）、T-006（callback 身份绑定）和 T-008（Windows agent 生命周期）。
