# Quick Share

**简体中文** | [English](README.en.md)

[![CI](https://github.com/Newbluecake/quick-share/actions/workflows/ci.yml/badge.svg)](https://github.com/Newbluecake/quick-share/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Quick Share 是一款面向 Linux、macOS 和 Windows 的安全单二进制局域网共享命令行工具。它可以发现附近的接收端，并通过经过身份认证的 Noise XX 加密传输文件、目录、文本或剪贴板内容。如果扫描成功但未发现兼容的接收端，它还可以改为启动面向浏览器的 HTTPS 共享。

Quick Share 2.0 是用于生产环境的 Rust 实现，取代了 Python 1.x 运行时和对等传输协议。升级前请参阅[迁移指南](docs/migration-v2.md)。

## 安装

无需安装 Python、pip、Node.js 或其他语言运行时。

### Linux 和 macOS

```bash
curl -fsSL https://raw.githubusercontent.com/Newbluecake/quick-share/master/install.sh | bash
```

安装程序会从固定的 GitHub 仓库下载一个可执行文件，校验 `SHA256SUMS`；当 OpenSSL 支持时，还会校验固定的 Ed25519 发布签名。随后程序会安装到 `~/.local/bin`，且仅在名称未被占用时创建 `sc`/`rc` 快捷命令。

如需在执行前检查安装脚本：

```bash
curl -fsSLO https://raw.githubusercontent.com/Newbluecake/quick-share/master/install.sh
less install.sh
bash install.sh
```

### Windows PowerShell

```powershell
iwr -useb https://raw.githubusercontent.com/Newbluecake/quick-share/master/install.ps1 | iex
```

默认安装目录为 `%LOCALAPPDATA%\QuickShare\bin`。已有的 `sc` 或 `rc` 命令不会被覆盖。只有在安装程序显式传入 `-AddPrivateFirewallRule` 时，才会添加一条仅适用于专用网络配置文件、作用范围限定到本程序的入站防火墙规则。

预构建产物、`SHA256SUMS`、`SHA256SUMS.sig`、SBOM 和 GitHub 构建来源证明均发布在 [GitHub Releases](https://github.com/Newbluecake/quick-share/releases)。

## 快速开始

在接收端计算机上：

```bash
quick-share receive
# 安装时未发生名称冲突的情况下，可使用快捷命令：
rc
```

在发送端计算机上：

```bash
quick-share send report.pdf photos/
# 快捷命令：
sc report.pdf photos/
```

首次连接未知设备时，两端会显示一个六位 SAS（短认证字符串）。请比较两个终端上的数字，确认一致后再选择“接受并信任”。`--yes` 只允许一次性 TOFU（首次使用时信任），不会创建持久信任关系。

## 命令

### 发送文件和目录

```bash
quick-share send file.txt directory/
quick-share send --peer 192.168.1.20:4242 file.txt
quick-share send --resume 018f47b4-5b3a-7c9d-8123-0123456789ab --peer 192.168.1.20:4242 file.txt
quick-share send --web file.txt
quick-share send --follow-links symlink
```

仅当发现流程成功完成且找到的兼容接收端数量为零时，才会自动回退到 Web 模式。发现错误、部分扫描、拒绝、超时、身份变更、连接失败和传输失败都不会触发 Web 回退。

### 发送文本或剪贴板内容

```bash
quick-share send --text 'hello from Quick Share'
quick-share send --clipboard
```

接收到的文本绝不会被执行或打开。如果未显式指定输出文件，接收端会尝试写入系统原生剪贴板，并在失败时安全地回退到标准输出。

### 接收

```bash
quick-share receive
quick-share receive --output ~/Downloads/received
quick-share receive --once --yes
```

除非提供 `--yes`，否则非交互模式下来自未知设备的传输请求会被拒绝；即使提供 `--yes`，也只能接受一次。第一次按下 Ctrl+C 会保存可恢复状态，第二次中断则强制终止。

### 跨设备远程选择（Windows 桌面）

第一阶段的原生桌面选择器仅在 Windows 上提供。在 Windows 上启动常驻代理，它会在系统托盘运行，并在收到请求时弹出原生窗口：

```powershell
quick-share agent
quick-share agent --bind 192.168.1.20 --port 4242
```

- 代理需要交互式 Windows 桌面会话；没有可用桌面时请求会失败关闭，不会静默继续。
- 托盘菜单提供“Open Quick Share”和“Exit”；关闭隐藏的宿主窗口不会退出代理。
- 已配对设备直接进入选择；未配对设备先弹出授权窗口（显示设备名、稳定 ID 和验证码），身份变化的设备默认被拒绝。

Linux 端从命令行请求 Windows 选择要发送的内容，并安全接收回连传输：

```bash
quick-share receive --request
quick-share receive --request --peer 192.168.1.20:4242
quick-share receive --request --peer 192.168.1.20:4242 --output ~/Downloads/received
```

- 自动发现只会列出协商支持远程选择的 Windows 代理；非交互模式下必须用 `--peer` 指定目标。
- Windows 上默认使用该设备上次保存的目录，并提供“更改目录”；仅在完整信任身份时才记忆目录。
- 目标出现同名内容时，Windows 会提示“覆盖 / 跳过 / 重命名”和“仅此项 / 应用到全部”。
- 回连只连接已认证控制连接观测到的来源 IP 并固定完整远端公钥，且精确校验请求与传输标识。

### 传统浏览器共享

```bash
quick-share serve file.txt directory/
quick-share serve --upload --output ~/Downloads/received
QUICK_SHARE_UPLOAD_PASSWORD='choose-a-password' \
  quick-share serve --upload --output ~/Downloads/received
```

Web 模式默认使用临时自签名 HTTPS 证书。终端会显示实际监听地址、带令牌的 URL、证书指纹、二维码、过期时间和下载次数限制。浏览器会对临时证书发出警告；请勿将其安装为证书颁发机构（CA）。

使用明文 HTTP 必须显式选择：

```bash
quick-share serve --allow-http file.txt
```

### 受信任设备

```bash
quick-share devices list
quick-share devices list --json
quick-share devices rename <DEVICE_ID> laptop
quick-share devices remove <DEVICE_ID>
```

信任关系固定到完整的、经过身份认证的静态公钥，而不是 IP 地址、设备名称、mDNS 记录或简短 SAS。

### 配置

```bash
quick-share config show
quick-share config path
quick-share config set device.name workstation
quick-share config set receive.output ~/Downloads/received
```

配置优先级依次为：命令行、环境变量、TOML 文件、内置默认值。身份信息与信任状态单独存储，并使用私有权限保护。

### 签名自更新

```bash
quick-share update --check
quick-share update
quick-share update --yes
quick-share update --version 2.0.0
```

更新只会从 `Newbluecake/quick-share` 获取；流式下载有大小限制，并使用签名的 SHA-256 清单和固定的 Ed25519 发布密钥进行校验。更新程序还会执行启动检查，并以具备回滚保护的方式替换程序。指向明文协议或外部主机的重定向会被拒绝。

## 安全模型

- 直接传输固定使用 `Noise_XX_25519_ChaChaPoly_BLAKE2s`，并固定完整静态密钥。
- 未知对等端在获批前只能提交有大小限制的传输请求；传输操作需要绑定到对等端的短期授权。
- 文件使用 BLAKE3 进行分块及最终完整性校验，并采用暂存区、持久化日志和原子提交。
- Web 模式使用随机 128 位访问令牌、与路径无关的目录项 ID、规范路径包含性检查、严格限制，以及 `no-store`/`no-referrer` 响应头，并默认启用 HTTPS。
- 产品 crate 禁止使用不安全 Rust。依赖项通过 RustSec 检查，协议帧在 CI 中进行模糊测试。
- 发布更新程序在二进制中固定 Ed25519 公钥。私钥仅作为受保护的 GitHub Actions Secret `RELEASE_SIGNING_KEY_PEM` 存在。

有关漏洞报告方式，请参阅 [`SECURITY.md`](SECURITY.md)；有关 v1 迁移与回滚指导，请参阅 [`docs/migration-v2.md`](docs/migration-v2.md)。

## 构建和测试

环境要求：

- Rust 1.92.0，由 `rust-toolchain.toml` 固定
- Rust 依赖项所需的平台 C 工具链

```bash
git clone https://github.com/Newbluecake/quick-share.git
cd quick-share
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo build --workspace --release --locked
```

也可以运行：

```bash
./build.sh
```

唯一的产品可执行文件是 `target/release/quick-share`。安装程序会创建快捷命令；Cargo 不会另外构建 `sc` 或 `rc` 程序。

开发与评审要求请参阅 [`CONTRIBUTING.md`](CONTRIBUTING.md)。

## 平台说明

- 发现功能使用链路本地 mDNS。对于经过路由或限制组播的网络，请使用 `--peer host:port`。
- 由于项目没有 Apple Developer 账户，当前 macOS 产物未签名且未经公证；Gatekeeper 可能要求用户显式允许。这不会削弱 Ed25519 发布清单校验。
- Quick Share 绝不会静默更改防火墙或网络配置文件设置。
- Windows 公用网络配置文件的入站规则可能阻止连接；诊断功能只会提供指导，不会修改系统。
- 每次直接发送都会输出一个传输 UUID。发送端或接收端进程中断后，可使用 `--resume UUID` 对相同内容和目标重新执行命令；接收端仅接受完全相同且已经身份认证的发送方和不可变清单，随后请求缺失的数据块。
- Windows 子进程输出会从 UTF-8、UTF-16LE 或传统 GBK 统一转换为内部 UTF-8；PowerShell 脚本会显式选择 UTF-8。文件内容和重定向的传输载荷绝不会被转码。
- 无图形界面的 Linux 环境无法访问剪贴板时，会安全地回退到标准输出或文件输出。
- 默认以符号链接形式传输符号链接；必须显式提供 `--follow-links` 才会跟随链接。

## 许可证

MIT
