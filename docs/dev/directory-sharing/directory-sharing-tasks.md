# 任务拆分文档: Directory Sharing

> **状态**: Planning Phase - Stage 3
> **生成时间**: 2026-01-14
> **复杂度**: standard
> **关联文档**:
> - 需求: docs/dev/directory-sharing/directory-sharing-requirements.md
> - 设计: docs/dev/directory-sharing/directory-sharing-design.md

## 任务总览

| 阶段 | 任务数 | 预计工作量 |
|------|--------|-----------|
| Phase 1: 安全层扩展 | 3 | 4h |
| Phase 2: 核心处理器 | 4 | 6h |
| Phase 3: Server层 | 5 | 8h |
| Phase 4: Main层集成 | 3 | 4h |
| Phase 5: 会话管理 | 2 | 3h |
| Phase 6: 回归测试 | 2 | 2h |
| **总计** | **19** | **27h** |

## 任务依赖关系

```
Phase 1: 安全层扩展
├── T-001: 路径遍历检测（目录版本）
├── T-002: 目录路径验证函数
└── T-003: 符号链接安全检测

Phase 2: 核心处理器 (依赖 Phase 1)
├── T-004: 目录信息统计
├── T-005: HTML列表生成器
├── T-006: 文件大小格式化
└── T-007: 流式Zip生成

Phase 3: Server层 (依赖 Phase 2)
├── T-008: DirectoryShareHandler - 目录列表
├── T-009: DirectoryShareHandler - 文件下载
├── T-010: DirectoryShareHandler - Zip下载
├── T-011: DirectoryShareServer 基础
└── T-012: DirectoryShareServer 超时

Phase 4: Main层集成 (依赖 Phase 3)
├── T-013: 路径类型检测
├── T-014: validate_path 统一函数
└── T-015: main() Server分发

Phase 5: 会话管理 (依赖 Phase 3)
├── T-016: 会话跟踪机制
└── T-017: 会话限制执行

Phase 6: 回归测试 (依赖 Phase 4 & 5)
├── T-018: 单文件共享回归测试
└── T-019: CLI选项兼容性测试
```

## 并行执行分组

### Group 1 (并行，无依赖)
- T-001: 路径遍历检测
- T-002: 目录路径验证
- T-003: 符号链接安全检测

### Group 2 (并行，依赖 Group 1)
- T-004: 目录信息统计
- T-005: HTML列表生成器
- T-006: 文件大小格式化
- T-007: 流式Zip生成

### Group 3 (并行，依赖 Group 2)
- T-008: DirectoryShareHandler - 目录列表
- T-009: DirectoryShareHandler - 文件下载
- T-010: DirectoryShareHandler - Zip下载

### Group 4 (顺序，依赖 Group 3)
- T-011: DirectoryShareServer 基础
- T-012: DirectoryShareServer 超时

### Group 5 (顺序，依赖 Group 4)
- T-013: 路径类型检测
- T-014: validate_path 统一函数
- T-015: main() Server分发

### Group 6 (并行，依赖 Group 4)
- T-016: 会话跟踪机制
- T-017: 会话限制执行

### Group 7 (并行，依赖 Group 5 & 6)
- T-018: 单文件共享回归测试
- T-019: CLI选项兼容性测试

---

## Phase 1: 安全层扩展

### T-001: 路径遍历检测（目录版本）

**优先级**: P0
**预计时间**: 1h
**依赖**: 无
**关联需求**: 需求2 - 目录沙箱安全机制
**关联测试**: test_should_reject_parent_traversal, test_should_reject_encoded_traversal

**目标**: 扩展 security.py 的路径遍历检测，支持多层编码和复杂遍历模式。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_security.py):
```python
def test_should_reject_parent_traversal():
    """测试拒绝父目录遍历攻击"""
    assert is_path_traversal_attack("/../../../etc/passwd") == True
    assert is_path_traversal_attack("/subdir/../../../secret.txt") == True
    assert is_path_traversal_attack("/./subdir/../../file.txt") == True

def test_should_reject_encoded_traversal():
    """测试拒绝URL编码的路径遍历"""
    import urllib.parse
    encoded = urllib.parse.quote("../../../etc/passwd")
    assert is_path_traversal_attack(encoded) == True
    # 多重编码
    double_encoded = urllib.parse.quote(encoded)
    assert is_path_traversal_attack(double_encoded) == True

def test_should_allow_normal_paths():
    """测试允许正常路径"""
    assert is_path_traversal_attack("/file.txt") == False
    assert is_path_traversal_attack("/subdir/file.txt") == False
    assert is_path_traversal_attack("/a/b/c/file.txt") == False
```

2. **Green - 实现功能** (src/security.py):
```python
def is_path_traversal_attack(path: str) -> bool:
    """
    检测路径遍历攻击（增强版）。

    检查:
    - '..' 在任何位置
    - URL编码的 '..'
    - 多重编码
    """
    # 原始检查
    if ".." in path:
        return True

    # URL解码并检查（最多解码2次防止多重编码）
    decoded = path
    for _ in range(2):
        try:
            decoded = urllib.parse.unquote(decoded)
            if ".." in decoded:
                return True
        except Exception:
            break

    return False
```

3. **Refactor - 重构优化**:
   - 添加日志记录
   - 优化性能
   - 添加边缘案例处理

**验收标准**:
- 所有3个测试通过
- 检测到所有路径遍历模式
- 不误报正常路径

---

### T-002: 目录路径验证函数

**优先级**: P0
**预计时间**: 2h
**依赖**: T-001
**关联需求**: 需求2 - 目录沙箱安全机制
**关联测试**: test_should_allow_subdirectory_access

