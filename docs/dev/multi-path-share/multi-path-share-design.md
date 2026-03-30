---
feature: multi-path-share
stage: design
generated_at: 2026-03-30T00:00:00Z
version: 1
---

# 技术设计文档: 多路径分享 (Multi-Path Share)

> **功能标识**: multi-path-share
> **复杂度**: complex
> **依赖需求**: multi-path-share-requirements.md

---

## 1. 架构概览

### 1.1 设计原则

1. **最大化复用**: 新的 `MultiShareServer` 复用 `DirectoryShareServer` 的 SPA 模板、会话管理、ZIP 流式下载等已验证的机制，仅在数据层面做抽象。
2. **向后兼容**: `DirectoryShareServer` 和 `FileShareServer` 保持不变，多路径分享通过新的 `MultiShareServer` 类实现。
3. **扁平化虚拟文件系统**: 所有传入路径被组织为一个虚拟根目录，接收方看到的是一个扁平的顶层列表，文件夹可向下展开。
4. **标准库约束**: 全程仅使用 Python 3.8+ 标准库。

### 1.2 整体流程

```
用户命令: quick-share file1.txt dir1/ file2.pdf
         │
         ▼
[cli.py] parse_arguments()
  file_paths: ['file1.txt', 'dir1/', 'file2.pdf']  (nargs='+')
         │
         ▼
[main.py] validate_multi_paths()
  - 逐路径检查存在性和可读性
  - 同名冲突检测（顶层 basename 不得重复）
  - 返回 List[Tuple[str, str]]：(resolved_abs_path, path_type)
         │
         ▼
[main.py] MultiShareServer(paths=[...])
         │
         ▼
[server.py] MultiShareHandler
  - / → 生成多路径 SPA HTML（复用 generate_multi_share_spa_html）
  - /api/tree → 虚拟根目录列表 或 子路径列表
  - /api/content → 文件内容预览
  - /files/<name> → 单文件下载（安全沙箱检查）
  - /files/<dir>/<...> → 目录内文件下载
  - /download/all.zip → 全部打包下载（流式 ZIP）
```

### 1.3 组件关系图

```
cli.py                 main.py               server.py
─────────────────      ──────────────────    ─────────────────────────
parse_arguments()  →   validate_multi_paths()
  file_paths           detect_path_type()  → MultiShareServer
  (nargs='+')          check_name_conflicts()   │
                                                ├─ MultiShareHandler
                                                │    ├─ /api/tree
                                                │    ├─ /api/content
                                                │    ├─ /files/...
                                                │    └─ /download/all.zip
                                                └─ track_session()

directory_handler.py   security.py           templates.py
────────────────────   ────────────────       ──────────────────────
stream_multi_paths_   validate_multi_       generate_multi_share_
  as_zip()            path_request()          spa_html()
get_multi_share_
  structure()
```

---

## 2. 详细设计

### 2.1 CLI 层 (`src/cli.py`)

#### 变更点

将 `file_path` 参数（单值 positional）改为 `file_paths`（`nargs='+'` 多值 positional）：

```python
# 变更前
parser.add_argument("file_path", help="Path to the file to share")

# 变更后
parser.add_argument(
    "file_paths",
    nargs='+',
    help="One or more files or directories to share"
)
```

#### 向后兼容性

- `--legacy` 参数保留，沿用到 `MultiShareServer`
- `-p`, `-n`, `-t` 参数保留，语义不变
- Shell 通配符由 Shell 自然展开后传入，`nargs='+'` 直接接收展开结果

#### `validate_arguments()` 扩展

无需修改，`validate_arguments` 只检查 port/timeout/max_downloads，与路径无关。

---

### 2.2 入口层 (`src/main.py`)

#### 新增函数：`validate_multi_paths()`

```python
def validate_multi_paths(
    paths: List[str]
) -> Tuple[bool, List[Tuple[str, str]], List[str]]:
    """
    Validate multiple paths and detect name conflicts.

    Args:
        paths: List of paths from CLI

    Returns:
        Tuple of:
        - is_valid: True if all paths valid and no conflicts
        - resolved_paths: List of (abs_path, path_type) tuples
        - errors: List of error messages
    """
```

**验证逻辑**：

1. 逐路径调用现有 `validate_path(path)` 获取 `(is_valid, path_type, resolved_path)`
2. 收集所有无效路径，生成友好错误信息
3. 同名冲突检测：提取所有 resolved_path 的 `basename`，检查重复
4. 有任何错误 → 返回 `(False, [], [error_msgs])`
5. 全部有效 → 返回 `(True, [(abs_path, type), ...], [])`

