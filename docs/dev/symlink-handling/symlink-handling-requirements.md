---
feature: symlink-handling
complexity: standard
generated_by: clarify
generated_at: 2025-01-27T16:51:00Z
version: 1
---

# 需求文档: Symlink Handling

> **功能标识**: symlink-handling
> **复杂度**: standard
> **生成方式**: clarify
> **生成时间**: 2025-01-27

## 1. 概述

### 1.1 一句话描述

当用户分享的文件或目录本身是软链接时，自动检测软链接目标并询问用户是否继续，增强用户对分享内容的掌控力和安全性。

### 1.2 核心价值

**解决什么问题**：
- **安全性**：防止用户无意中分享了错误的文件（软链接可能指向敏感文件）
- **透明性**：用户清楚知道软链接指向哪里，避免"我以为分享的是A文件，实际分享了B文件"
- **可控性**：由用户决定是否跟随软链接，而不是系统自动跟随

**带来什么价值**：
- 降低误分享风险（如软链接指向 `/etc/passwd`）
- 提升用户信任（用户知道会分享什么）
- 保持工具的简洁性（不需要复杂的黑名单机制）

### 1.3 目标用户

- **主要用户**：通过命令行分享文件/目录的用户
- **次要用户**：通过脚本调用 quick-share 的开发者

---

## 2. 需求与用户故事

### 2.1 需求清单

| ID | 需求点 | 优先级 | 用户故事 |
|----|--------|--------|----------|
| R-001 | 检测软链接文件/目录 | P0 | As a 用户, I want 系统检测我要分享的文件/目录是否是软链接, so that 我知道会分享什么内容 |
| R-002 | 显示软链接目标路径 | P0 | As a 用户, I want 看到软链接指向的真实路径, so that 我能判断是否应该分享它 |
| R-003 | 交互式确认是否跟随 | P0 | As a 用户, I want 被询问是否跟随软链接, so that 我能控制分享行为 |
| R-004 | 处理损坏的软链接 | P1 | As a 用户, I want 系统检测软链接目标是否存在, so that 我能避免分享失败 |
| R-005 | 保持普通文件分享不受影响 | P0 | As a 用户, I want 普通文件/目录的分享流程不受影响, so that 现有使用方式不改变 |

### 2.2 验收标准

#### R-001: 检测软链接文件/目录

- **WHEN** 用户执行 `quick-share <path>` 且 `<path>` 是软链接
- **THEN** 系统 **SHALL** 检测到软链接（使用 `os.path.islink()`）
- **AND** 系统 **SHALL NOT** 进入普通分享流程

#### R-002: 显示软链接目标路径

- **WHEN** 检测到软链接
- **THEN** 系统 **SHALL** 读取软链接目标（使用 `os.readlink()` 或 `path.resolve()`）
- **AND** 系统 **SHALL** 显示软链接信息，格式：
  ```
  ⚠️  检测到软链接：
      源路径: /path/to/symlink
      目标路径: /real/path/to/target
  ```

#### R-003: 交互式确认是否跟随

- **WHEN** 检测到软链接
- **THEN** 系统 **SHALL** 询问用户：
  ```
  是否跟随软链接并分享目标文件/目录？(y/n):
  ```
- **AND** 用户输入 `y` **SHALL** 继续分享真实目标
- **AND** 用户输入 `n` **SHALL** 退出程序
- **AND** 用户输入其他内容 **SHALL** 重新询问

#### R-004: 处理损坏的软链接

- **WHEN** 检测到软链接但目标不存在（`os.path.exists(target) == False`）
- **THEN** 系统 **SHALL** 显示错误：
  ```
  ❌ 错误：软链接目标不存在
      源路径: /path/to/symlink
      目标路径: /nonexistent/target
  ```
- **AND** 系统 **SHALL** 以非零状态码退出

#### R-005: 保持普通文件分享不受影响

- **WHEN** 用户分享普通文件/目录（非软链接）
- **THEN** 系统 **SHALL** 不显示任何软链接相关信息
- **AND** 系统 **SHALL** 按现有流程直接开始分享

---

## 3. 功能验收清单

