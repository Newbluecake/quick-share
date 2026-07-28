---
feature: windows-file-picker
task: T-001
stage: execution
batch: 0
status: conditional-go-approved
generated_at: 2026-07-27T09:54:11Z
---

# T-001 Windows 桌面可行性验证

## 1. 当前结论

**建议：Conditional Go，等待用户批准将 MSVC 原生构建门延期到生产 UI 任务 T-007。**

Linux 自动化、Windows GNU 交叉编译、资源嵌入、依赖图、许可证、RustSec 和 Windows 11 交互式桌面真机均已通过。Windows 真机没有 Rust、Cargo、Visual Studio 或 MSVC Build Tools，因此本批无法完成 MSVC 原生构建；在未获得明确延期批准前，不宣称完整 Go，也不进入 Batch 1。

## 2. 候选方案

| 能力 | 候选 | 版本 | 结论 |
|---|---|---:|---|
| 原生文件/文件夹/消息对话框 | `rfd` | 0.17.2 | GNU cross-build + Windows 11 真机通过 |
| Windows-only 托盘 | `tray-icon-win` | 0.1.5 | Windows 11 托盘重开/退出通过 |
| owner window / event loop | `winit` | 0.30.13 | 活动 Session 1 启动并保持 Responding |
| Common Controls v6 manifest | `embed-resource` | 3.0.11 | PE 中已验证 RT_MANIFEST 和 `.rsrc` |

最初评估的跨平台 `tray-icon` 被替换为 Windows-only `tray-icon-win`。原因：即使顶层依赖只在 Windows 启用，跨平台 `tray-icon` 的 lockfile 仍引入旧 GTK3 依赖，`cargo audit` 报告多个 unmaintained/unsound warning。`tray-icon-win` 保留相似 API，不引入 GTK，并使用项目许可白名单允许的 MIT/Apache-2.0。

## 3. Spike 范围

路径：`spikes/windows-desktop/`

实现验证：

- Windows 主线程 `winit` event loop；
- 隐藏 owner window；
- `tray-icon-win` 托盘菜单；
- Tokio 后台 runtime 通过容量为 1 的 channel 和 `EventLoopProxy` 唤起 UI；
- RAII single-flight gate，拒绝重叠模态工作流；
- rfd 自定义“选择文件 / 选择文件夹 / 取消”；
- 原生文件多选和文件夹选择；
- parent window、初始目录和 request-user-attention；
- Common Controls v6 / asInvoker manifest。

该代码是 disposable spike，不得复制到生产 crate；正式实现仍须按 TDD 重写。

## 4. 已完成证据

### 4.1 基线

```text
cargo test --workspace --all-targets
PASS
```

### 4.2 Spike 单元测试

```text
cargo test --manifest-path spikes/Cargo.toml -p qs-windows-desktop-spike
1 passed
```

覆盖 single-flight：首请求获得 UI、重叠请求返回 Busy、lease drop 后恢复可用。

完整 spike workspace：

```text
cargo test --manifest-path spikes/Cargo.toml --workspace --quiet
PASS
```

### 4.3 Windows GNU 交叉构建与 Clippy

```text
cargo check --manifest-path spikes/Cargo.toml \
  -p qs-windows-desktop-spike \
  --target x86_64-pc-windows-gnu
PASS

cargo clippy --manifest-path spikes/Cargo.toml \
  -p qs-windows-desktop-spike \
  --target x86_64-pc-windows-gnu \
  --all-targets -- -D warnings
PASS

cargo build --manifest-path spikes/Cargo.toml \
  -p qs-windows-desktop-spike \
  --target x86_64-pc-windows-gnu --release
PASS
```

产物：

```text
spikes/target/x86_64-pc-windows-gnu/release/qs-windows-desktop-spike.exe
PE32+ console executable, x86-64
SHA-256: 91768fe2b4497339cd765ba916e247f19c20da6aa41fcae1d408eedcaf053bc3
```

### 4.4 Windows manifest

`objdump` 证明：

```text
Resource Directory [.rsrc]
resource type ID 0x18 (RT_MANIFEST) / ID 1 / language 0x409
.rsrc size 0x460
```

从 PE 提取的 manifest 文本包含：

```text
Microsoft.Windows.Common-Controls version="6.0.0.0"
requestedExecutionLevel level="asInvoker" uiAccess="false"
```

过程中发现并修复一个 cross-build 问题：build script 的 `cfg(windows)` 判断的是 build host，不是 target。现改为检查 Cargo target 环境，并已确认 GNU cross binary 确实包含 `.rsrc`。

### 4.5 依赖与安全

Windows 目标依赖树不包含 GTK、ATK、glib 或 `proc-macro-error`。主要许可证：