**目标**: 实现 validate_directory_path() 函数，验证请求路径在共享目录沙箱内。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_security.py):
```python
def test_should_allow_subdirectory_access(tmp_path):
    """测试允许合法子目录访问"""
    # 创建测试目录结构
    shared_dir = tmp_path / "shared"
    shared_dir.mkdir()
    (shared_dir / "file.txt").write_text("content")
    subdir = shared_dir / "subdir"
    subdir.mkdir()
    (subdir / "nested.txt").write_text("nested")

    # 测试根目录访问
    is_valid, real_path = validate_directory_path("/", str(shared_dir))
    assert is_valid == True
    assert os.path.samefile(real_path, str(shared_dir))

    # 测试文件访问
    is_valid, real_path = validate_directory_path("/file.txt", str(shared_dir))
    assert is_valid == True
    assert os.path.samefile(real_path, str(shared_dir / "file.txt"))

    # 测试子目录访问
    is_valid, real_path = validate_directory_path("/subdir/nested.txt", str(shared_dir))
    assert is_valid == True
    assert os.path.samefile(real_path, str(subdir / "nested.txt"))

def test_should_reject_path_outside_sandbox(tmp_path):
    """测试拒绝沙箱外路径"""
    shared_dir = tmp_path / "shared"
    shared_dir.mkdir()
    outside_file = tmp_path / "outside.txt"
    outside_file.write_text("secret")

    # 尝试遍历到父目录
    is_valid, _ = validate_directory_path("/../outside.txt", str(shared_dir))
    assert is_valid == False

    is_valid, _ = validate_directory_path("/subdir/../../outside.txt", str(shared_dir))
    assert is_valid == False

def test_should_reject_nonexistent_path(tmp_path):
    """测试拒绝不存在的路径"""
    shared_dir = tmp_path / "shared"
    shared_dir.mkdir()

    is_valid, _ = validate_directory_path("/nonexistent.txt", str(shared_dir))
    assert is_valid == False
```

2. **Green - 实现功能** (src/security.py):
```python
def validate_directory_path(
    request_path: str,
    shared_directory: str
) -> Tuple[bool, str]:
    """验证目录访问请求路径。"""
    # 1. 清理路径
    clean_path = request_path.split('?')[0].split('#')[0]

    # 2. URL解码
    decoded_path = urllib.parse.unquote(clean_path)

    # 3. 检测遍历攻击
    if is_path_traversal_attack(decoded_path):
        return False, ""

    # 4. 构建完整路径
    relative_path = decoded_path.lstrip('/')
    if not relative_path:  # 根路径
        full_path = shared_directory
    else:
        full_path = os.path.join(shared_directory, relative_path)

    # 5. 解析真实路径
    try:
        real_path = os.path.realpath(full_path)
        real_shared = os.path.realpath(shared_directory)
    except Exception:
        return False, ""

    # 6. 验证在沙箱内
    try:
        common = os.path.commonpath([real_path, real_shared])
        if common != real_shared:
            return False, ""
    except ValueError:
        return False, ""

    # 7. 验证路径存在
    if not os.path.exists(real_path):
        return False, ""

    return True, real_path
```

3. **Refactor**:
   - 提取常量
   - 添加日志记录
   - 性能优化

**验收标准**:
- 所有测试通过
- 正确处理符号链接
- 正确验证沙箱边界

---

### T-003: 符号链接安全检测

**优先级**: P0
**预计时间**: 1h
**依赖**: T-002
**关联需求**: 需求2 - 目录沙箱安全机制
**关联功能**: F-011 边缘场景：符号链接

**目标**: 验证 validate_directory_path 能正确处理符号链接逃逸。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_security.py):
```python
def test_symlink_escape_detection(tmp_path):
    """测试检测符号链接逃逸沙箱"""
    shared_dir = tmp_path / "shared"
    shared_dir.mkdir()

    # 创建沙箱外的文件
    secret_dir = tmp_path / "secret"
    secret_dir.mkdir()
    secret_file = secret_dir / "confidential.txt"
    secret_file.write_text("secret data")

    # 在共享目录内创建指向外部的符号链接
    symlink_path = shared_dir / "escape_link"
    symlink_path.symlink_to(secret_file)

    # 尝试通过符号链接访问
    is_valid, _ = validate_directory_path("/escape_link", str(shared_dir))
    assert is_valid == False, "应拒绝指向沙箱外的符号链接"

def test_symlink_within_sandbox_allowed(tmp_path):
    """测试允许沙箱内的符号链接"""
    shared_dir = tmp_path / "shared"
    shared_dir.mkdir()

    # 创建沙箱内的文件
    real_file = shared_dir / "real.txt"
    real_file.write_text("real content")

    # 创建指向沙箱内的符号链接
    symlink_path = shared_dir / "link.txt"
    symlink_path.symlink_to(real_file)

    # 应允许访问
    is_valid, real_path = validate_directory_path("/link.txt", str(shared_dir))
    assert is_valid == True
    # 验证解析到真实路径
    assert os.path.samefile(real_path, str(real_file))
```

2. **Green - 验证实现**:
   - validate_directory_path 的现有实现应已通过 realpath 处理符号链接
   - 运行测试验证

3. **Refactor**:
   - 添加日志记录符号链接事件
   - 文档化符号链接行为

**验收标准**:
- 拒绝指向沙箱外的符号链接
- 允许沙箱内的符号链接
- 日志记录符号链接访问

---

## Phase 2: 核心处理器

### T-004: 目录信息统计

**优先级**: P0
**预计时间**: 1h
**依赖**: 无（独立功能）
**关联需求**: 需求3 - Web文件浏览界面
**关联功能**: F-001, F-013

**目标**: 实现 get_directory_info() 和 format_file_size()。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_directory_handler.py - 新文件):
```python
import sys
import os
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from directory_handler import get_directory_info, format_file_size

def test_get_directory_info_basic(tmp_path):
    """测试基本目录统计"""
    test_dir = tmp_path / "test"
    test_dir.mkdir()

    # 创建文件
    (test_dir / "file1.txt").write_text("a" * 100)
    (test_dir / "file2.txt").write_text("b" * 200)

    # 创建子目录和文件
    subdir = test_dir / "subdir"
    subdir.mkdir()
    (subdir / "file3.txt").write_text("c" * 150)

    info = get_directory_info(str(test_dir))

    assert info['total_files'] == 3
    assert info['total_dirs'] == 1
    assert info['total_size'] == 450

def test_get_directory_info_empty(tmp_path):
    """测试空目录"""
    empty_dir = tmp_path / "empty"
    empty_dir.mkdir()

    info = get_directory_info(str(empty_dir))

    assert info['total_files'] == 0
    assert info['total_dirs'] == 0
    assert info['total_size'] == 0

def test_format_file_size():
    """测试文件大小格式化"""
    assert format_file_size(500) == "500.0 B"
    assert format_file_size(1500) == "1.5 KB"
    assert format_file_size(1024 * 1024) == "1.0 MB"
    assert format_file_size(1536 * 1024) == "1.5 MB"
    assert format_file_size(1024 * 1024 * 1024) == "1.0 GB"
```

