---
feature: update-command
phase: tasks
generated_at: 2026-01-27
version: 1
---

# 任务清单: Update Command

## 任务概览

| ID | 任务 | 优先级 | 预计复杂度 | 依赖 |
|----|------|--------|-----------|------|
| T-001 | 创建 updater.py 核心模块 | P0 | Medium | - |
| T-002 | 改造 cli.py 支持 update 子命令 | P0 | Low | - |
| T-003 | 改造 main.py 入口分发 | P0 | Low | T-002 |
| T-004 | 实现版本检查功能 | P0 | Medium | T-001 |
| T-005 | 实现 pip 更新策略 | P0 | Medium | T-001, T-004 |
| T-006 | 实现 exe 更新策略 (Windows) | P1 | High | T-001, T-004 |
| T-007 | 实现回滚机制 | P1 | Medium | T-005, T-006 |
| T-008 | 添加单元测试 | P0 | Medium | T-001~T-005 |
| T-009 | 更新文档和 CHANGELOG | P1 | Low | T-008 |

## 并行分组

```text
Group 1 (并行): T-001, T-002
Group 2 (串行): T-003 (依赖 T-002)
Group 3 (并行): T-004, T-005 (依赖 T-001)
Group 4 (串行): T-006, T-007 (依赖 T-005)
Group 5 (串行): T-008 (依赖 T-001~T-005)
Group 6 (串行): T-009 (依赖 T-008)
```

---

## 详细任务

### T-001: 创建 updater.py 核心模块

**目标**: 创建更新功能的核心模块框架

**验收标准**:
- [ ] 创建 `src/updater.py` 文件
- [ ] 实现 `Updater` 类基础结构
- [ ] 实现 `_detect_install_method()` 方法
- [ ] 实现 `_compare_versions()` 版本比较方法
- [ ] 定义异常类 `UpdateError`, `UpdateCheckError`

**实现要点**:
```python
# src/updater.py
class UpdateError(Exception): pass
class UpdateCheckError(UpdateError): pass

class Updater:
    GITHUB_API = "https://api.github.com/repos/Newbluecake/quick-share/releases/latest"
    GITHUB_REPO = "https://github.com/Newbluecake/quick-share"

    def __init__(self):
        from . import __version__
        self.current_version = __version__
        self.install_method = self._detect_install_method()
        self._previous_version = None

    def _detect_install_method(self) -> str: ...
    def _compare_versions(self, v1: str, v2: str) -> int: ...
```

---

### T-002: 改造 cli.py 支持 update 子命令

**目标**: 添加 update 命令参数解析，保持向后兼容

**验收标准**:
- [ ] 添加 `is_update_command()` 函数
- [ ] 添加 `parse_update_arguments()` 函数
- [ ] 支持 `--check` 参数
- [ ] 支持 `-y/--yes` 参数
- [ ] 现有 `parse_arguments()` 不受影响

**实现要点**:
```python
def is_update_command(args: list = None) -> bool:
    """检测是否是 update 命令"""
    import sys
    args = args if args is not None else sys.argv[1:]
    if not args:
        return False
    return args[0] == 'update'

def parse_update_arguments(args: list = None):
    """解析 update 子命令参数"""
    import sys
    args = args if args is not None else sys.argv[1:]
    parser = argparse.ArgumentParser(
        prog='quick-share update',
        description='Check and update quick-share to the latest version'
    )
    parser.add_argument('--check', action='store_true',
                        help='Only check for updates, do not install')
    parser.add_argument('-y', '--yes', action='store_true',
                        help='Skip confirmation prompt')
    # 跳过 'update' 本身
    return parser.parse_args(args[1:] if args and args[0] == 'update' else args)
```

---

### T-003: 改造 main.py 入口分发

**目标**: 在 main() 入口添加 update 命令分发逻辑

**验收标准**:
- [ ] 在 main() 开头添加 update 命令检测
- [ ] 调用 updater 模块处理更新
- [ ] 保持原有文件分享逻辑不变
- [ ] 测试 `quick-share update` 进入更新流程
- [ ] 测试 `quick-share file.txt` 仍正常工作

**实现要点**:
```python
def main() -> None:
    from .cli import is_update_command

    # 检测 update 命令
    if is_update_command():
        from .updater import run_update
        sys.exit(run_update())

    # 原有逻辑保持不变
    try:
        args = parse_arguments()
        ...
```

---

### T-004: 实现版本检查功能

**目标**: 实现从 GitHub API 获取最新版本信息

**验收标准**:
- [ ] 实现 `check_update()` 方法
- [ ] 返回 `(has_update, latest_version, changelog)`
- [ ] 处理网络超时（10秒）
- [ ] 处理 API 限流错误
- [ ] 实现 `--check` 模式的输出格式

**实现要点**:
```python
def check_update(self) -> tuple:
    """返回 (has_update, latest_version, changelog)"""
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
```

---

### T-005: 实现 pip 更新策略

**目标**: 实现通过 pip 更新的逻辑

**验收标准**:
- [ ] 实现 `_update_pip()` 方法
- [ ] 保存当前版本用于回滚
- [ ] 显示更新进度
- [ ] 处理权限不足错误
- [ ] 实现用户确认交互

