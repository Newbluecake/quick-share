# 技术设计文档: README 更新与安装脚本

> **生成时间**: 2026-01-12
> **需求文档**: readme-and-install-requirements.md
> **状态**: Draft

## 1. 架构概览

### 1.1 系统架构图

```
+-------------------+     +----------------------+     +------------------+
|   GitHub Releases |     |    Install Scripts   |     |   User System    |
|-------------------|     |----------------------|     |------------------|
| - quick-share-    |<----|  install.sh (Unix)   |---->| /usr/local/bin/  |
|   linux-amd64     |     |  install.ps1 (Win)   |     | ~/.local/bin/    |
| - quick-share-    |     +----------------------+     | AppData/Local/   |
|   linux-arm64     |              ^                   +------------------+
| - quick-share-    |              |
|   macos-amd64     |     +----------------------+
| - quick-share-    |     |      README.md       |
|   macos-arm64     |     |----------------------|
| - quick-share-    |     | - Quick Start        |
|   windows.exe     |     | - Install Commands   |
+-------------------+     | - Usage Examples     |
                          +----------------------+
```

### 1.2 数据流

```
用户执行一键安装命令
         |
         v
+------------------+
| 下载安装脚本     |  curl -fsSL .../install.sh | bash
+------------------+
         |
         v
+------------------+
| 检测平台/架构    |  uname -s, uname -m / $env:PROCESSOR_ARCHITECTURE
+------------------+
         |
         v
+------------------+
| 构造下载 URL     |  GitHub Releases latest redirect
+------------------+
         |
         v
+------------------+
| 下载二进制文件   |  curl / Invoke-WebRequest
+------------------+
         |
         v
+------------------+
| 安装到目标目录   |  /usr/local/bin 或 ~/.local/bin / AppData
+------------------+
         |
         v
+------------------+
| 配置 PATH        |  提示或自动添加
+------------------+
         |
         v
+------------------+
| 验证安装         |  quick-share --version
+------------------+
```

## 2. 组件设计

### 2.1 新增组件

| 组件 | 文件路径 | 职责 |
|------|----------|------|
| install.sh | /install.sh | Unix 平台安装脚本 |
| install.ps1 | /install.ps1 | Windows 平台安装脚本 |
| README.md | /README.md | 项目文档（重构） |

### 2.2 修改组件

| 组件 | 修改内容 |
|------|----------|
| README.md | 完全重构，新增快速开始、安装指南、使用示例 |

## 3. 接口设计

### 3.1 GitHub Releases URL 规范

```
# 最新版本下载（使用 GitHub 重定向，避免 API 调用）
https://github.com/Newbluecake/quick-share/releases/latest/download/{binary_name}

# 二进制文件命名规范
quick-share-linux-amd64      # Linux x86_64
quick-share-linux-arm64      # Linux ARM64
quick-share-macos-amd64      # macOS Intel
quick-share-macos-arm64      # macOS Apple Silicon
quick-share-windows.exe      # Windows x64
```

### 3.2 install.sh 接口

```bash
# 用法
curl -fsSL https://raw.githubusercontent.com/Newbluecake/quick-share/main/install.sh | bash

# 环境变量（可选）
INSTALL_DIR=/custom/path    # 自定义安装目录
VERSION=v1.0.0              # 指定版本（默认 latest）

# 退出码
0 - 安装成功
1 - 平台不支持
2 - 下载失败
3 - 安装失败（权限等）
```

### 3.3 install.ps1 接口

```powershell
# 用法
irm https://raw.githubusercontent.com/Newbluecake/quick-share/main/install.ps1 | iex

# 参数（可选）
-InstallDir "C:\custom\path"    # 自定义安装目录
-Version "v1.0.0"               # 指定版本

# 退出码
0 - 安装成功
1 - 下载失败
2 - PATH 配置失败
```

## 4. 详细设计

### 4.1 install.sh 结构