2. **Green - 实现功能** (src/directory_handler.py - 新文件):
```python
"""目录共享的核心处理逻辑。"""

import os
from pathlib import Path
from typing import Dict

def get_directory_info(directory_path: str) -> Dict:
    """获取目录统计信息。"""
    path = Path(directory_path)

    total_files = 0
    total_dirs = 0
    total_size = 0

    try:
        for item in path.rglob('*'):
            if item.is_file():
                total_files += 1
                try:
                    total_size += item.stat().st_size
                except (OSError, PermissionError):
                    pass
            elif item.is_dir():
                total_dirs += 1
    except (OSError, PermissionError):
        pass

    return {
        'total_files': total_files,
        'total_dirs': total_dirs,
        'total_size': total_size
    }

def format_file_size(size_bytes: int) -> str:
    """将字节数格式化为人类可读格式。"""
    for unit in ['B', 'KB', 'MB', 'GB', 'TB']:
        if size_bytes < 1024.0:
            return f"{size_bytes:.1f} {unit}"
        size_bytes /= 1024.0
    return f"{size_bytes:.1f} PB"
```

3. **Refactor**:
   - 优化性能（缓存统计？）
   - 处理权限错误

**验收标准**:
- 正确统计文件和目录数量
- 正确计算总大小
- 人类可读的大小格式

---

### T-005: HTML列表生成器

**优先级**: P0
**预计时间**: 3h
**依赖**: T-004
**关联需求**: 需求3 - Web文件浏览界面
**关联测试**: test_directory_listing_display
**关联功能**: F-001, F-005, F-013

**目标**: 实现 generate_directory_listing_html()。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_directory_handler.py):
```python
from directory_handler import generate_directory_listing_html

def test_generate_directory_listing_basic(tmp_path):
    """测试基本目录列表HTML生成"""
    base_dir = tmp_path / "shared"
    base_dir.mkdir()

    # 创建文件和目录
    (base_dir / "file1.txt").write_text("content")
    subdir = base_dir / "subdir"
    subdir.mkdir()

    html = generate_directory_listing_html(str(base_dir), str(base_dir))

    # 验证HTML包含关键元素
    assert "Quick Share" in html
    assert "file1.txt" in html
    assert "subdir" in html
    assert "Download All as Zip" in html
    assert "<!DOCTYPE html>" in html

def test_generate_directory_listing_subdirectory(tmp_path):
    """测试子目录列表HTML"""
    base_dir = tmp_path / "shared"
    base_dir.mkdir()
    subdir = base_dir / "subdir"
    subdir.mkdir()
    (subdir / "nested.txt").write_text("nested")

    html = generate_directory_listing_html(str(base_dir), str(subdir))

    # 应显示子目录路径
    assert "/subdir" in html
    assert "nested.txt" in html
    # 应有"返回上级"按钮
    assert "Go Up" in html

def test_generate_directory_listing_empty(tmp_path):
    """测试空目录"""
    empty_dir = tmp_path / "empty"
    empty_dir.mkdir()

    html = generate_directory_listing_html(str(empty_dir), str(empty_dir))

    assert "No files or directories" in html or "No files" in html

def test_generate_directory_listing_special_chars(tmp_path):
    """测试特殊字符文件名"""
    base_dir = tmp_path / "shared"
    base_dir.mkdir()

    # 创建包含特殊字符的文件名
    (base_dir / "file with spaces.txt").write_text("content")
    (base_dir / "中文文件.txt").write_text("中文")

    html = generate_directory_listing_html(str(base_dir), str(base_dir))

    # HTML转义
    assert "file with spaces.txt" in html
    assert "中文文件.txt" in html
```

2. **Green - 实现功能** (src/directory_handler.py):
```python
import html
from datetime import datetime

def generate_directory_listing_html(
    base_dir: str,
    current_dir: str
) -> str:
    """生成目录列表HTML页面。"""
    # 计算相对路径
    relative_path = os.path.relpath(current_dir, base_dir)
    if relative_path == '.':
        relative_path = '/'
    else:
        relative_path = '/' + relative_path

    # 列出当前目录内容
    items = []
    try:
        for entry in os.scandir(current_dir):
            try:
                stat = entry.stat(follow_symlinks=False)
                items.append({
                    'name': entry.name,
                    'is_dir': entry.is_dir(),
                    'size': stat.st_size if entry.is_file() else 0,
                    'modified': datetime.fromtimestamp(stat.st_mtime)
                })
            except (OSError, PermissionError):
                continue
    except PermissionError:
        return generate_error_html("Permission denied")

    # 排序：目录在前，然后按名称
    items.sort(key=lambda x: (not x['is_dir'], x['name'].lower()))

    # 生成HTML
    html_parts = [
        '<!DOCTYPE html>',
        '<html lang="en">',
        '<head>',
        '    <meta charset="UTF-8">',
        '    <meta name="viewport" content="width=device-width, initial-scale=1.0">',
        '    <title>Quick Share - Directory Listing</title>',
        '    <style>',
        '        body { font-family: Arial, sans-serif; margin: 20px; background: #f5f5f5; }',
        '        .container { max-width: 1200px; margin: 0 auto; background: white; padding: 20px; border-radius: 8px; }',
        '        h1 { color: #333; border-bottom: 2px solid #007bff; padding-bottom: 10px; }',
        '        .btn { padding: 10px 20px; background: #007bff; color: white; text-decoration: none; border-radius: 4px; }',
        '        table { width: 100%; border-collapse: collapse; margin-top: 20px; }',
        '        th, td { text-align: left; padding: 12px; border-bottom: 1px solid #ddd; }',
        '        .dir { color: #007bff; font-weight: bold; }',
        '    </style>',
        '</head>',
        '<body>',
        '    <div class="container">',
        f'        <h1>Quick Share - {html.escape(os.path.basename(base_dir))}</h1>',
        f'        <div>Current Path: {html.escape(relative_path)}</div>',
        '        <div>',
        f'            <a href="/?download=zip" class="btn">Download All as Zip</a>',
    ]

    # 添加"返回上级"按钮
    if current_dir != base_dir:
        parent_relative = os.path.dirname(relative_path)
        if not parent_relative:
            parent_relative = '/'
        html_parts.append(f'            <a href="{parent_relative}" class="btn">Go Up</a>')

    html_parts.extend([
        '        </div>',
        '        <table>',
        '            <thead>',
        '                <tr><th>Name</th><th>Size</th><th>Modified</th></tr>',
        '            </thead>',
        '            <tbody>',
    ])

    # 添加文件/目录行
    if not items:
        html_parts.append('                <tr><td colspan="3">No files or directories</td></tr>')
    else:
        for item in items:
            icon = '📁' if item['is_dir'] else '📄'
            css_class = 'dir' if item['is_dir'] else 'file'
            size_str = '-' if item['is_dir'] else format_file_size(item['size'])
            modified_str = item['modified'].strftime('%Y-%m-%d %H:%M')

            link_path = os.path.join(relative_path, item['name'])
            if item['is_dir']:
                link_path += '/'

            html_parts.append(
                f'                <tr>'
                f'<td class="{css_class}"><a href="{html.escape(link_path)}">{icon} {html.escape(item["name"])}</a></td>'
                f'<td>{size_str}</td>'
                f'<td>{modified_str}</td>'
                f'</tr>'
            )

    html_parts.extend([
        '            </tbody>',
        '        </table>',
        '    </div>',
        '</body>',
        '</html>',
    ])

    return '\n'.join(html_parts)

def generate_error_html(error_message: str) -> str:
    """生成错误页面HTML。"""
    return f"""
    <!DOCTYPE html>
    <html>
    <head>
        <title>Error - Quick Share</title>
    </head>
    <body>
        <h1>Error</h1>
        <p>{html.escape(error_message)}</p>
    </body>
    </html>
    """
```

