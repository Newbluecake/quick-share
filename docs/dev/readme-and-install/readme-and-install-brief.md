# readme-and-install - 需求简报

> **生成时间**: 2026-01-12
> **访谈轮次**: 第 3 轮
> **细节级别**: standard
> **版本**: v1.0.0 Draft

## 1. 一句话描述

更新 README.md 提供更完善的文档，并提供一键自动安装脚本（Linux/macOS/Windows），支持从 GitHub Releases 下载预编译的二进制文件，无需 Python 环境。

## 2. 目标用户

- **主要用户**：非 Python 开发者、运维人员、希望快速使用的普通用户
- **次要用户**：开发者（仍可通过 pip/source 安装）

## 3. 核心场景（Top 3）

### 场景 1: Linux/macOS 一键安装
- **触发**: 用户在终端执行 `curl -sSL https://.../install.sh | bash`
- **结果**: 脚本自动检测系统，下载对应二进制文件，安装到 PATH，命令即刻可用。

### 场景 2: Windows 一键安装
- **触发**: 用户在 PowerShell 执行 `iwr -useb https://.../install.ps1 | iex`
- **结果**: 脚本自动下载 .exe 文件，添加到 PATH，命令即刻可用。

### 场景 3: 快速查阅文档
- **触发**: 用户打开 GitHub 首页或查看 README
- **结果**: 5秒内看懂如何安装、如何分享文件、常用参数有哪些。

## 4. 关键需求点

**必须有 (P0)**:
- [ ] 全新的 README.md（结构清晰，含快速开始、功能特性、多平台安装指南）
- [ ] `install.sh`: Linux/macOS 通用安装脚本
- [ ] `install.ps1`: Windows 安装脚本
- [ ] 安装脚本需支持从 GitHub Releases 下载最新版本

**应该有 (P1)**:
- [ ] 脚本自动检测系统架构 (amd64/arm64)
- [ ] 脚本自动配置 PATH 环境变量

## 5. 明确不做

- [ ] 不自行搭建分发服务器（完全依赖 GitHub Releases）
- [ ] 不支持包管理器（apt/brew/choco）发布（本次仅做脚本安装）

## 6. 关键风险

- **GitHub API 限制** → 使用 Public URL 下载 asset，尽量避免依赖 API token。
- **PATH 配置生效** → 提示用户 `source ~/.bashrc` 或重启终端。

## 7. 下一步

✅ **确认 Brief 后，查看完整 Requirements**:
`docs/dev/readme-and-install/readme-and-install-requirements.md`

✅ **在新会话中执行**:
```bash
/dev:spec-dev readme-and-install --skip-requirements
```