**同名冲突检测**：

```python
names = [os.path.basename(p) for p, _ in resolved_paths]
seen = {}
for name, (abs_path, _) in zip(names, resolved_paths):
    if name in seen:
        errors.append(
            f"Name conflict: '{name}' appears in both "
            f"'{seen[name]}' and '{abs_path}'"
        )
    else:
        seen[name] = abs_path
```

#### `main()` 函数变更

```python
# 旧路径：args.file_path
# 新路径：args.file_paths（列表）

is_valid, resolved_paths, errors = validate_multi_paths(args.file_paths)
if not is_valid:
    for err in errors:
        print(f"Error: {err}", file=sys.stderr)
    sys.exit(1)

# 始终使用 MultiShareServer（包括单路径场景）
server = MultiShareServer(
    paths=resolved_paths,
    port=port,
    timeout_minutes=server_timeout_minutes,
    max_sessions=args.max_downloads,
    legacy_mode=args.legacy
)
```

**注意**：单路径（无论文件还是目录）也走 `MultiShareServer`，实现 R-007（单文件统一列表体验）。

---

### 2.3 服务层 (`src/server.py`)

#### 新类：`MultiShareServer`

```python
class MultiShareServer:
    """
    Managed HTTP server for sharing multiple files/directories.
    Always shows a file list page regardless of the number of paths.
    """

    def __init__(
        self,
        paths: List[Tuple[str, str]],  # [(abs_path, path_type), ...]
        port: Optional[int] = None,
        timeout_minutes: int = 30,
        max_sessions: int = 10,
        legacy_mode: bool = False
    ):
        self.paths = paths
        self.port = ...
        self.max_sessions = max_sessions
        self.legacy_mode = legacy_mode
        # Session management (同 DirectoryShareServer)
        self.sessions = {}
        self.session_lock = threading.Lock()
        ...
```

#### 新类：`MultiShareHandler`

继承 `BaseHTTPRequestHandler`，路由如下：

| 路径模式 | 处理逻辑 |
|---------|---------|
| `GET /` | 返回多路径 SPA HTML |
| `GET /?legacy=1` | 返回 legacy HTML（多路径版本）|
| `GET /api/tree?path=/` | 返回虚拟根目录列表 |
| `GET /api/tree?path=/dirname` | 返回目录内容 |
| `GET /api/content?path=/filename` | 返回文件预览内容 |
| `GET /files/filename` | 下载顶层文件 |
| `GET /files/dirname/subpath` | 下载目录内文件 |
| `GET /download/all.zip` | 全部打包 ZIP 下载 |
| 其他 | 403 或 404 |

**虚拟根目录逻辑**：

当请求 `/api/tree?path=/` 时，返回所有 `self.server.paths` 的顶层列表：

```json
{
  "path": "/",
  "items": [
    {"name": "file1.txt", "type": "file", "size": 1024, "modified": "..."},
    {"name": "dir1", "type": "directory", "size": 0, "modified": "..."},
    {"name": "file2.pdf", "type": "file", "size": 51200, "modified": "..."}
  ]
}
```

当请求 `/api/tree?path=/dir1` 时，找到对应的真实路径，调用现有的 `get_directory_structure()` 返回子目录内容。

**安全沙箱**：

新增 `validate_multi_share_path()` 函数（`security.py`），验证文件下载请求：

```python
def validate_multi_share_path(
    request_path: str,
    shared_paths: List[Tuple[str, str]]
) -> Tuple[bool, str]:
    """
    Validate a download request against the list of shared paths.

    The request_path follows the virtual filesystem convention:
      /files/<top_name>              → top-level file
      /files/<top_dir>/<sub_path>   → file inside a shared directory

    Returns (is_valid, real_abs_path)
    """
```

逻辑：
1. 提取 `request_path` 中的 `top_name`（虚拟文件系统第一级）
2. 在 `shared_paths` 中查找 `basename == top_name` 的项
3. 若为文件：确认请求路径仅为 `/files/<top_name>`，无子路径
4. 若为目录：构造 `real_path = shared_dir / sub_path`，调用 `os.path.realpath()` 并检查是否在 `shared_dir` 内（防止路径遍历）
5. 验证最终文件存在且可读

---

### 2.4 数据层 (`src/directory_handler.py`)

#### 新增函数：`get_multi_share_root_structure()`