3. **Refactor**:
   - 提取CSS到常量
   - 优化HTML模板结构

**验收标准**:
- 生成有效的HTML5
- 正确显示文件和目录
- 处理特殊字符
- 显示文件大小和时间

---

### T-006: 文件大小格式化

**优先级**: P1
**预计时间**: 0.5h
**依赖**: 无
**关联功能**: F-013

**说明**: 已在 T-004 中实现，此任务可合并。

---

### T-007: 流式Zip生成

**优先级**: P0
**预计时间**: 1.5h
**依赖**: 无（独立功能）
**关联需求**: 需求4 - 整目录下载为Zip
**关联测试**: test_zip_preserves_structure, test_download_directory_as_zip
**关联功能**: F-003

**目标**: 实现 stream_directory_as_zip()。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_directory_handler.py):
```python
import zipfile
import io

def test_stream_directory_as_zip_basic(tmp_path):
    """测试基本zip生成"""
    base_dir = tmp_path / "shared"
    base_dir.mkdir()

    # 创建文件
    (base_dir / "file1.txt").write_text("content1")
    subdir = base_dir / "subdir"
    subdir.mkdir()
    (subdir / "file2.txt").write_text("content2")

    # 流式生成zip到内存
    output = io.BytesIO()
    stream_directory_as_zip(output, str(base_dir), str(base_dir))

    # 验证zip内容
    output.seek(0)
    with zipfile.ZipFile(output, 'r') as zf:
        names = zf.namelist()
        assert 'file1.txt' in names
        assert 'subdir/file2.txt' in names

        # 验证内容
        assert zf.read('file1.txt') == b'content1'
        assert zf.read('subdir/file2.txt') == b'content2'

def test_stream_directory_as_zip_preserves_structure(tmp_path):
    """测试zip保留目录结构"""
    base_dir = tmp_path / "shared"
    base_dir.mkdir()

    # 创建深层目录结构
    deep = base_dir / "a" / "b" / "c"
    deep.mkdir(parents=True)
    (deep / "deep.txt").write_text("deep content")

    output = io.BytesIO()
    stream_directory_as_zip(output, str(base_dir), str(base_dir))

    output.seek(0)
    with zipfile.ZipFile(output, 'r') as zf:
        assert 'a/b/c/deep.txt' in zf.namelist()

def test_stream_directory_as_zip_empty(tmp_path):
    """测试空目录zip"""
    empty_dir = tmp_path / "empty"
    empty_dir.mkdir()

    output = io.BytesIO()
    stream_directory_as_zip(output, str(empty_dir), str(empty_dir))

    output.seek(0)
    with zipfile.ZipFile(output, 'r') as zf:
        # 空zip仍然有效
        assert len(zf.namelist()) == 0
```

2. **Green - 实现功能** (src/directory_handler.py):
```python
import zipfile

def stream_directory_as_zip(
    output_stream,
    base_dir: str,
    target_dir: str
) -> None:
    """流式生成目录的zip文件。"""
    with zipfile.ZipFile(output_stream, 'w', zipfile.ZIP_DEFLATED) as zipf:
        for root, dirs, files in os.walk(target_dir):
            for file in files:
                file_path = os.path.join(root, file)

                # 计算zip内的相对路径
                arcname = os.path.relpath(file_path, base_dir)

                try:
                    zipf.write(file_path, arcname)
                except (OSError, PermissionError):
                    # 跳过无法读取的文件
                    continue
```

3. **Refactor**:
   - 添加错误处理
   - 考虑压缩级别

**验收标准**:
- 生成有效的zip文件
- 保留目录结构
- 流式传输（不占用大量内存）

---

## Phase 3: Server层

### T-008: DirectoryShareHandler - 目录列表

**优先级**: P0
**预计时间**: 2h
**依赖**: T-002, T-005
**关联需求**: 需求3 - Web文件浏览界面
**关联功能**: F-001, F-005

**目标**: 实现 DirectoryShareHandler 的目录列表功能。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_server.py):
```python
def test_directory_handler_listing_response(tmp_path):
    """测试DirectoryShareHandler返回目录列表"""
    from server import DirectoryShareHandler
    from unittest.mock import MagicMock

    # 创建测试目录
    test_dir = tmp_path / "shared"
    test_dir.mkdir()
    (test_dir / "file.txt").write_text("content")

    # 模拟server和request
    mock_server = MagicMock()
    mock_server.directory_path = str(test_dir)

    mock_request = MagicMock()
    mock_client = ('127.0.0.1', 12345)

    # 创建handler
    with patch.object(BaseHTTPRequestHandler, '__init__', return_value=None):
        handler = DirectoryShareHandler(mock_request, mock_client, mock_server)
        handler.path = "/"
        handler.server = mock_server
        handler.send_response = MagicMock()
        handler.send_header = MagicMock()
        handler.end_headers = MagicMock()
        handler.wfile = MagicMock()

        # Mock validate_directory_path
        with patch('server.validate_directory_path', return_value=(True, str(test_dir))):
            handler.do_GET()

        # 验证响应
        handler.send_response.assert_called_with(200)
        handler.send_header.assert_any_call('Content-Type', 'text/html; charset=utf-8')

        # 验证HTML包含文件名
        written_data = b''.join(call.args[0] for call in handler.wfile.write.call_args_list)
        html_content = written_data.decode('utf-8')
        assert 'file.txt' in html_content
```

