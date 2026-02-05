---
feature: symlink-handling
complexity: standard
document_type: tasks
generated_by: architect-planner
generated_at: 2025-02-05T10:20:00Z
version: 1
---

# 任务拆分文档: Symlink Handling

> **功能**: Symlink Handling
> **复杂度**: standard
> **文档版本**: v1

## 1. 任务概览

| 任务 ID | 任务名称 | 优先级 | 复杂度 | 预估时间 | 依赖 |
|---------|---------|--------|--------|----------|------|
| T-001 | 实现软链接检测函数 | P0 | simple | 20 min | - |
| T-002 | 集成到 validate_path() | P0 | simple | 10 min | T-001 |
| T-003 | 修改 main() 错误处理 | P0 | simple | 15 min | T-002 |
| T-004 | 编写单元测试 | P0 | standard | 40 min | T-003 |
| T-005 | 手动测试验收 | P0 | simple | 20 min | T-004 |

**总预估时间**: 1 小时 45 分钟

## 2. 任务分组

### 2.1 第一组：核心功能实施

**并行策略**: 串行执行（有依赖关系）

**任务列表**:
- T-001: 实现软链接检测函数
- T-002: 集成到 validate_path()
- T-003: 修改 main() 错误处理

**执行顺序**: T-001 → T-002 → T-003

### 2.2 第二组：测试与验收

**并行策略**: T-004 完成后，T-005 可以开始

**任务列表**:
- T-004: 编写单元测试
- T-005: 手动测试验收

**执行顺序**: T-004 → T-005

## 3. 详细任务定义

### T-001: 实现软链接检测函数

**优先级**: P0
**复杂度**: simple
**预估时间**: 20 分钟

**描述**:
在 `src/main.py` 中新增 `handle_symlink()` 函数，实现软链接检测、信息显示和用户确认逻辑。

**验收标准**:
- [ ] 函数签名符合设计文档：`def handle_symlink(symlink_path: str) -> Tuple[bool, str, Optional[Path]]`
- [ ] 使用 `os.path.islink()` 检测软链接
- [ ] 使用 `os.readlink()` + `Path.resolve()` 获取真实路径
- [ ] 显示软链接信息（源路径、目标路径）
- [ ] 检查目标是否存在（`resolved_path.exists()`）
- [ ] 交互式用户确认（y/n），循环直到有效输入
- [ ] 返回值符合设计文档的表格定义
- [ ] 损坏的软链接返回 `(False, "symlink_broken", None)`
- [ ] 用户取消返回 `(False, "symlink_cancelled", None)`
- [ ] 用户确认返回 `(True, "file"/"directory", Path(target))`

**实施细节**:
```python
def handle_symlink(symlink_path: str) -> Tuple[bool, str, Optional[Path]]:
    """
    Handle symlink detection and user confirmation.

    Args:
        symlink_path: The symlink path to handle.

    Returns:
        Tuple containing:
        - is_valid: True if user confirms, False otherwise
        - path_type: "file", "directory", or "symlink_broken"/"symlink_cancelled"
        - resolved_path: Real target path if confirmed, None otherwise
    """
    # 读取软链接目标
    target = os.readlink(symlink_path)
    real_path = Path(symlink_path).resolve()

    # 显示信息
    print(f"⚠️  检测到软链接：")
    print(f"    源路径: {symlink_path}")
    print(f"    目标路径: {real_path}")

    # 检查目标是否存在
    if not real_path.exists():
        print(f"❌ 错误：软链接目标不存在")
        return False, "symlink_broken", None

    # 询问用户
    while True:
        choice = input("是否跟随软链接并分享目标文件/目录？(y/n): ").strip().lower()
        if choice == 'y':
            # 递归调用 validate_path 验证真实路径
            # 注意：这里需要重新调用 validate_path 而不是 detect_path_type
            # 因为真实路径可能也是软链接（虽然罕见）
            return validate_path(str(real_path))
        elif choice == 'n':
            print("❌ 用户取消分享")
            return False, "symlink_cancelled", None
        else:
            print("请输入 y 或 n")
```

**注意事项**:
- 函数应放在 `validate_path()` 之前定义
- 使用 `os.readlink()` 读取软链接目标
- 使用 `Path.resolve()` 获取绝对路径
- 递归调用 `validate_path()` 而不是 `detect_path_type()`，确保真实路径通过完整验证

---

### T-002: 集成到 validate_path()

**优先级**: P0
**复杂度**: simple
**预估时间**: 10 分钟

**描述**:
修改 `validate_path()` 函数，在开头添加软链接检测逻辑，调用 `handle_symlink()` 函数。