```bash
#!/bin/bash
set -e

# === 常量定义 ===
REPO="Newbluecake/quick-share"
INSTALL_DIR_SYSTEM="/usr/local/bin"
INSTALL_DIR_USER="$HOME/.local/bin"
BINARY_NAME="quick-share"

# === 函数定义 ===

# 检测操作系统
detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        *)       echo "unsupported" ;;
    esac
}

# 检测架构
detect_arch() {
    case "$(uname -m)" in
        x86_64)  echo "amd64" ;;
        aarch64) echo "arm64" ;;
        arm64)   echo "arm64" ;;
        *)       echo "unsupported" ;;
    esac
}

# 构造下载 URL
get_download_url() {
    local os=$1
    local arch=$2
    echo "https://github.com/${REPO}/releases/latest/download/quick-share-${os}-${arch}"
}

# 选择安装目录
select_install_dir() {
    if [ -w "$INSTALL_DIR_SYSTEM" ]; then
        echo "$INSTALL_DIR_SYSTEM"
    else
        mkdir -p "$INSTALL_DIR_USER"
        echo "$INSTALL_DIR_USER"
    fi
}

# 下载并安装
install_binary() {
    local url=$1
    local dest=$2

    echo "Downloading from: $url"
    curl -fsSL "$url" -o "$dest"
    chmod +x "$dest"
}

# 验证安装
verify_installation() {
    local binary=$1
    if "$binary" --version >/dev/null 2>&1; then
        echo "Installation successful!"
        "$binary" --version
    else
        echo "Installation verification failed"
        exit 3
    fi
}

# 检查 PATH
check_path() {
    local dir=$1
    if [[ ":$PATH:" != *":$dir:"* ]]; then
        echo ""
        echo "NOTE: $dir is not in your PATH."
        echo "Add this line to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
        echo "  export PATH=\"\$PATH:$dir\""
    fi
}

# === 主流程 ===
main() {
    echo "Quick Share Installer"
    echo "====================="

    local os=$(detect_os)
    local arch=$(detect_arch)

    if [ "$os" = "unsupported" ] || [ "$arch" = "unsupported" ]; then
        echo "Error: Unsupported platform: $(uname -s) $(uname -m)"
        exit 1
    fi

    local url=$(get_download_url "$os" "$arch")
    local install_dir=$(select_install_dir)
    local dest="$install_dir/$BINARY_NAME"

    echo "Platform: $os-$arch"
    echo "Install to: $dest"

    install_binary "$url" "$dest"
    verify_installation "$dest"
    check_path "$install_dir"
}

main "$@"
```

### 4.2 install.ps1 结构

```powershell
#Requires -Version 5.1
<#
.SYNOPSIS
    Quick Share Windows Installer
.DESCRIPTION
    Downloads and installs Quick Share from GitHub Releases
#>

param(
    [string]$InstallDir = "$env:LOCALAPPDATA\quick-share",
    [string]$Version = "latest"
)

$ErrorActionPreference = "Stop"

# === 常量 ===
$Repo = "Newbluecake/quick-share"
$BinaryName = "quick-share.exe"

# === 函数 ===

function Get-DownloadUrl {
    param([string]$Ver)
    if ($Ver -eq "latest") {
        return "https://github.com/$Repo/releases/latest/download/quick-share-windows.exe"
    }
    return "https://github.com/$Repo/releases/download/$Ver/quick-share-windows.exe"
}

function Install-Binary {
    param(
        [string]$Url,
        [string]$Destination
    )

    Write-Host "Downloading from: $Url"
    Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
}

function Add-ToPath {
    param([string]$Dir)

    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -notlike "*$Dir*") {
        $newPath = "$currentPath;$Dir"
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        Write-Host "Added $Dir to User PATH"
        return $true
    }
    Write-Host "$Dir is already in PATH"
    return $false
}

function Test-Installation {
    param([string]$Binary)

    try {
        $version = & $Binary --version 2>&1
        Write-Host "Installation successful!"
        Write-Host $version
        return $true
    } catch {
        Write-Host "Installation verification failed"
        return $false
    }
}

# === 主流程 ===
function Main {
    Write-Host "Quick Share Installer for Windows"
    Write-Host "================================="

    # 创建安装目录
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    $url = Get-DownloadUrl -Ver $Version
    $dest = Join-Path $InstallDir $BinaryName

    Write-Host "Install to: $dest"

    # 下载
    Install-Binary -Url $url -Destination $dest

    # 添加到 PATH
    $pathChanged = Add-ToPath -Dir $InstallDir

    # 验证
    if (-not (Test-Installation -Binary $dest)) {
        exit 1
    }

    if ($pathChanged) {
        Write-Host ""
        Write-Host "IMPORTANT: Please restart your terminal for PATH changes to take effect."
    }
}

Main
```

### 4.3 README.md 结构