2. **Green - 实现功能** (src/server.py):
```python
from .security import validate_directory_path
from .directory_handler import generate_directory_listing_html

class DirectoryShareHandler(BaseHTTPRequestHandler):
    """处理目录共享的HTTP请求。"""

    def do_GET(self):
        """处理GET请求：列表显示、文件下载、目录zip。"""
        directory_path = self.server.directory_path

        # 验证路径
        is_valid, real_path = validate_directory_path(
            self.path,
            directory_path
        )

        if not is_valid:
            self.send_error(403, "Access denied")
            return

        # 检查是否请求zip下载
        if '?download=zip' in self.path or '?action=zip' in self.path:
            self._serve_directory_zip(directory_path, real_path)
            return

        # 判断是文件还是目录
        if os.path.isfile(real_path):
            self._serve_file(real_path)
        else:
            self._serve_directory_listing(directory_path, real_path)

    def _serve_directory_listing(self, base_dir: str, current_dir: str):
        """生成并返回目录列表HTML。"""
        html = generate_directory_listing_html(base_dir, current_dir)
        html_bytes = html.encode('utf-8')

        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(html_bytes)))
        self.end_headers()
        self.wfile.write(html_bytes)

    def log_message(self, format, *args):
        """Suppress default logging."""
        pass
```

3. **Refactor**:
   - 提取公共逻辑
   - 添加错误处理

**验收标准**:
- 返回200响应和HTML
- HTML包含目录内容
- 正确处理子目录

---

### T-009: DirectoryShareHandler - 文件下载

**优先级**: P0
**预计时间**: 1.5h
**依赖**: T-002
**关联需求**: 需求3 - Web文件浏览界面
**关联测试**: test_download_single_file_from_directory
**关联功能**: F-002

**目标**: 实现从目录下载单个文件。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_server.py):
```python
def test_directory_handler_file_response(tmp_path):
    """测试DirectoryShareHandler下载单个文件"""
    from server import DirectoryShareHandler

    test_dir = tmp_path / "shared"
    test_dir.mkdir()
    test_file = test_dir / "download.txt"
    test_file.write_text("download content")

    mock_server = MagicMock()
    mock_server.directory_path = str(test_dir)

    with patch.object(BaseHTTPRequestHandler, '__init__', return_value=None):
        handler = DirectoryShareHandler(MagicMock(), ('127.0.0.1', 12345), mock_server)
        handler.path = "/download.txt"
        handler.server = mock_server
        handler.send_response = MagicMock()
        handler.send_header = MagicMock()
        handler.end_headers = MagicMock()
        handler.wfile = MagicMock()

        with patch('server.validate_directory_path', return_value=(True, str(test_file))):
            handler.do_GET()

        # 验证文件下载响应
        handler.send_response.assert_called_with(200)
        handler.send_header.assert_any_call('Content-Type', 'application/octet-stream')
        handler.send_header.assert_any_call('Content-Disposition', 'attachment; filename="download.txt"')
```

2. **Green - 实现功能** (src/server.py):
```python
def _serve_file(self, file_path: str):
    """流式传输单个文件。"""
    filename = os.path.basename(file_path)

    try:
        file_size = os.path.getsize(file_path)

        self.send_response(200)
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Disposition', f'attachment; filename="{filename}"')
        self.send_header('Content-Length', str(file_size))
        self.end_headers()

        # 流式传输
        with open(file_path, 'rb') as f:
            while True:
                chunk = f.read(8192)
                if not chunk:
                    break
                self.wfile.write(chunk)
    except Exception as e:
        self.send_error(500, "Internal server error")
```

3. **Refactor**:
   - 复用现有的文件传输逻辑
   - 优化chunk大小

**验收标准**:
- 流式传输文件
- 正确的Content-Disposition头
- 处理大文件

---

### T-010: DirectoryShareHandler - Zip下载

**优先级**: P0
**预计时间**: 1.5h
**依赖**: T-007, T-008
**关联需求**: 需求4 - 整目录下载为Zip
**关联功能**: F-003

**目标**: 实现整个目录的zip下载。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_server.py):
```python
def test_directory_handler_zip_response(tmp_path):
    """测试DirectoryShareHandler的zip下载"""
    from server import DirectoryShareHandler

    test_dir = tmp_path / "shared"
    test_dir.mkdir()
    (test_dir / "file1.txt").write_text("content1")

    mock_server = MagicMock()
    mock_server.directory_path = str(test_dir)

    with patch.object(BaseHTTPRequestHandler, '__init__', return_value=None):
        handler = DirectoryShareHandler(MagicMock(), ('127.0.0.1', 12345), mock_server)
        handler.path = "/?download=zip"
        handler.server = mock_server
        handler.send_response = MagicMock()
        handler.send_header = MagicMock()
        handler.end_headers = MagicMock()
        handler.wfile = io.BytesIO()

        with patch('server.validate_directory_path', return_value=(True, str(test_dir))):
            handler.do_GET()

        # 验证zip响应
        handler.send_response.assert_called_with(200)
        handler.send_header.assert_any_call('Content-Type', 'application/zip')
```

2. **Green - 实现功能** (src/server.py):
```python
from .directory_handler import stream_directory_as_zip

def _serve_directory_zip(self, base_dir: str, target_dir: str):
    """流式生成并传输目录zip。"""
    zip_filename = os.path.basename(base_dir) + '.zip'

    try:
        self.send_response(200)
        self.send_header('Content-Type', 'application/zip')
        self.send_header('Content-Disposition', f'attachment; filename="{zip_filename}"')
        self.send_header('Transfer-Encoding', 'chunked')
        self.end_headers()

        # 流式生成zip
        stream_directory_as_zip(self.wfile, base_dir, target_dir)
    except Exception as e:
        self.send_error(500, "Failed to generate zip")
```

3. **Refactor**:
   - 错误处理
   - 日志记录

**验收标准**:
- 返回有效zip文件
- 使用chunked传输
- 不占用大量内存

---

### T-011: DirectoryShareServer 基础

**优先级**: P0
**预计时间**: 2h
**依赖**: T-008, T-009, T-010
**关联需求**: 需求7 - 命令行选项适用于目录

