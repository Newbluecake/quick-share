---
feature: symlink-handling
complexity: standard
document_type: design
generated_by: architect-planner
generated_at: 2025-02-05T10:15:00Z
version: 1
---

# 技术设计文档: Symlink Handling

> **功能**: Symlink Handling
> **复杂度**: standard
> **文档版本**: v1

## 1. 系统架构设计

### 1.1 整体架构

```
┌─────────────────────────────────────────────────────────────┐
│                     User Input Layer                        │
│                  quick-share <symlink>                      │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                   Symlink Detection Layer                    │
│                   (NEW - handle_symlink)                     │
│  ┌────────────────────────────────────────────────────┐    │
│  │  1. os.path.islink(path) detection                 │    │
│  │  2. os.readlink() + path.resolve()                 │    │
│  │  3. Display info to user                           │    │
│  │  4. Interactive confirmation (y/n)                 │    │
│  │  5. Handle broken symlinks                          │    │
│  └────────────────────────────────────────────────────┘    │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                 Existing Validation Layer                   │
│              (validate_path, validate_file)                  │
│                  (unchanged behavior)                        │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                   Server Launch Layer                       │
│              (FileShareServer / DirectoryShareServer)        │
└─────────────────────────────────────────────────────────────┘
```

### 1.2 处理流程

```
用户执行: quick-share <path>
            │
            ▼
    ┌───────────────────┐
    │ Is <path> valid?  │
    └───────┬───────────┘
            │ Yes
            ▼
    ┌───────────────────────┐
    │ Is <path> a symlink?  │───No───▶ Continue normal flow
    └───────┬───────────────┘              (validate_path)
            │ Yes
            ▼
    ┌──────────────────────────────┐
    │ Display symlink info:        │
    │  - Source path               │
    │  - Target path               │
    └───────┬──────────────────────┘
            ▼
    ┌──────────────────────────┐
    │ Does target exist?       │
    └───────┬──────────────────┘
            │ No                        │ Yes
            ▼                           ▼
    ┌──────────────────┐      ┌─────────────────────┐
    │ Show error +     │      │ Ask user:           │
    │ exit(1)          │      │ Follow symlink?     │
    └──────────────────┘      └───┬──────────────┬──┘
                                    │ No           │ Yes
                                    ▼             ▼
                            ┌───────────┐  ┌──────────────────┐
                            │ exit(0)   │  │ validate(target) │
                            └───────────┘  └──────────────────┘
```

## 2. 组件设计

### 2.1 新增组件: handle_symlink()

**位置**: `src/main.py`

**函数签名**:
```python
def handle_symlink(symlink_path: str) -> Tuple[bool, str, Optional[Path]]:
    """
    Handle symlink detection and user confirmation.

    Args:
        symlink_path: The symlink path to handle.

    Returns:
        Tuple containing:
        - is_valid: True if user confirms, False otherwise
        - path_type: "file", "directory", or "symlink_broken"
        - resolved_path: Real target path if confirmed, None otherwise
    """
```

**设计要点**:
1. **检测软链接**: 使用 `os.path.islink()` 判断
2. **读取目标**: 使用 `os.readlink()` + `Path.resolve()` 获取真实路径
3. **显示信息**: 格式化输出源路径和目标路径
4. **验证目标**: 检查 `resolved_path.exists()`
5. **用户确认**: 交互式输入 y/n，循环直到有效输入
6. **返回值**: 返回与 `validate_path()` 兼容的元组格式

### 2.2 修改组件: validate_path()

**位置**: `src/main.py:30`

**修改内容**:
```python
def validate_path(path: str) -> Tuple[bool, str, Optional[Path]]:
    """
    Unified validation for both files and directories.

    NEW: Detect symlinks first before normal validation.
    """
    # NEW: 检测软链接（在 detect_path_type 之前）
    if os.path.islink(path):
        return handle_symlink(path)

    # 原有逻辑保持不变...
    path_type = detect_path_type(path)
    # ...
```