```markdown
# Quick Share

> 一行命令启动 HTTP 服务，快速分享文件

## 快速开始

### 一键安装

**Linux / macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/Newbluecake/quick-share/main/install.sh | bash
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/Newbluecake/quick-share/main/install.ps1 | iex
```

### 基本用法
```bash
quick-share myfile.zip
```

## 功能特性

- 自动检测局域网 IP
- 智能端口选择（冲突自动递增）
- 安全限制（仅允许指定文件）
- 自动停止（下载次数/时间限制）
- 实时日志

## 安装方式

### 方式 1: 一键安装（推荐）
[详细说明...]

### 方式 2: pip 安装
```bash
pip install quick-share
```

### 方式 3: 手动下载
[GitHub Releases 链接...]

## 使用示例

### 基础用法
```bash
# 分享文件（默认：10次下载或5分钟后停止）
quick-share file.zip

# 自定义配置
quick-share file.zip -n 3 -t 10m -p 9000
```

### 命令行选项
| 选项 | 说明 | 默认值 |
|------|------|--------|
| -n | 最大下载次数 | 10 |
| -t | 最大运行时间 | 5m |
| -p | 端口号 | 8000 |

## 使用场景

- 服务器间快速传输文件
- 临时分享日志给同事
- 局域网设备间共享

## 开发

[开发相关信息...]

## License

MIT
```

## 5. 技术选型

| 决策点 | 选型 | 理由 |
|--------|------|------|
| Shell 兼容性 | Bash (非 POSIX sh) | macOS 和主流 Linux 均预装 Bash，功能更丰富 |
| PowerShell 版本 | 5.1+ | Windows 10/11 默认版本，无需额外安装 |
| 下载工具 | curl (Unix) / Invoke-WebRequest (Windows) | 系统内置，无需额外依赖 |
| GitHub API | 不使用 | 避免速率限制，使用 releases/latest/download 重定向 |
| 安装目录 (Unix) | /usr/local/bin 优先，~/.local/bin 备选 | 符合 FHS 标准，无 sudo 时使用用户目录 |
| 安装目录 (Windows) | %LOCALAPPDATA%\quick-share | 标准用户应用目录，无需管理员权限 |

## 6. 安全考量

| 风险 | 缓解措施 |
|------|----------|
| 中间人攻击 | 使用 HTTPS 下载；未来可添加 SHA256 校验 |
| 恶意脚本执行 | 脚本托管在官方仓库，用户可先下载查看 |
| 权限提升 | 优先使用用户目录，仅必要时请求 sudo |
| PATH 污染 | 仅添加到用户 PATH，不修改系统 PATH |

## 7. 测试策略

### 7.1 测试层级

| 层级 | 范围 | 方法 |
|------|------|------|
| Unit | 脚本函数 | Bash: bats-core; PS: Pester |
| Integration | 完整安装流程 | Docker 容器 / GitHub Actions |
| E2E | 真实平台验证 | 手动测试 matrix |

### 7.2 测试文件规划

```
tests/
  install/
    test_install_sh.bats      # install.sh 单元测试
    test_install_ps1.Tests.ps1 # install.ps1 单元测试
    Dockerfile.ubuntu         # Ubuntu 测试环境
    Dockerfile.alpine         # Alpine 测试环境
```

### 7.3 测试用例

**install.sh:**
- test_detect_os_linux: Linux 平台检测
- test_detect_os_macos: macOS 平台检测
- test_detect_arch_amd64: x86_64 架构检测
- test_detect_arch_arm64: ARM64 架构检测
- test_download_url_construction: URL 构造正确性
- test_install_dir_selection: 安装目录选择逻辑
- test_path_check: PATH 检查逻辑

**install.ps1:**
- Test-DownloadUrlConstruction: URL 构造
- Test-InstallDirCreation: 目录创建
- Test-PathUpdate: PATH 更新
- Test-VerifyInstallation: 安装验证

## 8. 迁移计划

### 8.1 阶段划分

| 阶段 | 内容 | 风险 |
|------|------|------|
| Phase 1 | 创建 install.sh/install.ps1 | 低 - 新增文件 |
| Phase 2 | 重构 README.md | 低 - 文档变更 |
| Phase 3 | 集成测试 | 中 - 需要真实环境 |

### 8.2 回滚策略

- 所有变更在 feature 分支进行
- README 保留旧版本在 git 历史
- 安装脚本出问题不影响现有 pip 安装方式

---

**文档版本**: v1.0.0
**最后更新**: 2026-01-12