**目标**: 实现 DirectoryShareServer 类。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_server.py):
```python
def test_directory_share_server_init():
    """测试DirectoryShareServer初始化"""
    from server import DirectoryShareServer

    with patch('server.find_available_port', return_value=8080):
        server = DirectoryShareServer("/tmp/test", port=8080, max_sessions=5)

        assert server.directory_path == "/tmp/test"
        assert server.port == 8080
        assert server.max_sessions == 5
        assert server.httpd is None

def test_directory_share_server_start():
    """测试DirectoryShareServer启动"""
    from server import DirectoryShareServer

    with patch('server.find_available_port', return_value=8080), \
         patch('server.ThreadingHTTPServer') as mock_httpd_cls:

        mock_httpd = MagicMock()
        mock_httpd_cls.return_value = mock_httpd

        server = DirectoryShareServer("/tmp/test")

        with patch('threading.Thread') as mock_thread_cls, \
             patch('threading.Timer') as mock_timer_cls:

            mock_thread = MagicMock()
            mock_thread_cls.return_value = mock_thread
            mock_timer = MagicMock()
            mock_timer_cls.return_value = mock_timer

            server.start()

            # 验证HTTPServer创建
            mock_httpd_cls.assert_called()

            # 验证属性注入
            assert mock_httpd.directory_path == "/tmp/test"

            # 验证线程启动
            mock_thread.start.assert_called()
            mock_timer.start.assert_called()
```

2. **Green - 实现功能** (src/server.py):
```python
class DirectoryShareServer:
    """目录共享服务器，支持会话管理。"""

    def __init__(
        self,
        directory_path: str,
        port: Optional[int] = None,
        timeout_minutes: int = 30,
        max_sessions: int = 10
    ):
        self.directory_path = os.path.abspath(directory_path)
        self.port = find_available_port(custom_port=port) if port else find_available_port()
        self.timeout_minutes = timeout_minutes
        self.max_sessions = max_sessions

        # 会话管理（待实现）
        self.sessions = {}
        self.session_lock = threading.Lock()

        self.httpd: Optional[HTTPServer] = None
        self.server_thread: Optional[threading.Thread] = None
        self.shutdown_timer: Optional[threading.Timer] = None

    def start(self):
        """启动服务器。"""
        self.httpd = ThreadingHTTPServer(('', self.port), DirectoryShareHandler)

        # 注入配置到handler
        self.httpd.directory_path = self.directory_path
        self.httpd.session_manager = self

        self.server_thread = threading.Thread(target=self.httpd.serve_forever)
        self.server_thread.daemon = True
        self.server_thread.start()

        # 超时自动停止
        self.shutdown_timer = threading.Timer(
            self.timeout_minutes * 60,
            self._shutdown_server
        )
        self.shutdown_timer.start()

    def stop(self):
        """停止服务器。"""
        self._shutdown_server()

    def _shutdown_server(self):
        """内部停止逻辑。"""
        if self.shutdown_timer:
            self.shutdown_timer.cancel()

        if self.httpd:
            self.httpd.shutdown()
            self.httpd.server_close()
```

3. **Refactor**:
   - 提取公共逻辑（与FileShareServer）

**验收标准**:
- 正确初始化
- 启动HTTP服务器
- 支持超时停止

---

### T-012: DirectoryShareServer 超时

**优先级**: P0
**预计时间**: 1h
**依赖**: T-011
**关联需求**: 需求7 - 命令行选项适用于目录

**目标**: 验证超时机制。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_server.py):
```python
def test_directory_server_timeout():
    """测试DirectoryShareServer超时停止"""
    from server import DirectoryShareServer

    with patch('server.find_available_port', return_value=8080), \
         patch('server.ThreadingHTTPServer'):

        server = DirectoryShareServer("/tmp/test", timeout_minutes=0.01)  # 0.6秒

        with patch('threading.Thread'), \
             patch('threading.Timer') as mock_timer_cls:

            mock_timer = MagicMock()
            mock_timer_cls.return_value = mock_timer

            server.start()

            # 验证Timer被创建，时间正确
            mock_timer_cls.assert_called()
            call_args = mock_timer_cls.call_args
            assert call_args[0][0] == 0.01 * 60  # 转换为秒
```

2. **Green - 验证实现**:
   - 现有实现应已满足
   - 运行测试

3. **Refactor**:
   - 无需修改

**验收标准**:
- 超时后自动停止
- 可手动停止

---

## Phase 4: Main层集成

### T-013: 路径类型检测

**优先级**: P0
**预计时间**: 1h
**依赖**: 无
**关联需求**: 需求1 - 自动检测文件类型
**关联测试**: test_should_detect_directory_path, test_should_detect_file_path

**目标**: 实现 validate_path() 函数。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_main.py):
```python
def test_should_detect_directory_path(tmp_path):
    """测试检测目录路径"""
    test_dir = tmp_path / "testdir"
    test_dir.mkdir()
    (test_dir / "file.txt").write_text("content")

    path_obj, path_type, size = validate_path(str(test_dir))

    assert path_type == 'directory'
    assert path_obj.is_dir()
    assert size > 0

def test_should_detect_file_path(tmp_path):
    """测试检测文件路径"""
    test_file = tmp_path / "test.txt"
    test_file.write_text("a" * 100)

    path_obj, path_type, size = validate_path(str(test_file))

    assert path_type == 'file'
    assert path_obj.is_file()
    assert size == 100

def test_should_error_on_invalid_path():
    """测试无效路径"""
    with pytest.raises(FileNotFoundError):
        validate_path("/nonexistent/path")
```

2. **Green - 实现功能** (src/main.py):
```python
def validate_path(path: str) -> Tuple[Path, str, int]:
    """
    验证路径并返回类型和大小信息。

    Returns:
        (Path对象, 类型('file'|'directory'), 大小字节数)
    """
    path_obj = Path(path).resolve()

    if not path_obj.exists():
        raise FileNotFoundError(f"Path not found: {path}")

    if path_obj.is_file():
        return path_obj, 'file', path_obj.stat().st_size
    elif path_obj.is_dir():
        # 计算目录总大小
        total_size = sum(
            f.stat().st_size
            for f in path_obj.rglob('*')
            if f.is_file()
        )
        return path_obj, 'directory', total_size
    else:
        raise ValueError(f"Unsupported path type: {path}")
```

3. **Refactor**:
   - 性能优化（大目录）
   - 错误处理

**验收标准**:
- 正确检测文件
- 正确检测目录
- 计算目录大小

---

### T-014: validate_path 统一函数

**优先级**: P1
**预计时间**: 1h
**依赖**: T-013
**关联需求**: 需求6 - 向后兼容单文件共享

**说明**: 已在 T-013 实现，此任务是重构现有 validate_file。

**TDD步骤**:

1. **Red - 编写测试确保向后兼容**:
```python
def test_validate_path_backward_compatible(tmp_path):
    """测试validate_path向后兼容validate_file"""
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    # 新函数
    path_obj, path_type, size = validate_path(str(test_file))

    # 旧函数（如果仍存在）
    # old_path, old_size = validate_file(str(test_file))

    # 验证等效
    assert path_type == 'file'
    assert size == len("content")
```

