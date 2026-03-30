---
feature: multi-path-share
stage: tasks
generated_at: 2026-03-30T00:00:00Z
version: 1
---

# 任务拆分: 多路径分享 (Multi-Path Share)

> **功能标识**: multi-path-share
> **总任务数**: 9
> **并行组数**: 4

---

## 任务总览

| ID | 标题 | 优先级 | 复杂度 | 并行组 | 依赖 |
|----|------|--------|--------|--------|------|
| T-001 | CLI 多路径参数支持 | P0 | simple | G1 | - |
| T-002 | 多路径验证与冲突检测 | P0 | medium | G1 | - |
| T-003 | security.py 多路径安全验证 | P0 | medium | G1 | - |
| T-004 | directory_handler.py 多路径数据函数 | P0 | medium | G2 | T-003 |
| T-005 | MultiShareServer 和 MultiShareHandler | P0 | complex | G3 | T-002, T-003, T-004 |
| T-006 | 多路径 SPA 模板 | P0 | medium | G2 | - |
| T-007 | main.py 入口层集成 | P0 | medium | G3 | T-001, T-002, T-005 |
| T-008 | 多路径 Legacy HTML 模板 | P1 | simple | G2 | - |
| T-009 | 集成测试与端到端验证 | P1 | medium | G4 | T-005, T-006, T-007, T-008 |

---

## 并行执行分组

```
G1（并行）: T-001, T-002, T-003
G2（并行）: T-004, T-006, T-008
G3（并行）: T-005, T-007
G4（串行）: T-009
```

---

## 任务详情

### T-001: CLI 多路径参数支持

**文件**: `src/cli.py`
**优先级**: P0
**复杂度**: simple
**并行组**: G1

**描述**:

将 `parse_arguments()` 中的 `file_path` 参数（单值 positional）改为 `file_paths`（`nargs='+'`），支持接收多个路径。

**具体变更**:

1. 在 `parse_arguments()` 中：
   - 将参数名从 `file_path` 改为 `file_paths`
   - 添加 `nargs='+'`
   - 更新 help 文本为 `"One or more files or directories to share"`

2. `validate_arguments()` 不需要改动（仅验证 port/timeout/max_downloads）

**验收标准**:

- `parse_arguments(['file1.txt', 'file2.pdf'])` 返回 `args.file_paths == ['file1.txt', 'file2.pdf']`
- `parse_arguments(['single.txt'])` 返回 `args.file_paths == ['single.txt']`
- `parse_arguments([])` 触发 argparse 错误（保持现有行为）
- `parse_arguments(['a.txt', 'b.txt', 'c/'])` 返回 3 个元素的列表

**测试**:
- `tests/test_cli.py` 中新增/修改对应测试用例
- 确保现有的 `test_parse_arguments` 用例在适配后仍然通过（`file_path` → `file_paths`）

---

### T-002: 多路径验证与冲突检测

**文件**: `src/main.py`
**优先级**: P0
**复杂度**: medium
**并行组**: G1

**描述**:

在 `main.py` 中新增 `validate_multi_paths()` 函数，支持对多个路径进行统一验证，并检测顶层同名冲突。

**具体实现**:

```python
from typing import List, Tuple

def validate_multi_paths(
    paths: List[str]
) -> Tuple[bool, List[Tuple[str, str]], List[str]]:
    """
    Validate multiple paths and detect name conflicts.

    Args:
        paths: List of path strings from CLI

    Returns:
        Tuple of:
        - is_valid: True if all paths valid and no conflicts
        - resolved_paths: List of (abs_path, path_type) tuples
        - errors: List of error messages
    """
```

**验证逻辑**:

1. 对每个 path 调用现有的 `validate_path(path)` 得到 `(is_valid, path_type, resolved_path)`
2. 处理 symlink 取消场景（path_type in ('symlink_broken', 'symlink_cancelled')）
3. 收集所有无效路径错误（不存在、无权限等）
4. 对有效路径做同名检测：提取 `os.path.basename(resolved_path)` 检查重复
5. 冲突时错误信息格式：`"Name conflict: 'test.txt' appears in both '/abs/path/a/test.txt' and '/abs/path/b/test.txt'"`
6. 有任何错误返回 `(False, [], errors)`，全部通过返回 `(True, resolved_paths, [])`

**验收标准**:

- 传入 `['/valid/file.txt', '/valid/dir/']` 返回 `(True, [...], [])`
- 传入 `['/nonexistent.txt']` 返回 `(False, [], ['Error: ...nonexistent.txt...'])`
- 传入 `['/a/test.txt', '/b/test.txt']` 返回 `(False, [], ['Name conflict: test.txt...'])`
- 传入 `['/a/docs/', '/b/docs/']` 返回同名目录冲突错误
- 传入单个有效路径 `['/file.txt']` 返回 `(True, [('/file.txt', 'file')], [])`

**测试**:
- `tests/test_main.py` 中新增测试，使用 `tmp_path` fixture 创建临时文件

---

### T-003: security.py 多路径安全验证

**文件**: `src/security.py`
**优先级**: P0
**复杂度**: medium
**并行组**: G1

**描述**:

新增 `validate_multi_share_path()` 函数，验证多路径分享场景下的文件下载请求路径，防止路径遍历攻击。

**函数签名**:

```python
from typing import List, Tuple

def validate_multi_share_path(
    request_path: str,
    shared_paths: List[Tuple[str, str]]
) -> Tuple[bool, str]:
    """
    Validate a download request path in multi-share context.

    The virtual filesystem uses /files/ prefix:
      /files/<top_name>              → top-level file
      /files/<top_dir>/<sub_path>   → file inside a shared directory

    Args:
        request_path: HTTP request path (e.g., /files/foo.txt or /files/dir1/sub/bar.txt)
        shared_paths: List of (abs_path, path_type) from MultiShareServer

    Returns:
        (is_valid, real_abs_path)
    """
```

**实现逻辑**:

1. 检查 `request_path` 以 `/files/` 开头
2. 提取去掉前缀后的虚拟路径 `virtual_path`（e.g., `dir1/sub/bar.txt`）
3. 路径遍历检测：调用 `is_path_traversal_attack(virtual_path)`
4. 提取 `top_name`（第一级路径组件）
5. 在 `shared_paths` 中查找 `os.path.basename(abs_path) == top_name`
6. 若未找到：返回 `(False, "")`
7. 若为文件类型：
   - 验证 `virtual_path == top_name`（无子路径）
   - 返回 `(True, abs_path)`
8. 若为目录类型：
   - 提取 `sub_path = virtual_path[len(top_name)+1:]` （去掉 `top_name/` 前缀）
   - 构造候选路径：`os.path.join(abs_dir, sub_path)`
   - 调用 `os.path.realpath()` 解析
   - 用 `os.path.commonpath()` 验证在 `abs_dir` 内
   - 验证文件存在
   - 返回 `(True, real_path)` 或 `(False, "")`

**安全验收标准**:

- `/files/foo.txt` 对应共享文件 `foo.txt` → 返回 `(True, '/abs/foo.txt')`
- `/files/dir1/sub/bar.txt` 对应共享目录 `dir1` → 返回 `(True, '/abs/dir1/sub/bar.txt')`
- `/files/../etc/passwd` → 返回 `(False, "")` （路径遍历）
- `/files/notexist.txt` → 返回 `(False, "")` （未共享）
- `/files/dir1/../../../etc/passwd` → 返回 `(False, "")` （目录穿越）
- `/files/foo.txt/extra` → 返回 `(False, "")` （文件有子路径）

**测试**:
- `tests/test_security.py` 中新增测试，覆盖所有安全场景

---

### T-004: directory_handler.py 多路径数据函数

**文件**: `src/directory_handler.py`
**优先级**: P0
**复杂度**: medium
**并行组**: G2
**依赖**: T-003

**描述**:

在 `directory_handler.py` 中新增两个函数：
1. `get_multi_share_root_structure()` - 返回虚拟根目录列表
2. `stream_multi_paths_as_zip()` - 流式 ZIP 多路径内容

**函数 1**: `get_multi_share_root_structure()`

```python
def get_multi_share_root_structure(
    paths: List[Tuple[str, str]]
) -> Dict:
    """
    Get the virtual root structure for multi-path sharing.

    Returns the top-level items (files and directories) without recursion.
    Compatible with the existing get_directory_structure() response format.

    Args:
        paths: List of (abs_path, path_type)

    Returns:
        {"path": "/", "items": [...]}
    """
```

每项包含：`name`, `type`, `size`（目录为0），`modified`（ISO格式）。
排序：目录优先，同类型按名称字母序。

**函数 2**: `stream_multi_paths_as_zip()`