**实现要点**:
```python
def _update_pip(self) -> bool:
    import subprocess
    self._previous_version = self.current_version

    cmd = [
        sys.executable, '-m', 'pip', 'install', '--upgrade',
        f'git+{self.GITHUB_REPO}.git'
    ]
    print("Updating via pip...")
    result = subprocess.run(cmd, capture_output=False)
    return result.returncode == 0

def do_update(self, skip_confirm: bool = False) -> bool:
    has_update, latest, changelog = self.check_update()
    if not has_update:
        print(f"✅ Already up to date ({self.current_version})")
        return True

    # 显示更新信息
    print(f"\nCurrent version: {self.current_version}")
    print(f"Latest version:  {latest}")
    if changelog:
        print(f"\nWhat's new:\n{changelog[:500]}")

    # 确认
    if not skip_confirm:
        confirm = input("\nDo you want to update? [Y/n]: ").strip().lower()
        if confirm and confirm != 'y':
            print("Update cancelled.")
            return False

    # 执行更新
    if self.install_method == 'pip':
        return self._update_pip()
    elif self.install_method == 'exe':
        return self._update_exe(latest)
    else:
        print("Source installation detected. Please run 'git pull' manually.")
        return False
```

---

### T-006: 实现 exe 更新策略 (Windows)

**目标**: 实现 Windows exe 文件的自更新

**验收标准**:
- [ ] 实现 `_update_exe()` 方法
- [ ] 从 GitHub Releases 下载 exe
- [ ] 备份当前 exe
- [ ] 使用批处理脚本延迟替换
- [ ] 处理 UAC 权限问题

**实现要点**:
```python
def _update_exe(self, latest_version: str) -> bool:
    import urllib.request
    from pathlib import Path

    # 获取下载 URL
    download_url = f"{self.GITHUB_REPO}/releases/download/v{latest_version}/quick-share.exe"

    current_exe = Path(sys.executable)
    backup_exe = current_exe.with_suffix('.exe.bak')
    temp_exe = current_exe.parent / 'quick-share.exe.new'

    # 备份
    shutil.copy2(current_exe, backup_exe)

    # 下载
    print(f"Downloading v{latest_version}...")
    urllib.request.urlretrieve(download_url, temp_exe)

    # 创建替换脚本
    self._create_replace_script(temp_exe, current_exe, backup_exe)
    print("✅ Update downloaded. Restart to complete installation.")
    return True
```

---

### T-007: 实现回滚机制

**目标**: 更新失败时自动回滚

**验收标准**:
- [ ] 实现 `rollback()` 方法
- [ ] pip 安装：重新安装旧版本
- [ ] exe 安装：恢复备份文件
- [ ] 更新失败时自动触发回滚

**实现要点**:
```python
def rollback(self) -> bool:
    print("Rolling back to previous version...")

    if self.install_method == 'pip' and self._previous_version:
        cmd = [
            sys.executable, '-m', 'pip', 'install',
            f'git+{self.GITHUB_REPO}.git@v{self._previous_version}'
        ]
        result = subprocess.run(cmd, capture_output=True)
        if result.returncode == 0:
            print(f"✅ Rollback successful. Restored to {self._previous_version}")
            return True

    elif self.install_method == 'exe':
        backup = Path(sys.executable).with_suffix('.exe.bak')
        if backup.exists():
            # 延迟替换脚本
            self._create_replace_script(backup, Path(sys.executable))
            print("✅ Rollback scheduled. Restart to complete.")
            return True

    print("❌ Rollback failed.")
    return False
```

---

### T-008: 添加单元测试

**目标**: 为更新功能添加完整测试

**验收标准**:
- [ ] 创建 `tests/test_updater.py`
- [ ] 测试版本比较逻辑
- [ ] 测试安装方式检测
- [ ] Mock GitHub API 测试版本检查
- [ ] 测试 CLI 参数解析
- [ ] 测试向后兼容性

**测试用例**:
```python
class TestUpdater:
    def test_compare_versions(self):
        updater = Updater()
        assert updater._compare_versions("1.0.1", "1.0.0") > 0
        assert updater._compare_versions("1.0.0", "1.0.1") < 0
        assert updater._compare_versions("1.0.0", "1.0.0") == 0

    def test_detect_install_method(self):
        # 根据环境测试

    @patch('urllib.request.urlopen')
    def test_check_update(self, mock_urlopen):
        # Mock API 响应

class TestUpdateCLI:
    def test_is_update_command(self):
        assert is_update_command(['update']) == True
        assert is_update_command(['update', '--check']) == True
        assert is_update_command(['file.txt']) == False
        assert is_update_command([]) == False

    def test_backward_compatibility(self):
        # 确保 quick-share file.txt 仍工作
```

---

### T-009: 更新文档和 CHANGELOG

**目标**: 更新用户文档和变更日志

**验收标准**:
- [ ] 更新 README.md 添加 update 命令说明
- [ ] 更新 CHANGELOG.md 添加新版本记录
- [ ] 更新 --help 输出（如需要）

**README 更新内容**:
```markdown
### Update Quick Share

Check for updates:
```bash
quick-share update --check
```

Update to latest version:
```bash
quick-share update
```
```

---

## 执行计划

**推荐执行顺序**:

1. **Phase 1** (并行): T-001 + T-002
2. **Phase 2**: T-003
3. **Phase 3** (并行): T-004 + T-005
4. **Phase 4**: T-006 (可选，仅 Windows)
5. **Phase 5**: T-007
6. **Phase 6**: T-008
7. **Phase 7**: T-009

**预计总工时**: 3-4 小时（不含 T-006 的 Windows 测试）