**验收标准**:
- [ ] 在 `validate_path()` 函数开头添加软链接检测
- [ ] 使用 `if os.path.islink(path):` 判断
- [ ] 调用 `handle_symlink(path)` 并返回其结果
- [ ] 非软链接路径完全走原有逻辑（不受影响）
- [ ] 函数签名和返回值格式不变

**实施细节**:
```python
def validate_path(path: str) -> Tuple[bool, str, Optional[Path]]:
    """
    Unified validation for both files and directories.

    Args:
        path: Path to validate (can be file or directory).

    Returns:
        Tuple containing:
        - is_valid: True if path is valid and accessible, False otherwise
        - path_type: "file", "directory", or "invalid"
        - resolved_path: Path object if valid, None otherwise
    """
    # NEW: 检测软链接
    if os.path.islink(path):
        return handle_symlink(path)

    # 原有逻辑保持不变
    path_type = detect_path_type(path)
    # ... 其余代码不变
```

**注意事项**:
- 在 `detect_path_type()` 调用之前检测软链接
- 不修改 `detect_path_type()` 函数
- 保持原有逻辑完全不变

---

### T-003: 修改 main() 错误处理

**优先级**: P0
**复杂度**: simple
**预估时间**: 15 分钟

**描述**:
修改 `main()` 函数中的路径验证错误处理逻辑，添加对 `symlink_broken` 和 `symlink_cancelled` 的处理。

**验收标准**:
- [ ] 在 `main()` 函数的验证部分添加 `symlink_broken` 处理
- [ ] 添加 `symlink_cancelled` 处理（退出码 0，无错误消息）
- [ ] 保持现有错误处理逻辑不变
- [ ] 退出码符合设计文档（用户取消 = 0，损坏软链接 = 1）

**实施细节**:
```python
# Validate path (file or directory)
try:
    is_valid, path_type, resolved_path = validate_path(args.file_path)

    if not is_valid:
        if path_type == "symlink_broken":
            # Error message already shown by handle_symlink
            sys.exit(1)
        elif path_type == "symlink_cancelled":
            # User cancelled, no error message needed
            sys.exit(0)
        else:
            print(f"Error: Invalid path: {args.file_path}", file=sys.stderr)
            sys.exit(1)

    if path_type == "invalid":
        print(f"Error: Invalid path: {args.file_path}", file=sys.stderr)
        sys.exit(1)
except PermissionError:
    print(f"Error: Permission denied reading {args.file_path}", file=sys.stderr)
    sys.exit(1)
```

**注意事项**:
- `symlink_broken` 错误消息已在 `handle_symlink()` 中显示，这里不需要重复
- `symlink_cancelled` 应该静默退出（退出码 0）
- 保持其他错误处理逻辑不变

---

### T-004: 编写单元测试

**优先级**: P0
**复杂度**: standard
**预估时间**: 40 分钟

**描述**:
在 `tests/test_main.py` 中添加软链接相关的单元测试，覆盖所有场景。

**验收标准**:
- [ ] 测试软链接文件场景（用户输入 y）
- [ ] 测试软链接目录场景（用户输入 y）
- [ ] 测试用户取消场景（用户输入 n）
- [ ] 测试损坏的软链接场景
- [ ] 测试普通文件不受影响
- [ ] 测试普通目录不受影响
- [ ] 所有测试通过

**测试用例列表**:

