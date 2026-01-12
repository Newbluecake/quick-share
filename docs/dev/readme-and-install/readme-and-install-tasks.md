# 任务清单: README 更新与安装脚本

> **生成时间**: 2026-01-12
> **设计文档**: readme-and-install-design.md
> **需求文档**: readme-and-install-requirements.md

## 任务总览

| ID | 任务 | 复杂度 | 依赖 | 并行组 | 执行方式 | 模型 |
|----|------|--------|------|--------|----------|------|
| T-001 | 创建 install.sh 安装脚本 | medium | - | 1 | agent | sonnet |
| T-002 | 创建 install.ps1 安装脚本 | medium | - | 1 | agent | sonnet |
| T-003 | 重构 README.md | medium | T-001, T-002 | 2 | agent | sonnet |
| T-004 | 创建安装脚本测试 | medium | T-001, T-002 | 2 | agent | sonnet |

## 依赖关系图

```
并行组 1:
  T-001 (install.sh) ──┬──> T-003 (README.md)
  T-002 (install.ps1) ─┴──> T-004 (测试)

并行组 2:
  T-003 (README.md)
  T-004 (测试)
```

## 执行计划

```
Phase 1 (并行组 1):
  ├── T-001: install.sh [agent, sonnet]
  └── T-002: install.ps1 [agent, sonnet]

Phase 2 (并行组 2):
  ├── T-003: README.md [agent, sonnet]
  └── T-004: 测试 [agent, sonnet]
```

---

## 任务详情

---

## T-001: 创建 install.sh 安装脚本

- **complexity**: medium (6/10)
- **review_strategy**: combined
- **parallel_group**: 1
- **execution**: agent
- **model**: sonnet
- **estimated_files**: 1
- **dependencies**: 无

### 描述

创建 Linux/macOS 平台的一键安装脚本。脚本需要自动检测操作系统和 CPU 架构，从 GitHub Releases 下载对应的二进制文件，安装到合适的目录，并处理 PATH 配置。

### 技术要点

1. **平台检测**: 使用 `uname -s` 检测 OS (Linux/Darwin)，`uname -m` 检测架构 (x86_64/aarch64/arm64)
2. **下载策略**: 使用 GitHub releases/latest/download 重定向 URL，避免 API 调用
3. **安装目录**: 优先 `/usr/local/bin`（需写权限），备选 `~/.local/bin`
4. **错误处理**: 使用 `set -e`，定义明确的退出码

### TDD 要求

**先写测试，再实现功能**

测试文件: `tests/install/test_install_sh.bats`

```bash
# 测试用例规划
@test "detect_os returns linux on Linux" { ... }
@test "detect_os returns macos on Darwin" { ... }
@test "detect_arch returns amd64 for x86_64" { ... }
@test "detect_arch returns arm64 for aarch64" { ... }
@test "get_download_url constructs correct URL" { ... }
@test "select_install_dir prefers /usr/local/bin when writable" { ... }
@test "select_install_dir falls back to ~/.local/bin" { ... }
```

### 验收标准

- [ ] 脚本文件 `/install.sh` 存在且可执行
- [ ] 在 Linux x86_64 环境执行时，检测结果为 `linux-amd64`
- [ ] 在 macOS ARM64 环境执行时，检测结果为 `macos-arm64`
- [ ] 下载 URL 格式正确: `https://github.com/Newbluecake/quick-share/releases/latest/download/quick-share-{os}-{arch}`
- [ ] 无 sudo 权限时自动切换到 `~/.local/bin`
- [ ] 安装目录不在 PATH 时输出提示信息
- [ ] 安装成功后执行 `quick-share --version` 验证
- [ ] 测试文件存在且所有测试通过

### 涉及文件

- /home/bluecake/ai/chat/quick-share/install.sh (新建)
- /home/bluecake/ai/chat/quick-share/tests/install/test_install_sh.bats (新建)

---

## T-002: 创建 install.ps1 安装脚本

- **complexity**: medium (6/10)
- **review_strategy**: combined
- **parallel_group**: 1
- **execution**: agent
- **model**: sonnet
- **estimated_files**: 1
- **dependencies**: 无

### 描述

创建 Windows 平台的 PowerShell 安装脚本。脚本需要从 GitHub Releases 下载 Windows 二进制文件，安装到用户的 AppData 目录，并自动配置用户 PATH 环境变量。

### 技术要点

1. **PowerShell 版本**: 兼容 5.1+（Windows 10/11 默认）
2. **下载方式**: 使用 `Invoke-WebRequest` 的 `-UseBasicParsing` 参数
3. **安装目录**: `$env:LOCALAPPDATA\quick-share`
4. **PATH 配置**: 修改用户级 PATH 环境变量，使用 `[Environment]::SetEnvironmentVariable`

### TDD 要求

**先写测试，再实现功能**

测试文件: `tests/install/test_install_ps1.Tests.ps1`

```powershell
# 测试用例规划 (Pester 框架)
Describe "Get-DownloadUrl" {
    It "returns latest URL when version is latest" { ... }
    It "returns versioned URL when version specified" { ... }
}

Describe "Add-ToPath" {
    It "adds directory to PATH when not present" { ... }
    It "skips when directory already in PATH" { ... }
}
```

### 验收标准