```python
def get_multi_share_root_structure(
    paths: List[Tuple[str, str]]
) -> Dict:
    """
    Get the virtual root structure for multi-path sharing.

    Args:
        paths: List of (abs_path, path_type) tuples

    Returns:
        Dictionary compatible with get_directory_structure() format
    """
    items = []
    for abs_path, path_type in paths:
        try:
            stat = os.stat(abs_path)
            items.append({
                'name': os.path.basename(abs_path),
                'type': 'directory' if path_type == 'directory' else 'file',
                'size': stat.st_size if path_type == 'file' else 0,
                'modified': datetime.fromtimestamp(stat.st_mtime).isoformat()
            })
        except OSError:
            continue
    # Sort: directories first, then alphabetically
    items.sort(key=lambda x: (x['type'] != 'directory', x['name'].lower()))
    return {'path': '/', 'items': items}
```

#### 新增函数：`stream_multi_paths_as_zip()`

```python
def stream_multi_paths_as_zip(
    output_stream,
    paths: List[Tuple[str, str]],
    progress_callback: bool = False
) -> None:
    """
    Stream multiple files/directories as a single ZIP.

    ZIP 内部结构：
      file1.txt          → 顶层文件，直接在根
      dir1/              → 目录，保留内部结构
      dir1/subfile.txt
      file2.pdf

    Args:
        output_stream: HTTP response wfile
        paths: List of (abs_path, path_type)
        progress_callback: If True, print progress logs
    """
```

内部逻辑：
- 对每个 `(abs_path, path_type)`：
  - `file`：`zipf.write(abs_path, arcname=basename)`
  - `directory`：`os.walk(abs_path)` 递归，`arcname = basename + '/' + relpath`

---

### 2.5 模板层 (`src/templates.py`)

#### 新增函数：`generate_multi_share_spa_html()`

多路径 SPA 相比目录 SPA 的主要差异：

1. **标题**：显示 "Quick Share - N items" 而非目录名
2. **ZIP 下载链接**：`/download/all.zip`（而非 `/?download=zip`）
3. **文件下载链接**：`/files/<name>`（虚拟路径）
4. **API 端点**：沿用 `/api/tree` 和 `/api/content`，但根目录语义不同
5. **无 Legacy View 跳转**（或跳转到 `/?legacy=1`，显示多路径 legacy 列表）

模板复用现有 SPA 的 CSS 变量和 TreeItem 组件结构，仅修改：
- 初始 `loadRoot()` 调用路径（同 `/api/tree?path=/`）
- 下载链接构造逻辑（`/files/` 前缀替代直接路径）
- 标题文案

#### 新增函数：`generate_multi_share_legacy_html()`

用于 `--legacy` 模式，以表格形式展示多路径列表，含全部打包 ZIP 按钮。

---

### 2.6 安全层 (`src/security.py`)

#### 新增函数：`validate_multi_share_path()`

详见 2.3 节描述。核心安全保证：

1. **路径遍历防护**: 提取 top_name 后，对子路径使用 `os.path.realpath()` + `os.path.commonpath()` 检查
2. **沙箱绑定**: 每个请求必须映射到某个 `shared_paths` 中的条目内部
3. **URL 解码**: 使用 `urllib.parse.unquote()` 处理编码路径
4. **双层 `..` 检测**: 与现有 `is_path_traversal_attack()` 一致

---

## 3. 数据流

### 3.1 正常下载流（文件）

```
浏览器 GET /files/file1.txt
  → MultiShareHandler.do_GET()
  → validate_multi_share_path('/files/file1.txt', server.paths)
    → top_name = 'file1.txt'
    → found: ('/abs/path/to/file1.txt', 'file')
    → return (True, '/abs/path/to/file1.txt')
  → _serve_file('/abs/path/to/file1.txt')
  → _stream_file_with_headers(...)
```

### 3.2 正常下载流（目录内文件）

```
浏览器 GET /files/dir1/subdir/readme.md
  → MultiShareHandler.do_GET()
  → validate_multi_share_path('/files/dir1/subdir/readme.md', server.paths)
    → top_name = 'dir1', sub_path = 'subdir/readme.md'
    → found dir: ('/abs/path/to/dir1', 'directory')
    → real_path = realpath('/abs/path/to/dir1/subdir/readme.md')
    → commonpath check: real_path starts with '/abs/path/to/dir1' ✓
    → return (True, '/abs/path/to/dir1/subdir/readme.md')
  → _serve_file('/abs/path/to/dir1/subdir/readme.md')
```

### 3.3 ZIP 全量下载流

```
浏览器 GET /download/all.zip
  → MultiShareHandler.do_GET()
  → _serve_all_as_zip()
  → stream_multi_paths_as_zip(wfile, server.paths, progress_callback=True)
    → zipf.write(file1.txt, 'file1.txt')
    → os.walk(dir1/) → zipf.write(each_file, 'dir1/relative_path')
    → zipf.write(file2.pdf, 'file2.pdf')
  → 流式传输完成
```

