---
feature: windows-file-picker
batch: 0
task: T-001
review: two-stage
status: passed-with-approved-deferral
generated_at: 2026-07-27T10:35:28Z
---

# Batch 0 两阶段评审

## 阶段 1：规范符合性评审

### 判定：通过（含用户明确批准的 MSVC 延期）

评审直接读取 spike 源码、构建配置、PE 资源、测试输出、Windows SSH 输出和真机交互结果，没有只依赖实施摘要。

| T-001 要求 | 状态 | 证据 |
|---|---|---|
| 自定义文件/文件夹/取消按钮 | 通过 | rfd custom buttons；Windows 11 用户可见并完成操作 |
| 原生文件多选 | 通过 | `pick_files`；用户按指示完成多文件选择并看到数量提示 |
| 原生文件夹选择 | 通过 | `pick_folder`；Windows 11 用户完成选择 |
| 起始目录 | 通过 | spike 显式 `set_directory(initial_directory())` |
| hidden owner + parent | 通过 | winit hidden window；所有 rfd dialog `set_parent(window)` |
| 后台 attention | 通过 | `request_user_attention`；活动 Session 1 可见 |
| 托盘打开/退出 | 通过 | 用户从托盘重新打开并退出；task result 0；无残留进程 |
| Tokio 后台 → bounded UI channel | 通过 | Tokio runtime + capacity-1 sync channel + EventLoopProxy |
| single-flight | 通过 | RAII `UiGate` 测试覆盖 acquire/busy/drop/reacquire |
| Common Controls v6 | 通过 | PE Type ID 0x18 manifest；真机 TaskDialog 正常 |
| Windows GNU cross-build | 通过 | check/build/Clippy `-D warnings` |
| 许可证/RustSec | 通过 | Windows graph 许可完整；audit 无 warning；排除旧 GTK 图 |
| Windows 11 交互真机 | 通过 | Enterprise 22621，console Session 1，多文件/目录/tray |
| Windows MSVC native | 延期 | 主机无 Rust/VS/Build Tools；用户批准设为 T-007 合入前硬门 |

范围控制符合：spike 位于独立 `spikes/` workspace，未修改产品 crate，未实现真实传输，未执行版本 bump/tag/release。

## 阶段 2：代码质量评审

### 判定：通过

### 已发现并在评审循环中修复

1. **跨平台 tray lockfile 审计污染**
   - 初始 `tray-icon` 将旧 GTK3 unmaintained/unsound warning 带入 lockfile。
   - 改为 Windows-only `tray-icon-win`，Windows target graph 不含 GTK/ATK/glib。

2. **GNU cross-build 未嵌入资源**
   - build script 的 `cfg(windows)` 判断 build host 而非 target。
   - 改为 Cargo target cfg 环境判断；PE 出现 `.rsrc`。

3. **GNU manifest 类型错误**
   - `RT_MANIFEST` 被 windres 当成字符串资源，Windows 返回 `0xC0000139`。
   - 改为数值类型 `24`；PE 显示 ID `0x18`；Windows 真机通过。

4. **SSH 证据脚本输出失控**
   - PowerShell `Get-Content` 扩展属性导致 JSON 膨胀，且混合编码。
   - 改为显式 UTF-8 byte decode、GBK console 输出和 GB18030→UTF-8 采集。

5. **无交互桌面判定**
   - SSH session 为 `UserInteractive=false`，winit 初始化失败。
   - 脚本现在将明确初始化拒绝识别为预期 fail-closed 证据并完成清理。

### 质量评价

| 维度 | 结果 |
|---|---|
| 可读性 | 模型、平台代码和验证脚本职责清晰 |
| 可测试性 | single-flight 自动测试；PE 静态检查；Windows 交互证据 |
| 可维护性 | disposable spike 与生产代码隔离；版本和资源方案已记录 |
| 安全性 | 无新 advisory；许可白名单内；无项目内 unsafe；无路径/内容采集 |
| 清理 | Windows task/temp/process 全部复核为不存在 |

### 强制后续项

- T-007 合入生产 Windows UI 前必须通过 Windows MSVC native test、Clippy 和 release build；
- 生产 agent 在无 UI 环境必须返回 `UiUnavailable` 或非成功状态；spike 当前仅打印诊断后 exit 0，不得复制；
- T-010 继续验证锁屏、MSVC 真机、真实 Linux↔Windows 流程。

当前无未解决代码级 P0/P1。MSVC 是已批准延期的发布/合入硬门，不是被标记为已通过。

## 验证摘要

```text
cargo test --workspace --all-targets                         PASS (baseline)
cargo test --manifest-path spikes/Cargo.toml --workspace    PASS
Windows GNU check/build                                      PASS
Windows GNU Clippy -D warnings                               PASS
cargo audit --file spikes/Cargo.lock --no-fetch              PASS
PE numeric RT_MANIFEST 0x18                                  PASS
Windows 11 multi-file/folder/tray/exit                       PASS
Windows cleanup: process/task/temp                            PASS
Windows MSVC native                                          DEFERRED (approved; T-007 hard gate)
```
