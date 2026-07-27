---
feature: rust-rewrite
batch: 0
status: approved
recommendation: conditional-go
generated_at: 2026-07-26T10:48:00+08:00
approved_at: 2026-07-26T10:58:51+08:00
---

# Batch 0 可行性验证：Go/No-Go 报告

## 1. 结论

**用户已批准继续：Conditional Go，允许进入 T-001 正式 workspace，但不得将 Spike 代码复制到生产实现。**

五项核心技术均找到可实施路径，并在 Linux、Debian、Windows 11 的组合中获得真实进程/双机证据。仍有 macOS、Windows 同链路 mDNS、非管理员符号链接和正式密码学安全评审等条件，必须保留为后续硬闸门。

## 2. 测试环境

| 机器 | 系统 | 架构 | 地址/用途 |
|---|---|---|---|
| 开发 Worktree | Linux 5.15 | x86_64 | `192.168.31.25`，构建与 Linux 测试 |
| Linux 远端 | Debian 13 / kernel 6.12 | x86_64 | `192.168.31.14`，双机发现/Noise/Web |
| Windows 远端 | Windows 11 Enterprise 10.0.22621 | AMD64 | `192.168.32.38`，原生运行/剪贴板/恢复/服务端 |

工具链：Rust 1.92.0；Windows 测试程序由 `x86_64-pc-windows-gnu` 交叉编译，目标机无需 Rust 运行时。

## 3. 验证汇总

### SP-001：身份协议

**建议：Conditional Go，选择 Noise XX。**

已验证：

- `Noise_XX_25519_ChaChaPoly_BLAKE2s` 双方 SAS 一致；
- 双方取得对方完整 static public key；
- Linux↔Linux、Windows→Linux 加密 ping/pong；
- 持久 key 重启后 pinning 保持；
- 错误 pin 失败关闭；
- 终止式 MITM 产生不同 SAS；
- frame 上限单测；
- `snow` crate 禁止 unsafe，2026 年仍有维护提交。

关键条件：

- `snow 0.10.0` README 明确声明未接受正式安全审计；
- 发布前必须专项安全评审、RustSec 审计、fuzz、replay/truncation/rekey 测试；
- 使用标准 Noise pattern，不自行发明密码原语；
- 长期信任绑定完整 static key，6 位 SAS 只用于人工比较。

拟议 ADR：`rust-rewrite-design.md` ADR-009。

### SP-002：发现协议

**建议：Conditional Go，使用 mDNS/DNS-SD，但必须显式选择接口。**

已验证：

- Linux↔Debian 同一 `/24` 双向发现：约 408ms / 1069ms；
- Windows 11 原生 advertise/scan：约 69ms；
- Linux/Windows 显式 `--ip` 均只发布正确主地址；
- 不同 `/24` 不发生 mDNS 发现，符合 link-local 语义；
- 跨子网 `--peer` + Noise TCP 仍可直连。

关键发现：

- `enable_addr_auto()` 在 Linux 返回最多 126 个虚拟/错误 scoped IPv6 地址；
- Windows 自动模式发布 VMware `192.168.240.1`，遗漏主地址 `192.168.32.38`；
- 生产实现必须枚举、过滤并显式注册每个 LAN IP，发现地址在安全握手前保持 unverified。

待验证：Windows↔Linux 同链路；macOS。

### SP-003：传统 Web TLS

**结论：Go，用户已批准策略 D。**

已验证：

- Linux 和 Windows 原生 rustls HTTPS server；
- Linux 双向 curl；
- Windows 浏览器显示自签名警告后成功访问；
- SAN 包含 advertise IP；
- 普通 curl 退出 60，显式 insecure 可访问；
- 用户选择默认自签名 HTTPS、支持用户证书、HTTP 只允许显式 `--allow-http`。

Windows 防火墙发现：

- Windows 当前网络为 Public profile；
- Windows 本机访问 HTTPS server 成功；
- Linux 访问 Windows receiver/Web 端口超时；
- Windows Noise server 入站同样超时；
- 系统没有 Quick Share inbound rule。