**设计要点**:
1. **最早检测**: 在函数开头就检测软链接，提前拦截
2. **向后兼容**: 非软链接路径完全走原有逻辑
3. **返回值统一**: `handle_symlink()` 返回相同格式的元组

### 2.3 保持不变: validate_file()

**位置**: `src/main.py:84`

**设计决策**: **不修改** `validate_file()`，原因：
- `validate_file()` 已被 `validate_path()` 替代（main() 中使用 validate_path）
- `validate_file()` 保留用于向后兼容，但主流程不再调用
- 所有路径验证都通过 `validate_path()` 统一处理

## 3. 接口设计

### 3.1 handle_symlink() 返回值

| 场景 | 返回值 | 说明 |
|------|--------|------|
| 用户确认跟随 | `(True, "file", Path(target))` | 目标是文件 |
| 用户确认跟随 | `(True, "directory", Path(target))` | 目标是目录 |
| 用户取消 | `(False, "symlink_cancelled", None)` | 用户输入 n |
| 损坏的软链接 | `(False, "symlink_broken", None)` | 目标不存在 |

### 3.2 main() 流程调整

**位置**: `src/main.py:127-136`

**现有代码**:
```python
# Validate path (file or directory)
try:
    is_valid, path_type, resolved_path = validate_path(args.file_path)

    if not is_valid or path_type == "invalid":
        print(f"Error: Invalid path: {args.file_path}", file=sys.stderr)
        sys.exit(1)
except PermissionError:
    print(f"Error: Permission denied reading {args.file_path}", file=sys.stderr)
    sys.exit(1)
```

**需要新增的错误处理**:
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

## 4. 数据设计

### 4.1 用户交互消息

**检测到软链接**:
```
⚠️  检测到软链接：
    源路径: /path/to/symlink
    目标路径: /real/path/to/target
```

**损坏的软链接**:
```
❌ 错误：软链接目标不存在
    源路径: /path/to/symlink
    目标路径: /nonexistent/target
```

**用户确认提示**:
```
是否跟随软链接并分享目标文件/目录？(y/n):
```

**无效输入**:
```
请输入 y 或 n
```

**用户取消**:
```
❌ 用户取消分享
```

### 4.2 错误码

| 场景 | 退出码 | 说明 |
|------|--------|------|
| 用户确认跟随 | 0 | 正常流程，继续分享 |
| 用户取消 | 0 | 用户主动取消，非错误 |
| 损坏的软链接 | 1 | 错误退出 |
| 其他验证失败 | 1 | 保持现有行为 |

## 5. 安全考量

### 5.1 安全风险分析

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 软链接指向敏感文件 | 高 | 用户确认机制，透明显示目标路径 |
| 软链接指向系统文件 | 高 | 用户确认机制，用户可见目标路径 |
| 损坏的软链接 | 低 | 提前检测并报错退出 |
| 循环软链接 | 低 | Path.resolve() 会抛出异常，现有异常处理捕获 |

### 5.2 安全约束

1. **不修改安全验证逻辑**: `validate_directory_path()` 中的 `os.path.realpath()` 保持不变
2. **不自动跟随**: 始终询问用户，不添加 `--follow-symlinks` 参数
3. **透明性**: 始终显示真实目标路径，用户清楚知道分享什么
4. **向后兼容**: 普通文件/目录的分享流程完全不受影响

### 5.3 权限检查

**现有权限检查保持不变**:
- `validate_path()` 中的 `PermissionError` 处理
- 文件可读性检查（`open(resolved_path, 'rb')`）
- 目录可访问性检查（`list(resolved_path.iterdir())`）

**新增权限检查**:
- 软链接目标路径的权限检查在用户确认后进行
- 如果目标无权限，`validate_path()` 会返回 `is_valid=False`
- main() 中的 `PermissionError` 处理会捕获