| ID | 功能点 | 验收步骤 | 优先级 | 关联需求 | 通过 |
|----|--------|----------|--------|----------|------|
| F-001 | 检测软链接文件 | 1. 创建指向文件的软链接<br>2. 执行 `quick-share symlink`<br>3. 验证显示软链接提示 | P0 | R-001, R-002 | ☐ |
| F-002 | 检测软链接目录 | 1. 创建指向目录的软链接<br>2. 执行 `quick-share symlink-dir`<br>3. 验证显示软链接提示 | P0 | R-001, R-002 | ☐ |
| F-003 | 用户选择跟随 | 1. 检测到软链接时输入 `y`<br>2. 验证开始分享真实目标 | P0 | R-003 | ☐ |
| F-004 | 用户选择不跟随 | 1. 检测到软链接时输入 `n`<br>2. 验证程序退出 | P0 | R-003 | ☐ |
| F-005 | 处理损坏的软链接 | 1. 创建指向不存在目标的软链接<br>2. 执行 `quick-share broken-symlink`<br>3. 验证显示错误并退出 | P1 | R-004 | ☐ |
| F-006 | 普通文件不受影响 | 1. 执行 `quick-share normal-file.txt`<br>2. 验证不显示软链接提示<br>3. 验证正常开始分享 | P0 | R-005 | ☐ |

---

## 4. 技术约束

### 4.1 技术栈

- **Python 版本**: Python 3.8+
- **依赖库**: 标准库（`os`, `pathlib`）
- **无需新增依赖**

### 4.2 集成点

- **模块**: `src/main.py`（`validate_file()` 和 `validate_path()` 函数）
- **模块**: `src/cli.py`（参数解析后、验证前）
- **测试**: `tests/test_main.py`（添加软链接相关测试）

### 4.3 实现约束

1. **不处理目录内部的软链接**：只检测用户分享的根路径是否是软链接，目录遍历时保持现有行为（`os.walk(followlinks=False)` 已在 `directory_handler.py` 中设置）
2. **不修改安全验证逻辑**：`validate_directory_path()` 中的 `os.path.realpath()` 逻辑保持不变
3. **向后兼容**：不影响现有功能和 API

---

## 5. 排除项

- **目录遍历中的软链接**：不处理目录内部包含的软链接文件（scope 限制）
- **命令行参数控制**：不提供 `--follow-symlinks` 参数（当前需求是交互式确认）
- **软链接环检测**：不处理循环软链接（环状链接）的情况（超出 scope）
- **跨文件系统软链接**：不特别处理跨文件系统的软链接（统一按普通软链接处理）
- **Windows 快捷方式**：不处理 Windows `.lnk` 快捷方式文件（只处理 Unix 软链接）

---

## 6. 下一步

✅ 在新会话中执行：
```bash
/clouditera:dev:spec-dev symlink-handling --skip-requirements
```

或直接开始实施：
```bash
# 修改 src/main.py 的 validate_file() 和 validate_path() 函数
# 添加软链接检测和用户确认逻辑
```

---

## 附录：当前代码分析

### A.1 现有软链接处理

**在 `src/main.py` 中**：
- `validate_file()` (line 84): 使用 `Path(file_path).resolve()` **自动跟随软链接**
- `validate_path()` (line 30): 使用 `Path(path).resolve()` **自动跟随软链接**

**在 `src/security.py` 中**：
- `validate_directory_path()` (line 78): 使用 `os.path.realpath()` **自动跟随软链接**

**问题**：
- 用户看不到软链接信息
- 用户无法控制是否跟随
- 可能分享到意料之外的文件

### A.2 修改建议

**位置**：`src/main.py` 的 `validate_path()` 函数开始处

**逻辑流程**：
```python
def validate_path(path: str) -> Tuple[bool, str, Optional[Path]]:
    # NEW: 检测软链接
    if os.path.islink(path):
        return handle_symlink(path)

    # 原有逻辑...
```

**新增函数**：
```python
def handle_symlink(symlink_path: str) -> Tuple[bool, str, Optional[Path]]:
    """处理软链接路径"""
    # 1. 读取软链接目标
    target = os.readlink(symlink_path)
    real_path = Path(symlink_path).resolve()

    # 2. 显示信息
    print(f"⚠️  检测到软链接：")
    print(f"    源路径: {symlink_path}")
    print(f"    目标路径: {real_path}")

    # 3. 检查目标是否存在
    if not real_path.exists():
        print(f"❌ 错误：软链接目标不存在")
        return False, "symlink_broken", None

    # 4. 询问用户
    while True:
        choice = input("是否跟随软链接并分享目标文件/目录？(y/n): ").strip().lower()
        if choice == 'y':
            return validate_path(str(real_path))  # 递归验证真实路径
        elif choice == 'n':
            print("❌ 用户取消分享")
            sys.exit(0)
        else:
            print("请输入 y 或 n")
```