```python
def stream_multi_paths_as_zip(
    output_stream,
    paths: List[Tuple[str, str]],
    progress_callback: bool = False
) -> None:
    """
    Stream multiple files/directories as a single ZIP archive.

    ZIP structure:
      file1.txt       (top-level file)
      dir1/           (directory, preserving internal structure)
      dir1/sub.txt
      file2.pdf

    Args:
        output_stream: HTTP response wfile
        paths: List of (abs_path, path_type)
        progress_callback: If True, print progress logs
    """
```

实现要点：
- 复用现有 `zipfile.ZipFile` 流式写入模式
- 文件：`zipf.write(abs_path, arcname=os.path.basename(abs_path))`
- 目录：`os.walk(abs_path)`，`arcname = basename + '/' + os.path.relpath(file_path, abs_path)`
- 进度日志参考现有 `stream_directory_as_zip()` 实现

**验收标准**:

- `get_multi_share_root_structure([(f1, 'file'), (d1, 'directory')])` 返回 2 项，目录排前
- ZIP 中包含顶层文件和目录内文件（保留目录结构）
- 空目录列表不崩溃，返回空 items
- ZIP 流式写入不在内存中积累（使用 wfile 直接写）

**测试**:
- `tests/test_directory_handler.py` 新增测试
- ZIP 测试使用 `io.BytesIO` 作为 mock stream

---

### T-005: MultiShareServer 和 MultiShareHandler

**文件**: `src/server.py`
**优先级**: P0
**复杂度**: complex
**并行组**: G3
**依赖**: T-002, T-003, T-004

**描述**:

在 `server.py` 中新增 `MultiShareServer` 和 `MultiShareHandler` 两个类，实现多路径 HTTP 服务。

**`MultiShareServer` 类**:

```python
class MultiShareServer:
    """
    Managed HTTP server for sharing multiple files/directories.
    Shows a unified file list page regardless of the number of paths.
    """

    def __init__(
        self,
        paths: List[Tuple[str, str]],
        port: Optional[int] = None,
        timeout_minutes: int = 30,
        max_sessions: int = 10,
        legacy_mode: bool = False
    ):
```

- 完全复用 `DirectoryShareServer` 的 session 管理（`track_session`, `_extract_session_id_from_cookie`）
- `start()` 方法将 `paths`, `legacy_mode` 等注入到 `httpd` 实例

**`MultiShareHandler` 类**:

继承 `BaseHTTPRequestHandler`，实现 `do_GET()`：

```python
def do_GET(self):
    # 1. Session tracking (复用 DirectoryShareHandler 逻辑)
    # 2. Route dispatch:
    if self.path == '/' or self.path.startswith('/?'):
        self._serve_root_page()
    elif self.path.startswith('/api/tree'):
        self._handle_api_tree()
    elif self.path.startswith('/api/content'):
        self._handle_api_content()
    elif self.path.startswith('/files/'):
        self._serve_file_download()
    elif self.path.startswith('/download/all.zip') or ...:
        self._serve_all_as_zip()
    else:
        self.send_error(404, "Not found")
```

**`_serve_root_page()`**:
- `legacy=1` → `generate_multi_share_legacy_html(server.paths)`
- 否则 → `generate_multi_share_spa_html(len(server.paths))`

**`_handle_api_tree()`**:
- `path=/` → `get_multi_share_root_structure(server.paths)`
- `path=/dirname` → 找到对应共享目录，调用 `get_directory_structure(abs_dir, sub_path)`

**`_handle_api_content()`**:
- 调用 `validate_multi_share_path('/files/' + path.lstrip('/'), server.paths)` 获取真实路径
- 复用 `DirectoryShareHandler._handle_api_content()` 的文件读取逻辑

**`_serve_file_download()`**:
- 调用 `validate_multi_share_path(self.path, server.paths)`
- 验证通过 → `_stream_file_with_headers(real_path, filename)`
- 验证失败 → 403

**`_serve_all_as_zip()`**:
- 调用 `stream_multi_paths_as_zip(self.wfile, server.paths, progress_callback=True)`
- 响应头：`Content-Type: application/zip`, `Content-Disposition: attachment; filename="quick-share.zip"`

**验收标准**:

- `GET /` 返回 200 HTML
- `GET /api/tree?path=/` 返回所有共享项的 JSON
- `GET /api/tree?path=/dirname` 返回该目录内容
- `GET /files/foo.txt` 下载顶层文件（正确内容）
- `GET /files/dir1/sub.txt` 下载目录内文件
- `GET /download/all.zip` 返回 ZIP（含所有文件）
- `GET /files/../etc/passwd` 返回 403
- `GET /files/notexist` 返回 403 或 404
- Session 计数：超出 `max_sessions` 后返回 403