2. **Green - 保持兼容**:
   - validate_path 已实现
   - validate_file 可保留或标记废弃

3. **Refactor**:
   - 文档化迁移路径

**验收标准**:
- 向后兼容
- 所有现有测试通过

---

### T-015: main() Server分发

**优先级**: P0
**预计时间**: 2h
**依赖**: T-013, T-011
**关联需求**: 需求1 - 自动检测文件类型

**目标**: 修改 main() 函数，根据路径类型选择Server。

**TDD步骤**:

1. **Red - 编写集成测试** (tests/test_integration.py):
```python
def test_directory_share_custom_port(tmp_path):
    """测试目录共享使用自定义端口"""
    test_dir = tmp_path / "shared"
    test_dir.mkdir()
    (test_dir / "file.txt").write_text("content")

    with patch('main.DirectoryShareServer') as mock_server_cls:
        server_instance = MagicMock()
        server_instance.server_thread = None
        mock_server_cls.return_value = server_instance

        with patch('sys.argv', ['quick-share', str(test_dir), '-p', '9090']):
            with patch('sys.stdout', io.StringIO()):
                main()

        # 验证DirectoryShareServer被调用
        mock_server_cls.assert_called_once()
        call_kwargs = mock_server_cls.call_args[1]
        assert call_kwargs['directory_path'] == str(test_dir)
        assert call_kwargs['port'] == 9090

def test_directory_share_custom_limits(tmp_path):
    """测试目录共享的会话限制"""
    test_dir = tmp_path / "shared"
    test_dir.mkdir()

    with patch('main.DirectoryShareServer') as mock_server_cls:
        server_instance = MagicMock()
        server_instance.server_thread = None
        mock_server_cls.return_value = server_instance

        with patch('sys.argv', ['quick-share', str(test_dir), '-n', '5', '-t', '10m']):
            with patch('sys.stdout', io.StringIO()):
                main()

        call_kwargs = mock_server_cls.call_args[1]
        assert call_kwargs['max_sessions'] == 5
        assert call_kwargs['timeout_minutes'] == 10.0
```

2. **Green - 实现功能** (src/main.py):
```python
from .server import FileShareServer, DirectoryShareServer

def main() -> None:
    """主执行流程。"""
    try:
        args = parse_arguments()
        validate_arguments(args)

        # 验证路径（新函数）
        path_obj, path_type, size_bytes = validate_path(args.file_path)

        # 获取网络信息
        local_ip = get_local_ip()
        port = find_available_port(custom_port=args.port)

        # 解析超时
        timeout_seconds = parse_duration(args.timeout)
        server_timeout_minutes = timeout_seconds / 60

        # 根据类型选择Server
        if path_type == 'file':
            # 现有单文件逻辑
            server = FileShareServer(
                file_path=str(path_obj),
                port=port,
                timeout_minutes=server_timeout_minutes
            )

            # 打印启动消息（文件）
            msg = logger.format_startup_message(
                ip=local_ip,
                port=port,
                filename=path_obj.name,
                file_size=format_file_size(size_bytes),
                max_downloads=args.max_downloads,
                timeout=timeout_seconds
            )
            print(msg)

        else:  # directory
            server = DirectoryShareServer(
                directory_path=str(path_obj),
                port=port,
                timeout_minutes=server_timeout_minutes,
                max_sessions=args.max_downloads
            )

            # 打印启动消息（目录）
            # TODO: 更新logger支持目录格式
            print(f"Sharing directory: {path_obj}")
            print(f"Browse at: http://{local_ip}:{port}/")
            print(f"Max sessions: {args.max_downloads}")
            print(f"Timeout: {timeout_seconds}s")

        # 启动服务器
        server.start()

        # 等待完成
        while server.server_thread and server.server_thread.is_alive():
            server.server_thread.join(timeout=0.5)

    except KeyboardInterrupt:
        print("\nStopping server...")
        server.stop()
        sys.exit(0)
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)
```

3. **Refactor**:
   - 提取启动消息生成逻辑
   - 统一错误处理

**验收标准**:
- 文件路径使用FileShareServer
- 目录路径使用DirectoryShareServer
- 所有CLI参数正确传递

---

## Phase 5: 会话管理

### T-016: 会话跟踪机制

**优先级**: P0
**预计时间**: 2h
**依赖**: T-011
**关联需求**: 需求5 - 按会话计数的限制
**关联测试**: test_session_based_download_counting

**目标**: 实现会话跟踪和cookie管理。

**TDD步骤**:

1. **Red - 编写失败测试** (tests/test_server.py):
```python
def test_session_based_download_counting():
    """测试基于会话的计数"""
    from server import DirectoryShareServer

    server = DirectoryShareServer("/tmp/test", max_sessions=2)

    # 模拟handler
    handler1 = MagicMock()
    handler1.client_address = ('192.168.1.10', 12345)
    handler1.headers.get.return_value = ''  # 无cookie

    handler2 = MagicMock()
    handler2.client_address = ('192.168.1.20', 12346)
    handler2.headers.get.return_value = ''

    # 第一个会话
    allowed1 = server.track_session(handler1)
    assert allowed1 == True

    # 第二个会话
    allowed2 = server.track_session(handler2)
    assert allowed2 == True

    # 第三个会话（应被拒绝）
    handler3 = MagicMock()
    handler3.client_address = ('192.168.1.30', 12347)
    handler3.headers.get.return_value = ''

    allowed3 = server.track_session(handler3)
    assert allowed3 == False

    # 已有会话重复访问（应允许）
    allowed1_again = server.track_session(handler1)
    assert allowed1_again == True
```

2. **Green - 实现功能** (src/server.py):
```python
import uuid
import time

class DirectoryShareServer:
    # ... 现有代码 ...

    def track_session(self, request_handler) -> bool:
        """
        跟踪会话，返回是否允许访问。
        """
        with self.session_lock:
            # 从cookie获取session_id
            session_id = self._get_or_create_session_id(request_handler)

            # 检查会话数量
            if session_id not in self.sessions:
                if len(self.sessions) >= self.max_sessions:
                    return False  # 新会话，拒绝

                # 记录新会话
                self.sessions[session_id] = {
                    'ip': request_handler.client_address[0],
                    'start_time': time.time()
                }

            return True

    def _get_or_create_session_id(self, request_handler) -> str:
        """从cookie获取或创建新的session_id。"""
        cookie_header = request_handler.headers.get('Cookie', '')

        # 简单的session_id提取
        if 'session_id=' in cookie_header:
            for part in cookie_header.split(';'):
                if 'session_id=' in part:
                    return part.split('=')[1].strip()

        # 创建新session_id
        new_id = str(uuid.uuid4())

        # 设置Set-Cookie响应头（在handler中处理）
        if hasattr(request_handler, 'session_id_to_set'):
            request_handler.session_id_to_set = new_id

        return new_id
```