生产要求：检测 profile/入站不可达；不静默关闭防火墙；可选规则必须经用户同意，限定程序和 Private profile，并支持卸载清理。

### SP-004：恢复一致性

**建议：Conditional Go。**

已验证：

- 7 个 Rust tests；
- chunk digest、乱序、重复幂等、冲突拒绝、损坏 journal 失败关闭；
- Linux 256 MiB 在 5/64 块后 `kill -9`，重启只补 59 块并完成；
- Windows 256 MiB 在 5/64 块后 `Stop-Process -Force`，重启完成；
- Windows tempfile persist/final rename 可行；
- Windows 中文和空格目标路径 kill/resume 通过；
- final 在完整校验前不可见。

边界：保证单文件不暴露伪完整 final；目录任务不能在所有平台保证整体原子出现。

待验证：Windows 文件被其他程序占用、磁盘满、真实断电、1/10 GiB 和大量小文件。

### SP-005：剪贴板与单文件

**建议：Windows/Linux Go，macOS/X11/Wayland Conditional。**

已验证：

- Linux headless 无 DISPLAY/Wayland 时结构化失败并可降级；
- Debian headless 同样行为；
- Windows 11 原生 clipboard create/write/read；
- Windows 中文 + emoji round-trip；
- Linux/Windows release 单文件无需 Rust 运行时；
- Windows 原生 `.exe` 约 2.3 MiB。

待验证：Linux 图形桌面、Wayland clipboard 生命周期、macOS、musl。

## 4. 额外跨平台发现

### Windows 私钥 ACL

Spike key 继承 ACL，只允许 SYSTEM、Administrators 和当前 Bluecake 用户，未包含普通 Users，但 `AreAccessRulesProtected=false`。生产 IdentityStore 必须创建受保护 ACL，不能假设父目录永远安全。

### Windows 符号链接

当前 SSH 用户是 elevated administrator，符号链接创建和读取通过。该结果不能代表普通用户；无管理员/无 Developer Mode 时必须测试并安全降级。

### Windows 输出编码

`ipconfig/certutil/tasklist` 中文使用 OEM/GBK，而 Agent 终端按 UTF-8 解码，导致日志乱码。PowerShell 设置 UTF-8 后正常。Rust Spike 的 UTF-8 JSON 未受影响。生产平台命令应避免解析本地化文本，优先使用 Win32 API/PowerShell object JSON。

## 5. 质量验证

- Python 基线：339 tests passed；
- Rust Spike tests：18 tests passed（identity 5、discovery 2、resume 7、clipboard 2、Web TLS 2）；
- `cargo clippy -- -D warnings`：通过；
- Linux release binaries：可运行；
- Windows GNU cross-check/build：可运行于真实 Windows 11；
- `git diff --check`：纳入最终 Batch 验证；
- Spike 源码与报告均在 `spikes/`、`docs/dev/rust-rewrite/spikes/`。

## 6. 进入正式实施的条件

批准 Conditional Go 后，可以执行 T-001，但必须将以下条件写入对应任务完成定义：

1. **T-003/T-009**：Noise key 权限、受保护 Windows ACL、安全评审；
2. **T-006**：禁止 `enable_addr_auto()`，显式接口注册；Windows/Linux 同链路与 macOS 测试；
3. **T-008**：剪贴板不可用绝不阻塞文件传输；补 X11/Wayland/macOS；
4. **T-009**：snow/Noise 专项安全评审和 fuzz；
5. **T-014/T-017**：Windows Public profile/inbound firewall 诊断；
6. **T-022**：防火墙规则仅显式可选、最小权限、可卸载；
7. **T-024**：macOS 真机、非管理员 Windows、同链路 Windows mDNS、移动二维码作为发布闸门。

任何条件未满足时可以继续独立任务，但不得宣称对应平台/安全能力完成，也不得发布 v2.0.0。

## 7. 闸门选项

- **批准 Conditional Go**：进入 Batch 1 / T-001，创建正式 Rust workspace；
- **修改条件**：调整 ADR、任务或安全/平台要求后重新确认；
- **No-Go**：保留报告和 Spike，停止正式重写。