**测试**:
- `tests/test_server.py` 新增 `MultiShareHandler` 测试
- 使用 `unittest.mock` mock handler 的 server 属性

---

### T-006: 多路径 SPA 模板

**文件**: `src/templates.py`
**优先级**: P0
**复杂度**: medium
**并行组**: G2

**描述**:

在 `templates.py` 中新增 `generate_multi_share_spa_html()` 函数，生成多路径场景的前端 SPA 页面。

**函数签名**:

```python
def generate_multi_share_spa_html(item_count: int) -> str:
    """
    Generate SPA HTML for multi-path sharing.

    Args:
        item_count: Number of shared items (for title display)

    Returns:
        Full HTML string
    """
```

**与现有 `generate_spa_html()` 的差异**:

1. **标题**: `Quick Share - {item_count} items` （而非目录名）
2. **ZIP 下载按钮**: `href="/download/all.zip"` （而非 `/?download=zip`）
3. **文件下载链接**: 构造为 `/files/` + 虚拟路径（而非直接路径）
4. **无 Legacy View 按钮**（或 `href="/?legacy=1"`）
5. **API 端点**: 保持 `/api/tree?path=/` 和 `/api/content?path=...` 不变

**TreeItem 组件下载链接构造**:

在现有 SPA 中，文件点击触发 `selectItem()` 调用 `/api/content`（预览）。
在多路径 SPA 中，文件点击应**直接下载**（不预览）。

因此：
- 文件项：生成 `<a href="/files/{virtual_path}" download>` 触发下载
- 目录项：展开/折叠（与现有行为相同）

或者：点击文件时直接 `window.location.href = '/files/' + encodedPath`。

**UI 设计要点**:

- 顶部展示 "Quick Share | N items shared"
- "Download All (ZIP)" 主按钮
- 文件夹可展开（树状结构，复用 TreeItem）
- 每个文件项有下载图标/链接

**验收标准**:

- HTML 包含 `<title>Quick Share - N items</title>` 或类似
- HTML 包含 `/download/all.zip` 下载链接
- HTML 包含 Vue 3 初始化和 TreeItem 组件（结构正确）
- 文件点击触发 `/files/` 路径下载（而非预览）
- 目录点击触发 `/api/tree?path=/dirname` 展开

**测试**:
- `tests/test_templates.py`（如不存在则新建）中验证 HTML 包含正确内容

---

### T-007: main.py 入口层集成

**文件**: `src/main.py`
**优先级**: P0
**复杂度**: medium
**并行组**: G3
**依赖**: T-001, T-002, T-005

**描述**:

修改 `main()` 函数，将原有的单路径分发逻辑替换为多路径统一分发逻辑，始终使用 `MultiShareServer`。

**主要变更**:

1. 导入：新增 `from .server import MultiShareServer`，移除或保留 `FileShareServer, DirectoryShareServer`（其他代码仍可能用到）
2. 路径验证：`args.file_path` → `args.file_paths`，调用 `validate_multi_paths(args.file_paths)` 替代原有的 `validate_path(args.file_path)`
3. 错误输出：若 `not is_valid`，打印所有 `errors` 并 `sys.exit(1)`
4. 服务器实例化：始终创建 `MultiShareServer(paths=resolved_paths, ...)`
5. 启动信息打印：
   - 若为单个文件：显示 "Sharing 1 file: filename (size)"
   - 若为多个项目：显示 "Sharing N items: [names...]"
   - 显示 URL（列表页面 URL，不含文件名）

**`format_startup_message` 更新**:

修改 `logger.py` 中的 `format_startup_message()` 函数（或新增重载），支持多路径场景：
- `file_size` 参数改为可接受 `"N items"` 形式（或新增 `item_count` 参数）
- URL 显示为 `http://ip:port/`（列表页面）

**验收标准**:

- `quick-share file1.txt file2.pdf` 启动 `MultiShareServer`，打印包含 URL 的启动信息
- `quick-share single.txt` 同样启动 `MultiShareServer`（列表页面）
- 无效路径时打印错误并退出码 1
- 同名冲突时打印冲突信息并退出码 1
- `-p`, `-n`, `-t`, `--legacy` 参数正常传递给 `MultiShareServer`
- Ctrl+C 正常停止服务器

**测试**:
- `tests/test_main.py` 更新现有测试，新增多路径场景测试

