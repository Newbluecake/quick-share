---
feature: windows-file-picker
complexity: complex
workflow: spec-dev
preset: normal
generated_at: 2026-07-27T09:54:11Z
status: batch-3-awaiting-approval
---

# Windows File Picker - SDD Context

## 配置

```yaml
feature_name: windows-file-picker
planning: batch
execution: batch
complexity: complex
requirements_source: docs/dev/windows-file-picker/windows-file-picker-requirements.md
worktree: disabled-by-user
branch: master
with_review: false
parallel: auto
```

## 阶段状态

| 阶段 | 状态 | 产物 |
|---|---|---|
| 阶段 0：环境选择 | 已完成 | 用户选择在当前主工作区执行 |
| 阶段 1：需求分析 | 已完成并确认 | `windows-file-picker-requirements.md` |
| 阶段 2：技术设计 | 已生成并获用户批准 | `windows-file-picker-design.md` |
| 阶段 3：任务拆分 | 已生成并获用户批准 | `windows-file-picker-tasks.md` |
| 阶段 4：环境准备 | 已完成 | 主 workspace 基线测试通过；用户选择主工作区 |
| 阶段 5：代码实施 | Batch 0、Batch 1、Batch 2、Batch 3 已完成 | T-007/T-008/T-009 两阶段评审通过；真机 F-001/F-002/F-003 已验证；等待进入 T-010 总验收 |

## 规划结论

- 新增 Windows-only `quick-share agent`，而不是改变现有 terminal receiver 默认行为；
- Linux 使用 `quick-share receive --request [--peer ...]` 请求 Windows 选择源内容；
- 采用两条 Noise XX 认证连接的 callback 模型，复用现有传输数据面；
- QSP/1.1 增加 remote-selection 消息，通过动态 INFO 响应兼容 QSP/1.0；
- 保存目录按稳定 DeviceId 持久化，活动 transfer 额外绑定原 output root；
- DestinationPlan 支持逐项 overwrite/skip/rename 和 apply-all；
- Windows UI 采用 rfd + tray-icon-win + winit 候选；跨平台 tray-icon 因旧 GTK lockfile 审计 warning 已在 T-001 排除；
- 计划分 5 个批次、10 个任务执行。

## 已完成实施

### Batch 0

- T-001 Windows desktop spike 已完成；
- Linux 自动化、Windows GNU cross-build、manifest、许可证、RustSec 和 Windows 11 交互式桌面已通过；
- 用户批准 Conditional Go；MSVC native test/Clippy/release build 仍为 T-007 生产 UI 合入前硬门；
- 评审记录：`windows-file-picker-batch-0-review.md`。

### Batch 1

- T-002：QSP/1.1 remote-selection 消息、动态 INFO 兼容、offer correlation、协商约束和 Noise control codec 已完成；
- T-003：受信设备目录偏好、活动 receive binding、逐项 `DestinationPlan` 和路径/冲突不变量已完成；
- T-004：`agent`、`receive --request [--peer ...]` CLI 合同以及平台无关 `DesktopInteraction` 已完成；
- 规范符合性评审和代码质量评审均通过；
- 评审记录：`windows-file-picker-batch-1-review.md`。

### Batch 2

- T-005：多目标 `ReceiverRouter`、planned file/metadata commit、冲突待决/计划更新、binding 驱动 resume 与统一资源上限已完成；
- T-006：`SelectionManager`、一次性 expected callback、动态 INFO、direct selection dispatch 和 callback offer 自动授权已完成；
- Noise fuzz target 已覆盖 selection request/response，并通过固定时长 fuzz smoke；
- 规范符合性评审和代码质量/安全评审均通过；
- 评审记录：`windows-file-picker-batch-2-review.md`。

### Batch 3（进行中）

- T-007：生产 `rfd + tray-icon-win + winit` adapter、owner window、托盘、Common Controls v6 manifest、有界 single-flight UI broker 和 graceful exit 已完成；
- Windows MSVC 全 workspace native tests、Clippy、release build 已通过；
- 用户完成多文件、文件夹、取消、托盘重开和退出真机清单；最终系统复核 `ProcessCount=0`、任务退出码 `0x00000000`；
- T-007 证据：`windows-file-picker-t007-acceptance.md`。

### Batch 3（已完成）

- T-007：生产 `rfd + tray-icon-win + winit` adapter、owner window、托盘、Common Controls v6 manifest、有界 single-flight UI broker 和 graceful exit 已完成；
- T-008：Windows 常驻 agent（主线程事件循环 + 后台 Tokio）、授权/目录/冲突 UI、按受信身份记忆目录、认证回连已完成；
- T-009：Linux `receive --request`、expected-callback 精确校验、终态状态到 CLI 退出类映射已完成；
- 真机验证：Windows MSVC 全 workspace native tests/Clippy/release；Linux ↔ Windows 11 实传 F-001（单文件，部分）、F-002（文件夹递归与结构保留）及仅允许一次授权；
- 实传中发现并修复两个 Critical UI 缺陷（隐藏 owner 导致对话框空白/关闭 owner 误退 agent）；
- 评审记录：`windows-file-picker-batch-3-review.md`、`windows-file-picker-t007-acceptance.md`。

## 当前闸门

Batch 0–Batch 3 已全部完成并通过两阶段评审。剩余工作为 T-010 总验收：在真机补齐多文件选择（F-001）、选择取消（F-003）、冲突覆盖/跳过/重命名（F-013/F-014）、更改目录（F-005）、身份变更拒绝、UI 不可用 fail-closed，以及 Linux GUI。最终 `v2.0.0` tag 仍需 T-024 发布验收与 `snow 0.10.0`、macOS 真机门。