- rfd: MIT
- tray-icon-win / muda-win: MIT OR Apache-2.0
- winit: Apache-2.0
- embed-resource: MIT

所有解析到的 Windows 目标包均有许可证元数据，且落在仓库许可白名单。

```text
cargo audit --file spikes/Cargo.lock --no-fetch
PASS — no vulnerability/warning after replacing tray-icon
```

项目源码和 spike 均保持 `#![forbid(unsafe_code)]`；Windows API unsafe 封装位于经过依赖审核的第三方 crate 内。

### 4.6 Windows 11 SSH + 交互式桌面真机

测试主机：

```text
Host: 192.168.32.38
OS: Microsoft Windows 11 企业版 10.0.22621.6060
User: DESKTOP-7TN9731\\bluecake
Active desktop: console Session 1
```

SSH 会话自身为非交互环境，`[Environment]::UserInteractive=false`。直接从 SSH 启动修复后的 probe 时，winit 返回 UI 初始化错误并立即退出，没有假装 UI 可用；这符合无交互桌面失败关闭的设计。

通过一次性 `/IT` 计划任务将同一 SHA-256 的 GNU 产物启动到活动 console Session 1 后：

- 进程位于 Session 1 且 `Responding=true`；
- 用户确认看见自定义源选择窗口；
- 用户按指示完成多文件选择，并确认结果提示显示；
- 用户完成原生文件夹选择；
- 用户通过托盘“打开选择器”再次唤起窗口并取消；
- 用户通过托盘“退出”关闭进程；
- 计划任务最终结果为 0；
- SSH 复核 `ProcessCount=0`；
- 两个一次性计划任务、临时目录和进程全部清理完成。

首次真机启动暴露并修复了一个关键问题：GNU `windres` 将未定义的 `RT_MANIFEST` token 编译为**字符串资源类型**，导致 Windows 未激活 ComCtl32 v6，任务返回 `0xC0000139`（`TaskDialogIndirect` 入口点不存在）。资源脚本已改为数值类型 `24`；修复后 PE 显示 `Type ID 0x18`，真机正常启动。该回归必须在生产构建测试中保留。

远端命令输出按 GB18030 → UTF-8 转码后记录，中文 Windows 信息无乱码。

## 5. 待完成的 MSVC 原生构建门

优先将 MSVC 原生构建测试包复制到装有 Rust 1.92 MSVC toolchain 的 Windows 11 并解压：

```text
spikes/target/x86_64-pc-windows-gnu/release/windows-desktop-msvc-source.zip
SHA-256: 8e1ded34f48b5080e49fb0fd482d4b1fb408f00ad2fe92966348df73693258d1
```

在交互式 Windows 11 PowerShell 中运行：

```powershell
Set-ExecutionPolicy -Scope Process Bypass
cd .\qs-windows-desktop-msvc-source
.\scripts\windows-msvc-build-and-test.ps1 `
  -Report .\windows-desktop-true-host.json
```

该脚本强制检查 Rust host 为 `pc-windows-msvc`，执行 locked test、Clippy、release build，然后启动真机交互清单。

如果 Windows 没有 Rust，可先使用预编译 GNU 运行包验证 UI，但这不能单独解除 MSVC 门：

```text
spikes/target/x86_64-pc-windows-gnu/release/windows-desktop-true-host.zip
SHA-256: f41511db45c862b7b7b2f1dd934c22555b8405c6b611574763df9a4ebeca57cf
```

解压后运行：

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\windows-true-host.ps1 `
  -Binary .\qs-windows-desktop-spike.exe `
  -Report .\windows-desktop-true-host.json
```

Windows 交互清单已完成。剩余缺口只有 MSVC 原生 test/Clippy/release build。可选择：

1. 在安装 Rust 1.92 MSVC + Build Tools 的 Windows 主机运行上述 source package；或
2. 明确批准 Conditional Go，把 MSVC native build 作为 T-007 合入生产 UI 前的硬门，同时继续保留 T-010 Windows 真机总验收。

## 6. 当前限制

- Windows 主机当前没有 Rust/Cargo、Visual Studio、`cl.exe` 或 MSVC Build Tools；用户已批准把 MSVC native build 延期为 T-007 合入前硬门；
- 本批只验证 GNU 产物，尚未验证 MSVC 原生产物；
- 文件选择器由用户按指示完成多文件操作，但未自动记录具体文件名或路径，以避免收集本地隐私；
- 锁屏状态尚未验证；该项在生产 T-007/T-010 继续保留；
- SSH 无 UI 时 spike 输出明确初始化诊断但进程状态码仍为 0；生产 T-007 必须映射为 `UiUnavailable`/非成功状态，不得复制该退出语义；
- spike 不是产品实现，不执行真实文件传输。
