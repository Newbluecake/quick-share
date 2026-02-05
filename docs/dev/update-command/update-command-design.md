---
feature: update-command
phase: design
generated_at: 2026-01-27
version: 1
---

# 技术设计文档: Update Command

## 1. 设计概述

### 1.1 架构决策

**CLI 架构改造**：采用「隐式子命令」模式保持向后兼容

```text
quick-share <path>           → 文件/目录分享（现有行为）
quick-share update [options] → 更新命令（新增）
quick-share --version        → 版本信息（保持）
```

**检测逻辑**：
1. 如果第一个位置参数是 `update`，进入更新流程
2. 否则，按现有逻辑处理为文件/目录路径

### 1.2 模块结构

```text
src/
├── __init__.py          # 版本号 (__version__)
├── cli.py               # CLI 解析（需改造）
├── main.py              # 主入口（需改造）
├── updater.py           # 【新增】更新逻辑模块
└── ...
```

---

## 2. 详细设计

### 2.1 CLI 改造 (cli.py)

**改造策略**：不使用 argparse subparsers（避免破坏向后兼容），而是在解析前预处理参数。

```python
def is_update_command(args: list) -> bool:
    """检测是否是 update 命令"""
    if not args:
        return False
    # 排除 --version, --help 等
    first_arg = args[0] if args else None
    return first_arg == 'update'

def parse_update_arguments(args: list) -> argparse.Namespace:
    """解析 update 子命令参数"""
    parser = argparse.ArgumentParser(
        prog='quick-share update',
        description='Check and update quick-share to the latest version'
    )
    parser.add_argument('--check', action='store_true',
                        help='Only check for updates, do not install')
    parser.add_argument('-y', '--yes', action='store_true',
                        help='Skip confirmation prompt')
    return parser.parse_args(args[1:])  # 跳过 'update'
```

### 2.2 更新模块 (updater.py)

#### 2.2.1 核心类设计

```python
class Updater:
    """更新管理器"""

    GITHUB_API = "https://api.github.com/repos/Newbluecake/quick-share/releases/latest"
    GITHUB_REPO = "https://github.com/Newbluecake/quick-share"

    def __init__(self):
        self.current_version = __version__
        self.install_method = self._detect_install_method()

    def _detect_install_method(self) -> str:
        """检测安装方式: pip | exe | source"""
        ...

    def check_update(self) -> tuple[bool, str, str]:
        """检查更新，返回 (has_update, latest_version, changelog)"""
        ...

    def do_update(self, skip_confirm: bool = False) -> bool:
        """执行更新，返回成功与否"""
        ...

    def rollback(self) -> bool:
        """回滚到之前版本"""
        ...
```

#### 2.2.2 安装方式检测

| 安装方式 | 检测条件 | 更新策略 |
|---------|---------|---------|
| `pip` | `pip show quick-share` 成功 | `pip install --upgrade git+...` |
| `exe` | Windows + 无 pip 信息 + exe 存在 | 下载新 exe 替换 |
| `source` | 存在 `.git` 目录 | 提示手动 git pull |

```python
def _detect_install_method(self) -> str:
    # 1. 尝试 pip
    try:
        result = subprocess.run(
            [sys.executable, '-m', 'pip', 'show', 'quick-share'],
            capture_output=True, text=True
        )
        if result.returncode == 0:
            return 'pip'
    except Exception:
        pass

    # 2. 检测 exe (Windows)
    if sys.platform == 'win32' and getattr(sys, 'frozen', False):
        return 'exe'

    # 3. 检测源码安装
    script_dir = Path(__file__).parent.parent
    if (script_dir / '.git').exists():
        return 'source'

    # 4. 默认尝试 pip
    return 'pip'
```

#### 2.2.3 版本检查

```python
def check_update(self) -> tuple[bool, str, str]:
    """
    Returns:
        has_update: 是否有新版本
        latest_version: 最新版本号
        changelog: 版本变更说明
    """
    try:
        import urllib.request
        import json

        req = urllib.request.Request(
            self.GITHUB_API,
            headers={'User-Agent': f'quick-share/{self.current_version}'}
        )
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read().decode())

        latest = data['tag_name'].lstrip('v')
        changelog = data.get('body', '')

        has_update = self._compare_versions(latest, self.current_version) > 0
        return has_update, latest, changelog

    except Exception as e:
        raise UpdateCheckError(f"Failed to check for updates: {e}")
```

#### 2.2.4 更新执行