```python
import os
import tempfile
from pathlib import Path
import pytest
from unittest.mock import patch
from src.main import handle_symlink, validate_path

class TestSymlinkHandling:
    """Test symlink detection and handling"""

    def test_handle_symlink_to_file(self, tmp_path, capsys):
        """Test handling symlink to file with user confirmation"""
        # Create a real file
        real_file = tmp_path / "real_file.txt"
        real_file.write_text("Hello, World!")

        # Create a symlink
        symlink = tmp_path / "symlink"
        symlink.symlink_to(real_file)

        # Mock user input to confirm
        with patch('builtins.input', return_value='y'):
            is_valid, path_type, resolved = handle_symlink(str(symlink))

        assert is_valid is True
        assert path_type == "file"
        assert resolved == real_file

        captured = capsys.readouterr()
        assert "检测到软链接" in captured.out
        assert "源路径:" in captured.out
        assert "目标路径:" in captured.out

    def test_handle_symlink_to_directory(self, tmp_path, capsys):
        """Test handling symlink to directory with user confirmation"""
        # Create a real directory
        real_dir = tmp_path / "real_dir"
        real_dir.mkdir()

        # Create a symlink
        symlink = tmp_path / "symlink_dir"
        symlink.symlink_to(real_dir)

        # Mock user input to confirm
        with patch('builtins.input', return_value='y'):
            is_valid, path_type, resolved = handle_symlink(str(symlink))

        assert is_valid is True
        assert path_type == "directory"
        assert resolved == real_dir

    def test_handle_symlink_user_cancel(self, tmp_path, capsys):
        """Test user cancels symlink following"""
        # Create a real file
        real_file = tmp_path / "real_file.txt"
        real_file.write_text("Hello")

        # Create a symlink
        symlink = tmp_path / "symlink"
        symlink.symlink_to(real_file)

        # Mock user input to cancel
        with patch('builtins.input', return_value='n'):
            is_valid, path_type, resolved = handle_symlink(str(symlink))

        assert is_valid is False
        assert path_type == "symlink_cancelled"
        assert resolved is None

        captured = capsys.readouterr()
        assert "用户取消分享" in captured.out

    def test_handle_broken_symlink(self, tmp_path, capsys):
        """Test handling broken symlink"""
        # Create a symlink to non-existent target
        symlink = tmp_path / "broken_symlink"
        symlink.symlink_to("/nonexistent/path")

        # Call handle_symlink (no user input needed)
        is_valid, path_type, resolved = handle_symlink(str(symlink))

        assert is_valid is False
        assert path_type == "symlink_broken"
        assert resolved is None

        captured = capsys.readouterr()
        assert "错误：软链接目标不存在" in captured.out

    def test_validate_path_with_symlink(self, tmp_path):
        """Test validate_path detects and handles symlink"""
        # Create a real file
        real_file = tmp_path / "real_file.txt"
        real_file.write_text("Hello")

        # Create a symlink
        symlink = tmp_path / "symlink"
        symlink.symlink_to(real_file)

        # Mock user input to confirm
        with patch('builtins.input', return_value='y'):
            is_valid, path_type, resolved = validate_path(str(symlink))

        assert is_valid is True
        assert path_type == "file"
        assert resolved == real_file

    def test_validate_path_normal_file_unaffected(self, tmp_path):
        """Test normal files are unaffected by symlink handling"""
        # Create a normal file
        normal_file = tmp_path / "normal_file.txt"
        normal_file.write_text("Hello")

        # No mock needed for input (should not prompt)
        is_valid, path_type, resolved = validate_path(str(normal_file))

        assert is_valid is True
        assert path_type == "file"
        assert resolved == normal_file

    def test_handle_symlink_invalid_input_loop(self, tmp_path):
        """Test invalid input prompts again"""
        # Create a real file
        real_file = tmp_path / "real_file.txt"
        real_file.write_text("Hello")

        # Create a symlink
        symlink = tmp_path / "symlink"
        symlink.symlink_to(real_file)

        # Mock user input: invalid -> invalid -> valid
        with patch('builtins.input', side_effect=['invalid', 'abc', 'y']):
            is_valid, path_type, resolved = handle_symlink(str(symlink))

        assert is_valid is True
        assert path_type == "file"
```

**注意事项**:
- 使用 `pytest` 的 `tmp_path` fixture 创建临时文件
- 使用 `unittest.mock.patch` 模拟用户输入
- 测试用户输入无效内容时的循环逻辑
- 测试所有返回值组合

---

### T-005: 手动测试验收

**优先级**: P0
**复杂度**: simple
**预估时间**: 20 分钟

**描述**:
手动测试软链接处理的各种场景，确保功能符合需求文档的验收标准。

**验收标准**:
- [ ] 测试场景 1：分享软链接文件，输入 y，验证分享目标文件
- [ ] 测试场景 2：分享软链接文件，输入 n，验证程序退出
- [ ] 测试场景 3：分享软链接目录，输入 y，验证分享目标目录
- [ ] 测试场景 4：分享损坏的软链接，验证显示错误并退出（退出码 1）
- [ ] 测试场景 5：分享普通文件，验证不显示软链接提示
- [ ] 测试场景 6：分享普通目录，验证不显示软链接提示
- [ ] 测试场景 7：用户输入无效内容，验证重新提示
- [ ] 所有场景的输出格式符合设计文档

**手动测试步骤**:

#### 场景 1：软链接文件，用户确认
```bash
# 准备测试文件
echo "Hello, World!" > /tmp/real_file.txt
ln -s /tmp/real_file.txt /tmp/symlink_file

# 运行测试（输入 y）
echo "y" | python -m src.main /tmp/symlink_file

# 验证：
# 1. 显示 "⚠️  检测到软链接："
# 2. 显示源路径和目标路径
# 3. 显示 "是否跟随软链接并分享目标文件/目录？(y/n):"
# 4. 开始分享真实文件（不是软链接）
```

#### 场景 2：软链接文件，用户取消
```bash
# 运行测试（输入 n）
echo "n" | python -m src.main /tmp/symlink_file

# 验证：
# 1. 显示软链接信息
# 2. 显示 "❌ 用户取消分享"
# 3. 程序退出（退出码 0）
```

