# 需求文档: README 更新与安装脚本

> **生成方式**: 由 /dev:clarify 自动生成
> **生成时间**: 2026-01-12
> **访谈轮次**: 第 3 轮
> **细节级别**: standard
> **可直接用于**: /dev:spec-dev readme-and-install --skip-requirements

## 1. 介绍

本项目旨在降低 Quick Share 的使用门槛。目前项目仅通过 Python 源码或 pip 运行，对于非 Python 用户不够友好。本需求将重构 README.md 文档，并提供跨平台的一键安装脚本（Shell & PowerShell），允许用户直接从 GitHub Releases 下载并安装预编译的二进制文件，无需配置 Python 环境。

**目标用户**:
- 主要用户：系统管理员、运维工程师、非 Python 开发人员
- 次要用户：希望在纯净环境（无 Python）使用的用户

**核心价值**:
- **零依赖安装**: 无需 Python 环境，下载即用
- **极致便捷**: 一行命令完成下载、安装、配置
- **文档清晰**: 结构化的文档降低学习成本

## 2. 需求与用户故事

### 需求 1: 重构 README.md
**用户故事:** As a 新用户, I want 一个结构清晰、重点突出的 README, so that 我能在一分钟内学会安装和使用。

#### 验收标准（可测试）
- **WHEN** 用户查看 README, **THEN** 第一屏应显示“快速开始”和“一键安装命令”。
- **WHEN** 用户查看功能列表, **THEN** 应清晰列出核心特性（IP检测、端口选择、安全限制等）。
- **WHEN** 用户需要安装, **THEN** 应提供 Linux/macOS/Windows 三种平台的独立选项卡或段落。
- **WHEN** 用户查看使用示例, **THEN** 应包含截图（或 ASCII 演示）和常见命令示例。

### 需求 2: Linux/macOS 自动安装脚本 (install.sh)
**用户故事:** As a Linux/macOS 用户, I want 通过一行 curl 命令安装工具, so that 我不需要手动下载和配置 PATH。

#### 验收标准（可测试）
- **WHEN** 执行脚本, **THEN** 自动检测操作系统（Linux/Darwin）和架构（x86_64/arm64）。
- **WHEN** 检测完成, **THEN** 从 GitHub Releases 下载对应的最新版二进制文件（`quick-share-linux` 或 `quick-share-macos`）。
- **WHEN** 下载完成, **THEN** 将文件安装到 `/usr/local/bin/quick-share` (需要 sudo) 或 `~/.local/bin/quick-share`。
- **IF** 安装目录不在 PATH 中, **THEN** 提示用户如何添加。
- **WHEN** 安装成功, **THEN** 运行 `quick-share --version` 验证并显示成功信息。

### 需求 3: Windows 自动安装脚本 (install.ps1)
**用户故事:** As a Windows 用户, I want 通过一行 PowerShell 命令安装工具, so that 我不需要手动下载和配置环境变量。

#### 验收标准（可测试）
- **WHEN** 执行脚本, **THEN** 从 GitHub Releases 下载 `quick-share-windows.exe`。
- **WHEN** 下载完成, **THEN** 将文件移动到用户的 Local AppData 目录（如 `~/AppData/Local/quick-share/`）。
- **WHEN** 移动完成, **THEN** 将该目录添加到用户的 User PATH 环境变量（如果尚未添加）。
- **WHEN** 安装成功, **THEN** 提示用户重启终端以生效。

## 3. 测试映射表

| 验收条目 | 测试层级 | 预期测试文件 | 预期函数/用例 |
|----------|----------|--------------|---------------|
| 需求2: 脚本下载逻辑 | integration | tests/install/test_install_sh.sh | test_download_url_construction |
| 需求2: 权限处理 | integration | tests/install/test_install_sh.sh | test_install_permissions |
| 需求3: Windows下载 | integration | tests/install/test_install_ps1.ps1 | Test-Download |
| 需求3: PATH配置 | integration | tests/install/test_install_ps1.ps1 | Test-Path-Update |

## 4. 功能验收清单

| ID | 功能点 | 验收步骤 | 优先级 | 关联任务 | 通过 |
|----|--------|----------|--------|----------|------|
| F-001 | README: 快速开始区 | 1. 打开 README 2. 确认首屏有一键安装命令 | P0 | T-003 | ✅ |
| F-002 | README: 多平台指南 | 1. 确认包含 Linux/macOS/Windows 安装说明 | P0 | T-003 | ✅ |
| F-003 | install.sh: 平台检测 | 1. 在 Linux x86_64 运行 2. 确认下载 linux-amd64 包 | P0 | T-001 | ✅ |
| F-004 | install.sh: 文件安装 | 1. 运行脚本 2. 验证 /usr/local/bin/quick-share 存在且可执行 | P0 | T-001 | ✅ |
| F-005 | install.ps1: 文件下载 | 1. 运行 PS 脚本 2. 确认 exe 下载成功 | P0 | T-002 | ✅ |
| F-006 | install.ps1: PATH配置 | 1. 运行 PS 脚本 2. 确认 User PATH 包含安装目录 | P0 | T-002 | ✅ |

## 5. 技术约束与要求

### 5.1 技术栈
- **Shell**: Bash (兼容 POSIX sh 优先)
- **PowerShell**: PowerShell 5.1+ (兼容 Windows 10/11 默认环境)
- **文档**: Markdown

### 5.2 集成点
- **GitHub Releases**: 脚本需构造正确的 URL 下载 Assets
  - URL 模板: `https://github.com/{user}/{repo}/releases/download/{tag}/{binary_name}`
  - 获取最新 Tag: `https://api.github.com/repos/{user}/{repo}/releases/latest` (解析 JSON) 或使用 `latest` 重定向 URL

### 5.3 性能要求
- **脚本执行时间**: 取决于网络，逻辑处理 < 1s

### 5.4 安全要求
- **校验**: (可选) 验证下载文件的 SHA256 Checksum
- **权限**: 脚本应最小权限运行，仅在移动到系统目录时请求 sudo

## 6. 排除项（明确不做）

- **包管理器发布**: 本次不发布到 npm, apt, brew, choco, scoop，仅提供直接脚本安装。
- **自动升级**: 脚本仅负责安装/覆盖，不包含自升级命令（update command）。

## 7. 风险与挑战

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| GitHub API 速率限制 | 中 | 脚本尽量使用 `releases/latest/download/` 的固定 URL 模式，避免频繁调用 API |
| 网络连接 GitHub 慢 | 高 | 无法完全解决，可考虑提供加速镜像 URL 选项（Future Feature） |
| Windows PATH 生效延迟 | 中 | 脚本明确提示用户重启终端 |

## 8. 相关文档

- **简报版本**: docs/dev/readme-and-install/readme-and-install-brief.md
- **当前 README**: README.md

## 9. 下一步行动

### 方式1：使用 spec-dev 继续（推荐）

在新会话中执行：
```bash
/dev:spec-dev readme-and-install --skip-requirements
```

这将：
1. ✅ 跳过阶段1（requirements 已完成）
2. 🎯 直接进入阶段2（技术设计）
3. 📋 然后阶段3（任务拆分）
4. 💻 最后 TDD 实施