- [ ] 脚本文件 `/install.ps1` 存在
- [ ] 脚本兼容 PowerShell 5.1+
- [ ] 下载 URL 正确: `https://github.com/Newbluecake/quick-share/releases/latest/download/quick-share-windows.exe`
- [ ] 安装到 `$env:LOCALAPPDATA\quick-share\quick-share.exe`
- [ ] 自动将安装目录添加到用户 PATH
- [ ] PATH 已存在时不重复添加
- [ ] 安装成功后提示重启终端
- [ ] 测试文件存在且所有测试通过

### 涉及文件

- /home/bluecake/ai/chat/quick-share/install.ps1 (新建)
- /home/bluecake/ai/chat/quick-share/tests/install/test_install_ps1.Tests.ps1 (新建)

---

## T-003: 重构 README.md

- **complexity**: medium (5/10)
- **review_strategy**: combined
- **parallel_group**: 2
- **execution**: agent
- **model**: sonnet
- **estimated_files**: 1
- **dependencies**: T-001, T-002

### 描述

重构项目 README.md 文档，使其对非 Python 用户更加友好。需要突出一键安装命令，提供清晰的多平台安装指南，并包含使用示例。

### 技术要点

1. **结构优先级**: 快速开始 > 功能特性 > 详细安装 > 使用示例 > 开发信息
2. **一键安装命令**: 放在首屏最显眼位置
3. **多平台支持**: Linux/macOS/Windows 分别说明
4. **保留现有内容**: 功能特性、使用场景等内容保留并优化

### TDD 要求

**文档类任务采用验收测试**

验收检查脚本: `tests/install/test_readme.sh`

```bash
# 验收检查
check_readme_has_quick_start()      # 检查包含快速开始章节
check_readme_has_install_command()  # 检查包含一键安装命令
check_readme_has_multi_platform()   # 检查包含多平台说明
check_readme_has_usage_examples()   # 检查包含使用示例
```

### 验收标准

- [ ] README.md 第一屏包含"快速开始"标题
- [ ] README.md 包含 Linux/macOS 的 curl 一键安装命令
- [ ] README.md 包含 Windows 的 PowerShell 一键安装命令
- [ ] README.md 包含功能特性列表
- [ ] README.md 包含至少 3 种安装方式（一键安装、pip、手动下载）
- [ ] README.md 包含命令行选项说明表格
- [ ] README.md 包含使用示例代码块

### 涉及文件

- /home/bluecake/ai/chat/quick-share/README.md (修改)
- /home/bluecake/ai/chat/quick-share/tests/install/test_readme.sh (新建)

---

## T-004: 创建安装脚本集成测试

- **complexity**: medium (5/10)
- **review_strategy**: combined
- **parallel_group**: 2
- **execution**: agent
- **model**: sonnet
- **estimated_files**: 3
- **dependencies**: T-001, T-002

### 描述

创建安装脚本的集成测试环境，包括 Docker 测试容器和测试脚本，确保安装脚本在真实环境中工作正常。

### 技术要点

1. **Docker 测试**: 使用 Ubuntu 和 Alpine 镜像测试 install.sh
2. **Mock 策略**: 测试时 mock GitHub 下载，使用本地测试二进制
3. **CI 集成**: 测试可在 GitHub Actions 中运行

### TDD 要求

**测试文件本身需要可运行验证**

```bash
# 运行测试
./tests/install/run_integration_tests.sh
```

### 验收标准

- [ ] Dockerfile.ubuntu 存在且可构建
- [ ] Dockerfile.alpine 存在且可构建
- [ ] 集成测试脚本 run_integration_tests.sh 存在
- [ ] 测试覆盖 install.sh 的完整安装流程
- [ ] 测试可在 CI 环境运行

### 涉及文件

- /home/bluecake/ai/chat/quick-share/tests/install/Dockerfile.ubuntu (新建)
- /home/bluecake/ai/chat/quick-share/tests/install/Dockerfile.alpine (新建)
- /home/bluecake/ai/chat/quick-share/tests/install/run_integration_tests.sh (新建)

---

## 风险评估

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|----------|
| GitHub Releases 尚未配置 | 高 | 中 | 测试时使用 mock URL，文档说明前置依赖 |
| 不同 shell 兼容性问题 | 中 | 中 | 测试覆盖 bash/zsh/sh |
| Windows 权限问题 | 中 | 低 | 使用用户目录，避免系统目录 |
| ARM64 平台测试困难 | 中 | 低 | CI 使用 QEMU 模拟或标记为手动测试 |

---

## 前置条件

在执行这些任务之前，请确保：

1. **GitHub Releases 已配置**: 需要有 `quick-share-linux-amd64`、`quick-share-macos-arm64`、`quick-share-windows.exe` 等二进制文件
2. **仓库 URL 正确**: 确认 `Newbluecake/quick-share` 是正确的仓库路径

如果 GitHub Releases 尚未配置，T-001 和 T-002 的测试需要使用 mock 模式。

---

## 执行命令

```bash
# 并行执行 Phase 1
/dev:task-execute T-001 --parallel &
/dev:task-execute T-002 --parallel &
wait

# 并行执行 Phase 2
/dev:task-execute T-003 --parallel &
/dev:task-execute T-004 --parallel &
wait
```

---

**文档版本**: v1.0.0
**最后更新**: 2026-01-12