#### 场景 3：软链接目录，用户确认
```bash
# 准备测试目录
mkdir -p /tmp/real_dir
echo "test" > /tmp/real_dir/test.txt
ln -s /tmp/real_dir /tmp/symlink_dir

# 运行测试（输入 y）
echo "y" | timeout 2 python -m src.main /tmp/symlink_dir || true

# 验证：
# 1. 显示软链接信息
# 2. 开始分享真实目录
```

#### 场景 4：损坏的软链接
```bash
# 创建损坏的软链接
ln -s /nonexistent/path /tmp/broken_symlink

# 运行测试
python -m src.main /tmp/broken_symlink
echo $?

# 验证：
# 1. 显示 "❌ 错误：软链接目标不存在"
# 2. 显示源路径和目标路径
# 3. 退出码为 1
```

#### 场景 5：普通文件
```bash
# 运行测试（普通文件）
echo "y" | timeout 2 python -m src.main /tmp/real_file.txt || true

# 验证：
# 1. 不显示软链接相关信息
# 2. 正常开始分享
```

#### 场景 6：用户输入无效内容
```bash
# 运行测试（输入 invalid, abc, y）
printf "invalid\nabc\ny\n" | timeout 2 python -m src.main /tmp/symlink_file || true

# 验证：
# 1. 每次输入无效内容后显示 "请输入 y 或 n"
# 2. 重新提示输入
# 3. 输入 y 后继续分享
```

**注意事项**:
- 使用 `/tmp` 目录进行测试，避免污染工作目录
- 使用 `timeout` 命令防止服务器一直运行
- 使用 `echo $?` 检查退出码
- 测试完成后清理：`rm -f /tmp/symlink* /tmp/real_file.txt /tmp/broken_symlink`

---

## 4. 测试覆盖率要求

### 4.1 代码覆盖率

| 组件 | 目标覆盖率 | 说明 |
|------|-----------|------|
| `handle_symlink()` | 90%+ | 核心新增函数，需要全面覆盖 |
| `validate_path()` 修改部分 | 80%+ | 软链接检测逻辑 |
| `main()` 错误处理 | 70%+ | 新增错误类型处理 |
| 整体 | 75%+ | 保持现有水平 |

### 4.2 场景覆盖率

| 场景 | 测试覆盖 | 验证方法 |
|------|---------|----------|
| 软链接文件（用户确认） | ✅ T-004, T-005 | 单元测试 + 手动测试 |
| 软链接目录（用户确认） | ✅ T-004, T-005 | 单元测试 + 手动测试 |
| 用户取消 | ✅ T-004, T-005 | 单元测试 + 手动测试 |
| 损坏的软链接 | ✅ T-004, T-005 | 单元测试 + 手动测试 |
| 普通文件不受影响 | ✅ T-004, T-005 | 单元测试 + 手动测试 |
| 用户输入无效内容 | ✅ T-004, T-005 | 单元测试 + 手动测试 |

---

## 5. 风险与依赖

### 5.1 技术风险

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 递归调用 `validate_path()` 导致循环 | 中 | 添加注释说明，真实路径通常不是软链接 |
| Windows 兼容性问题 | 低 | Windows 支持软链接（需要管理员权限），测试覆盖 |
| 用户输入测试困难 | 低 | 使用 `unittest.mock.patch` 模拟输入 |

### 5.2 任务依赖

```
T-001 (handle_symlink)
    ↓
T-002 (集成到 validate_path)
    ↓
T-003 (修改 main 错误处理)
    ↓
T-004 (单元测试)
    ↓
T-005 (手动测试验收)
```

---

## 6. 完成标准

### 6.1 代码完成标准

- [ ] 所有任务的代码实施完成
- [ ] 代码符合 PEP 8 规范
- [ ] 添加必要的注释和文档字符串
- [ ] 无 lint 警告（pylint, flake8）

### 6.2 测试完成标准

- [ ] 所有单元测试通过
- [ ] 测试覆盖率达到要求
- [ ] 所有手动测试场景通过
- [ ] 无回归问题（现有功能不受影响）

### 6.3 文档完成标准

- [ ] README 更新（添加软链接处理说明）
- [ ] CHANGELOG 更新（记录新功能）
- [ ] 版本号 bump（v1.0.11 → v1.0.12）

---

## 7. 后续清理

测试完成后清理临时文件：
```bash
rm -f /tmp/symlink_file /tmp/symlink_dir /tmp/broken_symlink
rm -f /tmp/real_file.txt
rm -rf /tmp/real_dir
```

---

**维护者**: AI Team
**文档版本**: v1
**最后更新**: 2025-02-05