### 3.4 会话计数流（R-008）

复用 `DirectoryShareServer.track_session()` 的完整逻辑：
- 第一次访问 `/` 或任意 API/下载端点时分配 session cookie
- 同一 session 内的所有请求不消耗额外次数
- 超出 `max_sessions` 限制后返回 403

---

## 4. 接口设计

### 4.1 HTTP API

| 端点 | 方法 | 参数 | 响应 | 描述 |
|------|------|------|------|------|
| `/` | GET | - | HTML | 多路径 SPA 页面 |
| `/?legacy=1` | GET | - | HTML | Legacy 列表页面 |
| `/api/tree` | GET | `path=/` | JSON | 虚拟根目录列表 |
| `/api/tree` | GET | `path=/dirname` | JSON | 子目录内容 |
| `/api/content` | GET | `path=/file` | JSON | 文件预览内容 |
| `/files/<name>` | GET | - | Binary | 顶层文件下载 |
| `/files/<dir>/<path>` | GET | - | Binary | 目录内文件下载 |
| `/download/all.zip` | GET | - | ZIP | 全量打包下载 |

### 4.2 JSON 结构

**`/api/tree` 响应**（复用现有格式）：

```json
{
  "path": "/",
  "items": [
    {
      "name": "file1.txt",
      "type": "file",
      "size": 1024,
      "modified": "2026-03-30T12:00:00"
    },
    {
      "name": "dir1",
      "type": "directory",
      "size": 0,
      "modified": "2026-03-30T11:00:00"
    }
  ]
}
```

---

## 5. 测试设计

### 5.1 单元测试

| 模块 | 测试函数 | 场景 |
|------|---------|------|
| `test_cli.py` | `test_parse_multi_paths` | nargs='+' 接受多个路径 |
| `test_cli.py` | `test_parse_single_path` | 向后兼容单路径 |
| `test_main.py` | `test_validate_multi_paths_all_valid` | 全部有效路径 |
| `test_main.py` | `test_validate_multi_paths_invalid_path` | 无效路径报错 |
| `test_main.py` | `test_validate_multi_paths_name_conflict` | 同名冲突检测 |
| `test_security.py` | `test_validate_multi_share_path_file` | 顶层文件安全验证 |
| `test_security.py` | `test_validate_multi_share_path_dir_file` | 目录内文件安全验证 |
| `test_security.py` | `test_validate_multi_share_path_traversal` | 路径遍历攻击防御 |
| `test_directory_handler.py` | `test_get_multi_share_root_structure` | 虚拟根目录结构 |
| `test_directory_handler.py` | `test_stream_multi_paths_as_zip` | ZIP 流式输出 |
| `test_server.py` | `test_multi_share_handler_root` | GET / 返回 HTML |
| `test_server.py` | `test_multi_share_handler_api_tree` | GET /api/tree |
| `test_server.py` | `test_multi_share_handler_file_download` | GET /files/name |
| `test_server.py` | `test_multi_share_handler_zip_download` | GET /download/all.zip |

### 5.2 集成测试

| 测试文件 | 场景 |
|---------|------|
| `tests/integration/test_multi_share_integration.py` | 端到端：多文件分享、下载、ZIP |

---

## 6. 风险与缓解措施

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|---------|
| 单文件体验变更（R-007）破坏现有用户期望 | 中 | 中 | README 更新说明行为变更；保留 `--legacy` 参数 |
| ZIP 内存溢出（大文件） | 低 | 高 | 复用现有流式 ZIP，不在内存中累积 |
| 路径遍历攻击 | 低 | 高 | `validate_multi_share_path()` + `commonpath` 双重防护 |
| 大量文件导致 `/api/tree` 响应超时 | 低 | 中 | 只扫描顶层（非递归），子目录按需加载 |
| SPA 模板复杂度增加 | 中 | 低 | 复用现有 TreeItem 组件，仅修改下载链接构造逻辑 |

---

## 7. 不变更项

- `FileShareServer` 类：不变更，不再被 `main.py` 调用（改由 `MultiShareServer` 统一处理）
- `DirectoryShareServer` 类：不变更，保留供测试和直接使用
- `validate_request_path()`：不变更
- `validate_directory_path()`：不变更
- `stream_directory_as_zip()`：不变更，被新的 `stream_multi_paths_as_zip()` 内部复用
- `generate_spa_html()`：不变更（目录模式继续使用）