在 DirectoryShareHandler 中添加 cookie 设置:

```python
class DirectoryShareHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        # 会话跟踪
        if hasattr(self.server, 'session_manager'):
            allowed = self.server.session_manager.track_session(self)
            if not allowed:
                self.send_error(403, "Session limit reached")
                return

        # ... 现有逻辑 ...

    def send_response(self, code, message=None):
        """重写send_response以添加Set-Cookie。"""
        super().send_response(code, message)

        # 如果需要设置cookie
        if hasattr(self, 'session_id_to_set'):
            self.send_header('Set-Cookie', f'session_id={self.session_id_to_set}; Path=/; HttpOnly')
```

3. **Refactor**:
   - 优化cookie解析
   - 添加会话过期清理

**验收标准**:
- 正确识别新会话和现有会话
- Cookie正确设置和解析
- 会话限制生效

---

### T-017: 会话限制执行

**优先级**: P0
**预计时间**: 1h
**依赖**: T-016
**关联需求**: 需求5 - 按会话计数的限制
**关联测试**: test_server_stops_after_session_limit

**目标**: 验证会话限制达到时的行为。

**TDD步骤**:

1. **Red - 编写集成测试** (tests/test_integration.py):
```python
def test_server_stops_after_session_limit(tmp_path):
    """测试达到会话限制后服务器停止"""
    test_dir = tmp_path / "shared"
    test_dir.mkdir()

    # 这是集成测试，可能需要实际启动服务器
    # 或者mock会话管理器

    with patch('main.DirectoryShareServer') as mock_server_cls:
        server_instance = MagicMock()
        server_instance.server_thread = None
        server_instance.sessions = {}
        server_instance.max_sessions = 2

        # 模拟会话填满
        def mock_track(handler):
            if len(server_instance.sessions) >= 2:
                return False
            server_instance.sessions[str(len(server_instance.sessions))] = {}
            return True

        server_instance.track_session = mock_track
        mock_server_cls.return_value = server_instance

        # 测试逻辑...
```

2. **Green - 验证实现**:
   - T-016 的实现应已覆盖
   - 运行测试

3. **Refactor**:
   - 确保服务器在会话限制时提示用户

**验收标准**:
- 会话限制达到时拒绝新会话
- 显示友好错误消息

---

## Phase 6: 回归测试

### T-018: 单文件共享回归测试

**优先级**: P0
**预计时间**: 1h
**依赖**: T-015
**关联需求**: 需求6 - 向后兼容单文件共享
**关联测试**: test_single_file_sharing_unchanged
**关联功能**: F-007

**目标**: 确保单文件共享功能不受影响。

**TDD步骤**:

1. **Red - 编写回归测试** (tests/test_integration.py):
```python
def test_single_file_sharing_unchanged(tmp_path):
    """测试单文件共享行为不变"""
    test_file = tmp_path / "test.txt"
    test_file.write_text("test content")

    with patch('main.FileShareServer') as mock_server_cls:
        server_instance = MagicMock()
        server_instance.server_thread = None
        mock_server_cls.return_value = server_instance

        with patch('sys.argv', ['quick-share', str(test_file), '-n', '10']):
            with patch('sys.stdout', io.StringIO()):
                main()

        # 验证使用FileShareServer（不是DirectoryShareServer）
        mock_server_cls.assert_called_once()
        call_kwargs = mock_server_cls.call_args[1]
        assert 'file_path' in call_kwargs
        assert call_kwargs['file_path'] == str(test_file)

        # max_downloads应传递给FileShareServer（如果支持）
        # 目前FileShareServer不支持max_downloads，但应不影响其运行
```

2. **Green - 运行所有现有测试**:
```bash
pytest tests/test_integration.py -k "not directory"
```

3. **Refactor**:
   - 确保所有旧测试通过
   - 更新文档

**验收标准**:
- 所有现有单文件测试通过
- 行为与之前版本一致
- 性能无退化

---

### T-019: CLI选项兼容性测试

**优先级**: P1
**预计时间**: 1h
**依赖**: T-015
**关联需求**: 需求7 - 命令行选项适用于目录
**关联功能**: F-015

**目标**: 验证所有CLI选项在目录模式下正常工作。

**TDD步骤**:

1. **Red - 编写测试** (tests/test_cli.py):
```python
def test_cli_help_mentions_directories():
    """测试--help提到目录支持"""
    parser = parse_arguments(['--help'])
    # 这会触发SystemExit，需要捕获

    # 或者检查帮助字符串
    import io
    from contextlib import redirect_stdout

    f = io.StringIO()
    try:
        with redirect_stdout(f):
            parse_arguments(['--help'])
    except SystemExit:
        pass

    help_text = f.getvalue()
    assert 'directory' in help_text.lower() or 'folder' in help_text.lower()
```

2. **Green - 更新帮助文本** (src/cli.py):
```python
parser.add_argument(
    "file_path",
    help="Path to the file or directory to share"
)

parser.add_argument(
    "-n", "--max-downloads",
    type=int,
    default=10,
    help="Maximum downloads (for files) or sessions (for directories) allowed (default: 10)"
)
```

3. **Refactor**:
   - 确保帮助文本清晰
   - 更新README

**验收标准**:
- 帮助文本准确
- 所有选项在目录模式下工作
- 文档更新

---

## 任务优先级总结

### P0 (必须完成，MVP)
- T-001, T-002, T-003: 安全层
- T-004, T-005, T-007: 核心处理器
- T-008, T-009, T-010, T-011, T-012: Server层
- T-013, T-015: Main层集成
- T-016, T-017: 会话管理
- T-018: 回归测试

### P1 (重要但可延后)
- T-014: 向后兼容重构
- T-019: CLI更新

### P2 (边缘场景，可选)
- 空目录处理（包含在T-005）
- 大文件列表分页（设计中提到，暂不实现）
- 特殊字符处理（包含在T-005）

---

## 估算总结

- **总任务数**: 19
- **P0任务**: 17
- **P1任务**: 2
- **预计总工时**: 27小时
- **建议Sprint周期**: 2-3天（全职开发）

---

**文档完成日期**: 2026-01-14
**审核状态**: 待审核
**下一步**: 用户确认后进入实施阶段