## 6. 性能考量

### 6.1 性能影响分析

| 操作 | 复杂度 | 影响评估 |
|------|--------|----------|
| `os.path.islink()` | O(1) | 可忽略 |
| `os.readlink()` | O(1) | 可忽略 |
| `Path.resolve()` | O(1) | 可忽略（不涉及目录遍历） |
| 用户等待输入 | - | **主要影响**，但符合需求 |

### 6.2 性能优化

**不需要特殊优化**，原因：
- 软链接检测只在用户提供的路径上执行一次
- 不涉及目录遍历（目录内部的软链接不处理）
- 用户等待输入是交互需求，不是性能瓶颈

## 7. 测试策略

### 7.1 单元测试

**新增测试函数**:
```python
def test_handle_symlink_file():
    """Test handling of symlink to file"""

def test_handle_symlink_directory():
    """Test handling of symlink to directory"""

def test_handle_broken_symlink():
    """Test handling of broken symlink"""

def test_validate_path_with_symlink():
    """Test validate_path detects symlink"""

def test_validate_path_normal_file():
    """Test normal files are unaffected"""
```

### 7.2 集成测试

**测试场景**:
1. 分享软链接文件 → 用户输入 y → 分享目标文件
2. 分享软链接目录 → 用户输入 n → 程序退出
3. 分享损坏的软链接 → 显示错误并退出
4. 分享普通文件 → 不显示软链接提示

### 7.3 边界测试

| 边界情况 | 预期行为 |
|---------|----------|
| 软链接指向软链接 | Path.resolve() 自动解析到最终目标 |
| 软链接指向不存在路径 | 显示"目标不存在"错误 |
| 用户输入无效（非 y/n） | 循环提示，直到输入有效 |
| 软链接无权限读取 | PermissionError 异常处理 |

## 8. 实现约束

### 8.1 Scope 限制

- **只检测根路径**: 不处理目录遍历中的软链接（`os.walk(followlinks=False)` 已设置）
- **不处理循环链接**: 循环软链接会导致 Path.resolve() 抛出异常，现有异常处理捕获
- **不修改目录遍历**: `directory_handler.py` 保持不变

### 8.2 向后兼容

- **普通文件不受影响**: 非软链接路径完全走原有逻辑
- **API 兼容**: 不改变函数签名和返回值格式
- **错误处理兼容**: 新错误类型（symlink_broken, symlink_cancelled）在 main() 中处理

### 8.3 依赖约束

- **无新增依赖**: 只使用标准库（`os`, `pathlib`, `sys`）
- **Python 版本**: Python 3.8+（现有约束）

## 9. 部署计划

### 9.1 发布流程

1. **代码实施**: 修改 `src/main.py`
2. **单元测试**: 添加测试到 `tests/test_main.py`
3. **集成测试**: 手动测试软链接场景
4. **文档更新**: 更新 README 和 CHANGELOG
5. **版本发布**: bump version → git tag → PyPI publish

### 9.2 回滚计划

如果出现问题：
1. 回滚 `src/main.py` 的修改
2. 保留测试用例（不影响现有行为）
3. 发布 patch 版本（如 v1.0.12 → v1.0.11.1）

## 10. 后续优化

### 10.1 可能的未来增强

| 特性 | 优先级 | 说明 |
|------|--------|------|
| `--follow-symlinks` 参数 | P2 | 跳过确认，自动跟随（脚本场景） |
| 软链接环检测 | P3 | 更好的错误提示 |
| 批量确认 | P3 | 多个软链接时一次性确认 |
| 配置文件支持 | P3 | 记住用户选择 |

### 10.2 不在当前 Scope

- Windows 快捷方式（`.lnk` 文件）
- 硬链接检测（hard link）
- 符号链接权限特殊处理

---

**维护者**: AI Team
**文档版本**: v1
**最后更新**: 2025-02-05