**pip 更新**：
```python
def _update_pip(self) -> bool:
    cmd = [
        sys.executable, '-m', 'pip', 'install', '--upgrade',
        f'git+{self.GITHUB_REPO}.git'
    ]
    result = subprocess.run(cmd, capture_output=True, text=True)
    return result.returncode == 0
```

**exe 更新**（Windows）：
```python
def _update_exe(self, download_url: str) -> bool:
    # 1. 备份当前 exe
    current_exe = Path(sys.executable)
    backup_exe = current_exe.with_suffix('.exe.bak')
    shutil.copy2(current_exe, backup_exe)

    # 2. 下载新版本到临时文件
    temp_exe = current_exe.with_suffix('.exe.new')
    urllib.request.urlretrieve(download_url, temp_exe)

    # 3. 替换（需要批处理脚本延迟执行）
    self._schedule_replace(temp_exe, current_exe, backup_exe)
    return True
```

#### 2.2.5 回滚机制

```python
def rollback(self) -> bool:
    """回滚到备份版本"""
    if self.install_method == 'exe':
        backup = Path(sys.executable).with_suffix('.exe.bak')
        if backup.exists():
            # Windows: 创建批处理脚本在进程退出后替换
            self._schedule_replace(backup, Path(sys.executable))
            return True
    elif self.install_method == 'pip':
        # pip 回滚：重新安装之前版本
        # 需要在更新前保存当前版本号
        if self._previous_version:
            cmd = [
                sys.executable, '-m', 'pip', 'install',
                f'quick-share=={self._previous_version}'
            ]
            result = subprocess.run(cmd, capture_output=True)
            return result.returncode == 0
    return False
```

### 2.3 主入口改造 (main.py)

```python
def main() -> None:
    # 预处理：检测是否是 update 命令
    if len(sys.argv) > 1 and sys.argv[1] == 'update':
        from .updater import run_update
        sys.exit(run_update(sys.argv[1:]))

    # 原有逻辑保持不变
    try:
        args = parse_arguments()
        ...
```

---

## 3. 用户交互设计

### 3.1 版本检查输出

```text
$ quick-share update --check

Quick Share Update Check
━━━━━━━━━━━━━━━━━━━━━━━━━
Current version: 1.0.12
Latest version:  1.0.15

What's new in 1.0.15:
  - Added update command
  - Fixed memory leak in directory sharing
  - Improved Windows compatibility

Run 'quick-share update' to install the update.
```

### 3.2 更新确认流程

```text
$ quick-share update

Quick Share Update
━━━━━━━━━━━━━━━━━━
Current version: 1.0.12
Latest version:  1.0.15

What's new:
  - Added update command
  - Fixed memory leak in directory sharing

Do you want to update? [Y/n]: y

Updating... ████████████████████ 100%

✅ Successfully updated to 1.0.15!
```

### 3.3 已是最新版本

```text
$ quick-share update

✅ Already up to date (1.0.15)
```

### 3.4 更新失败回滚

```text
$ quick-share update

Updating... ████████████░░░░░░░░ 60%

❌ Update failed: Network error

Rolling back to 1.0.12...
✅ Rollback successful. Previous version restored.
```

---

## 4. 错误处理

| 错误场景 | 处理策略 |
|---------|---------|
| 网络不可用 | 提示检查网络连接，退出码 1 |
| GitHub API 限流 | 提示稍后重试，显示限流重置时间 |
| 权限不足 | 提示使用 sudo/管理员权限 |
| 下载中断 | 自动重试 3 次，失败后提示 |
| 安装失败 | 触发回滚，恢复原版本 |

---

## 5. 测试策略

### 5.1 单元测试

- `test_updater.py`：测试 Updater 类各方法
  - 版本比较逻辑
  - 安装方式检测
  - Mock GitHub API 响应

### 5.2 集成测试

- 模拟完整更新流程（使用 mock）
- 测试向后兼容性（`quick-share file.txt` 仍工作）

---

## 6. 依赖说明

**无新增外部依赖**：使用 Python 标准库
- `urllib.request`：HTTP 请求
- `json`：解析 API 响应
- `subprocess`：执行 pip 命令
- `shutil`：文件操作

---

## 7. 安全考虑

1. **HTTPS 强制**：所有 GitHub 请求使用 HTTPS
2. **版本验证**：下载后验证文件完整性（可选：SHA256 校验）
3. **权限最小化**：exe 更新使用用户目录临时文件
4. **无自动更新**：仅用户主动触发，避免供应链攻击风险
