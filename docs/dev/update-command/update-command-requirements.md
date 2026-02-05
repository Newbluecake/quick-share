---
feature: update-command
complexity: standard
generated_by: clarify
generated_at: 2026-01-27T10:30:00+08:00
version: 1
---

# 需求文档: Update Command

> **功能标识**: update-command
> **复杂度**: standard
> **生成方式**: clarify
> **生成时间**: 2026-01-27

## 1. 概述

### 1.1 一句话描述
为 quick-share 提供 `update` 子命令，让用户可以方便地检查和更新到最新版本。

### 1.2 核心价值
- 用户无需手动访问 GitHub 或记住 pip 命令即可更新
- 智能选择更新源（GitHub Releases / pip），适配不同安装方式
- 全平台支持（Linux、macOS、Windows）

### 1.3 目标用户
- **主要用户**：已安装 quick-share 的终端用户

---

## 2. 需求与用户故事

### 2.1 需求清单

| ID | 需求点 | 优先级 | 用户故事 |
|----|--------|--------|----------|
| R-001 | 检查新版本 | P0 | As a user, I want to check if a new version is available, so that I know whether to update |
| R-002 | 执行更新 | P0 | As a user, I want to update to the latest version with one command, so that I don't need to remember installation steps |
| R-003 | 显示更新日志 | P1 | As a user, I want to see what changed between versions, so that I can decide whether to update |
| R-004 | 失败回滚 | P1 | As a user, I want automatic rollback if update fails, so that I don't break my installation |
| R-005 | 向后兼容 | P0 | As a user, I want `quick-share file.txt` to continue working, so that I don't need to change my workflow |

### 2.2 验收标准

#### R-001: 检查新版本
- **WHEN** 用户执行 `quick-share update --check`, **THEN** 系统 **SHALL** 显示当前版本和最新版本对比
- **WHEN** 已是最新版本, **THEN** 系统 **SHALL** 提示 "Already up to date"

#### R-002: 执行更新
- **WHEN** 用户执行 `quick-share update`, **THEN** 系统 **SHALL** 检查版本并询问用户确认
- **WHEN** 用户确认更新, **THEN** 系统 **SHALL** 下载并安装新版本
- **WHEN** 更新完成, **THEN** 系统 **SHALL** 显示新版本号

#### R-003: 显示更新日志
- **WHEN** 检测到新版本, **THEN** 系统 **SHALL** 显示版本间的 changelog 差异

#### R-004: 失败回滚
- **WHEN** 更新过程失败, **THEN** 系统 **SHALL** 自动回滚到之前版本
- **WHEN** 回滚完成, **THEN** 系统 **SHALL** 提示用户更新失败原因

#### R-005: 向后兼容
- **WHEN** 用户执行 `quick-share document.pdf`, **THEN** 系统 **SHALL** 正常启动文件分享
- **WHEN** 用户执行 `quick-share update`, **THEN** 系统 **SHALL** 进入更新流程

---

## 3. 功能验收清单

| ID | 功能点 | 验收步骤 | 优先级 | 关联需求 | 通过 |
|----|--------|----------|--------|----------|------|
| F-001 | 版本检查 | 1. 执行 `quick-share update --check` 2. 显示版本对比信息 | P0 | R-001 | ☐ |
| F-002 | 交互确认 | 1. 执行 `quick-share update` 2. 显示版本差异并询问 [Y/n] | P0 | R-002 | ☐ |
| F-003 | GitHub 更新 | 1. 在 exe 安装环境执行更新 2. 从 Releases 下载新版本 | P0 | R-002 | ☐ |
| F-004 | pip 更新 | 1. 在 pip 安装环境执行更新 2. 使用 pip install --upgrade 更新 | P0 | R-002 | ☐ |
| F-005 | Changelog 展示 | 1. 检测到新版本 2. 显示 CHANGELOG.md 差异 | P1 | R-003 | ☐ |
| F-006 | 回滚机制 | 1. 模拟更新失败 2. 验证自动回滚成功 | P1 | R-004 | ☐ |
| F-007 | 向后兼容 | 1. 执行 `quick-share file.txt` 2. 正常启动分享 | P0 | R-005 | ☐ |

---

## 4. 技术约束

### 4.1 技术栈
- Python 3.8+
- argparse（需改造为支持子命令）
- requests（获取 GitHub API）

### 4.2 更新源智能选择

| 安装方式 | 检测方法 | 更新方式 |
|---------|---------|---------|
| pip 安装 | `pip show quick-share` 成功 | `pip install --upgrade` |
| exe 安装 | Windows + 无 pip 信息 | GitHub Releases 下载 |
| 源码安装 | git 仓库存在 | `git pull` + `pip install` |

### 4.3 集成点
- GitHub API: `https://api.github.com/repos/Newbluecake/quick-share/releases/latest`
- GitHub Releases: 下载 exe/tar.gz 资产

---

## 5. 排除项

- **自动更新**：不在后台自动检查更新，避免安全风险
- **降级功能**：不支持安装指定旧版本
- **代理配置**：不提供专门的代理设置，使用系统代理

---

## 6. 下一步

✅ 在新会话中执行：
```bash
/clouditera:dev:spec-dev update-command --skip-requirements
```