---

### T-008: 多路径 Legacy HTML 模板

**文件**: `src/templates.py` 或 `src/directory_handler.py`
**优先级**: P1
**复杂度**: simple
**并行组**: G2

**描述**:

新增 `generate_multi_share_legacy_html()` 函数，为 `--legacy` 模式提供服务端渲染的多路径文件列表页面。

**函数签名**:

```python
def generate_multi_share_legacy_html(
    paths: List[Tuple[str, str]]
) -> str:
    """
    Generate legacy server-side rendered HTML for multi-path sharing.

    Args:
        paths: List of (abs_path, path_type) tuples

    Returns:
        HTML string with a table listing all shared items
    """
```

**页面内容**:

- 标题：`Quick Share - N items`
- "Download All as ZIP" 按钮（`/download/all.zip`）
- 表格：Name | Type | Size | Download
  - 文件行：文件名 | File | 大小 | `<a href="/files/name" download>Download</a>`
  - 目录行：目录名 | Directory | - | `<a href="/download/all.zip">Download ZIP</a>`（或不提供单目录下载）

**验收标准**:

- HTML 包含所有共享项的名称
- HTML 包含 `/download/all.zip` 链接
- 文件项包含 `/files/<name>` 下载链接
- HTML 是合法的 HTML5（基本格式正确）

**测试**:
- 简单的字符串断言测试，验证关键内容存在

---

### T-009: 集成测试与端到端验证

**文件**: `tests/integration/test_multi_share_integration.py`（新建）
**优先级**: P1
**复杂度**: medium
**并行组**: G4
**依赖**: T-005, T-006, T-007, T-008

**描述**:

编写集成测试，验证多路径分享的端到端流程，包括服务器启动、HTTP 请求、文件下载和 ZIP 打包。

**测试场景**:

1. **多文件分享**:
   - 启动 `MultiShareServer` 共享 3 个文件
   - GET `/` 返回 200 HTML（含 SPA 内容）
   - GET `/api/tree?path=/` 返回 3 个文件的 JSON
   - GET `/files/file1.txt` 返回正确文件内容
   - GET `/download/all.zip` 返回 ZIP（含 3 个文件）

2. **混合文件和目录**:
   - 启动 `MultiShareServer` 共享 1 个文件 + 1 个目录
   - GET `/api/tree?path=/` 返回 2 项（目录排前）
   - GET `/api/tree?path=/dirname` 返回目录内容
   - GET `/files/dirname/subfile.txt` 返回子文件
   - GET `/download/all.zip` ZIP 包含文件和目录内容

3. **单文件统一体验** (R-007):
   - 启动 `MultiShareServer` 共享 1 个文件
   - GET `/` 返回文件列表 HTML（而非直接下载）

4. **会话计数** (R-008):
   - `max_sessions=1`
   - 第一个会话访问成功
   - 第二个会话访问返回 403

5. **安全测试**:
   - GET `/files/../etc/passwd` 返回 403（不含系统文件内容）
   - GET `/files/notexist.txt` 返回 403

**验收标准**:

- 所有 5 个场景的测试通过
- ZIP 内容与共享文件列表完全匹配
- 安全测试不泄露任何非共享路径内容

**测试工具**:
- `tempfile.mkdtemp()` 创建临时目录
- `threading.Thread` 启动测试服务器
- `urllib.request.urlopen()` 发送 HTTP 请求（标准库）
- `zipfile.ZipFile(io.BytesIO(...))` 验证 ZIP 内容

---

## 测试覆盖率目标

| 模块 | 目标覆盖率 |
|------|----------|
| `src/cli.py`（新增部分） | 100% |
| `src/main.py`（新增函数） | 95% |
| `src/security.py`（新增函数） | 100% |
| `src/directory_handler.py`（新增函数） | 90% |
| `src/server.py`（新增类） | 85% |
| `src/templates.py`（新增函数） | 80% |

---

## 关键约束提醒

1. **仅标准库**: 所有实现必须仅使用 Python 3.8+ 标准库（zipfile, threading, http.server 等）
2. **流式传输**: ZIP 下载必须流式传输，不在内存中积累
3. **线程安全**: session 管理使用 `threading.Lock()`（复用现有模式）
4. **向后兼容**: `-p`, `-n`, `-t`, `--legacy` 参数行为不变
5. **TDD 执行顺序**: 每个任务先写测试（Red），再实现（Green），再重构（Refactor）
