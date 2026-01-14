# 技术设计文档: Directory Sharing

> **状态**: Planning Phase - Stage 2
> **生成时间**: 2026-01-14
> **复杂度**: standard
> **关联需求**: docs/dev/directory-sharing/directory-sharing-requirements.md

## 1. 架构概览

### 1.1 设计原则

1. **最小侵入性**: 在现有架构基础上扩展，避免破坏单文件共享功能
2. **安全优先**: 强化路径验证，防止目录遍历和符号链接逃逸
3. **性能优化**: 使用流式传输处理大文件和目录zip
4. **代码复用**: 复用现有的server、security、cli等模块

### 1.2 架构分层

```
┌─────────────────────────────────────────────────────────┐
│                     CLI Layer (cli.py)                  │
│  - 参数解析（支持文件和目录）                              │
│  - 帮助文本更新                                          │
└─────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────┐
│                  Main Layer (main.py)                   │
│  - 路径类型检测（文件 vs 目录）                           │
│  - 验证逻辑（validate_file + validate_directory）        │
│  - Server初始化和启动                                    │
└─────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────┐
│               Security Layer (security.py)              │
│  - 路径遍历检测（扩展以支持目录沙箱）                      │
│  - 符号链接验证                                          │
│  - URL解码和规范化                                       │
└─────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────┐
│             Server Layer (server.py)                    │
│  - FileShareServer: 单文件共享（现有）                    │
│  - DirectoryShareServer: 目录共享（新增）                 │
│  - FileShareHandler: 文件处理（现有）                     │
│  - DirectoryShareHandler: 目录处理（新增）                │
└─────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────┐
│          Directory Handler (directory_handler.py)       │
│  - list_directory(): 生成HTML文件列表                     │
│  - stream_zip(): 流式生成目录zip                          │
│  - get_directory_info(): 获取目录统计信息                 │
└─────────────────────────────────────────────────────────┘
```

## 2. 模块设计

### 2.1 main.py 扩展

#### 新增函数

```python
def validate_path(path: str) -> Tuple[Path, str, int]:
    """
    验证路径并返回类型和大小信息。

    Args:
        path: 文件或目录路径

    Returns:
        (Path对象, 类型('file'|'directory'), 大小字节数)

    Raises:
        FileNotFoundError: 路径不存在
    """
    path_obj = Path(path).resolve()

    if not path_obj.exists():
        raise FileNotFoundError(f"Path not found: {path}")

    if path_obj.is_file():
        return path_obj, 'file', path_obj.stat().st_size
    elif path_obj.is_dir():
        # 计算目录总大小
        total_size = sum(f.stat().st_size for f in path_obj.rglob('*') if f.is_file())
        return path_obj, 'directory', total_size
    else:
        raise ValueError(f"Unsupported path type: {path}")
```

#### 修改 main() 函数

```python
def main() -> None:
    # ... 参数解析 ...

    # 验证路径（替换原来的validate_file）
    path_obj, path_type, size_bytes = validate_path(args.file_path)

    # 根据类型初始化不同的Server
    if path_type == 'file':
        server = FileShareServer(
            file_path=str(path_obj),
            port=port,
            timeout_minutes=server_timeout_minutes,
            max_downloads=args.max_downloads
        )
        # 现有的单文件启动逻辑
    else:  # directory
        server = DirectoryShareServer(
            directory_path=str(path_obj),
            port=port,
            timeout_minutes=server_timeout_minutes,
            max_sessions=args.max_downloads  # 对目录，max_downloads表示会话数
        )
        # 目录共享启动逻辑
```

### 2.2 security.py 扩展

#### 新增函数

```python
def validate_directory_path(
    request_path: str,
    shared_directory: str
) -> Tuple[bool, str]:
    """
    验证目录访问请求路径。

    安全检查：
    1. URL解码
    2. 路径遍历检测（..）
    3. 路径规范化
    4. 验证最终路径在shared_directory内
    5. 符号链接真实路径检测

    Args:
        request_path: HTTP请求路径（如 /subdir/file.txt）
        shared_directory: 共享目录的绝对路径

    Returns:
        (是否有效, 规范化后的路径)
    """
    # 1. 清理路径（去除查询字符串）
    clean_path = request_path.split('?')[0].split('#')[0]

    # 2. URL解码（防止编码绕过）
    decoded_path = urllib.parse.unquote(clean_path)

    # 3. 检测路径遍历
    if is_path_traversal_attack(decoded_path):
        return False, ""

    # 4. 构建完整路径
    # 将请求路径转换为相对路径（去掉前导/）
    relative_path = decoded_path.lstrip('/')
    full_path = os.path.join(shared_directory, relative_path)

    # 5. 解析真实路径（处理符号链接）
    try:
        real_path = os.path.realpath(full_path)
        real_shared = os.path.realpath(shared_directory)
    except Exception:
        return False, ""

    # 6. 验证在沙箱内（使用commonpath更安全）
    try:
        common = os.path.commonpath([real_path, real_shared])
        if common != real_shared:
            return False, ""
    except ValueError:
        # 不同驱动器（Windows）或无共同路径
        return False, ""

    # 7. 验证路径存在
    if not os.path.exists(real_path):
        return False, ""

    return True, real_path
```

### 2.3 server.py 扩展

#### 新增 DirectoryShareHandler

```python
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

        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(html)))
        self.end_headers()
        self.wfile.write(html.encode('utf-8'))

    def _serve_file(self, file_path: str):
        """流式传输单个文件。"""
        filename = os.path.basename(file_path)
        file_size = os.path.getsize(file_path)

        self.send_response(200)
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Disposition', f'attachment; filename="{filename}"')
        self.send_header('Content-Length', str(file_size))
        self.end_headers()

        # 流式传输
        with open(file_path, 'rb') as f:
            while chunk := f.read(8192):
                self.wfile.write(chunk)

    def _serve_directory_zip(self, base_dir: str, target_dir: str):
        """流式生成并传输目录zip。"""
        zip_filename = os.path.basename(base_dir) + '.zip'

        self.send_response(200)
        self.send_header('Content-Type', 'application/zip')
        self.send_header('Content-Disposition', f'attachment; filename="{zip_filename}"')
        self.send_header('Transfer-Encoding', 'chunked')
        self.end_headers()

        # 流式生成zip（通过directory_handler）
        stream_directory_as_zip(self.wfile, base_dir, target_dir)
```

#### 新增 DirectoryShareServer

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

        # 会话管理
        self.sessions = {}  # {session_id: {ip, first_access_time}}
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

    def track_session(self, request_handler) -> bool:
        """
        跟踪会话，返回是否允许访问。

        使用cookie识别会话，如果会话数达到上限则拒绝。
        """
        with self.session_lock:
            # 从cookie获取session_id
            session_id = self._get_or_create_session(request_handler)

            # 检查会话数量
            if len(self.sessions) > self.max_sessions:
                if session_id not in self.sessions:
                    return False  # 新会话，拒绝

            # 记录会话
            if session_id not in self.sessions:
                self.sessions[session_id] = {
                    'ip': request_handler.client_address[0],
                    'start_time': time.time()
                }

            return True

    def _get_or_create_session(self, request_handler) -> str:
        """从cookie获取或创建新的session_id。"""
        # 解析Cookie头
        cookie_header = request_handler.headers.get('Cookie', '')
        # 简单的session_id提取（或生成UUID）
        # 实现细节见directory_handler.py
        pass
```

### 2.4 directory_handler.py (新增模块)

```python
"""目录共享的核心处理逻辑。"""

import os
import zipfile
import html
from pathlib import Path
from typing import List, Dict
from datetime import datetime

def get_directory_info(directory_path: str) -> Dict:
    """
    获取目录统计信息。

    Returns:
        {
            'total_files': int,
            'total_dirs': int,
            'total_size': int
        }
    """
    path = Path(directory_path)
    files = list(path.rglob('*'))

    total_files = sum(1 for f in files if f.is_file())
    total_dirs = sum(1 for f in files if f.is_dir())
    total_size = sum(f.stat().st_size for f in files if f.is_file())

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

def generate_directory_listing_html(
    base_dir: str,
    current_dir: str
) -> str:
    """
    生成目录列表HTML页面。

    Args:
        base_dir: 共享的根目录
        current_dir: 当前正在列出的目录

    Returns:
        HTML字符串
    """
    # 计算相对路径（用于显示）
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
                # 跳过无法访问的文件
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
        '        .container { max-width: 1200px; margin: 0 auto; background: white; padding: 20px; border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }',
        '        h1 { color: #333; border-bottom: 2px solid #007bff; padding-bottom: 10px; }',
        '        .info { background: #e9ecef; padding: 10px; border-radius: 4px; margin: 10px 0; }',
        '        .actions { margin: 15px 0; }',
        '        .btn { display: inline-block; padding: 10px 20px; background: #007bff; color: white; text-decoration: none; border-radius: 4px; margin-right: 10px; }',
        '        .btn:hover { background: #0056b3; }',
        '        table { width: 100%; border-collapse: collapse; margin-top: 20px; }',
        '        th, td { text-align: left; padding: 12px; border-bottom: 1px solid #ddd; }',
        '        th { background: #f8f9fa; font-weight: bold; }',
        '        tr:hover { background: #f8f9fa; }',
        '        .dir { color: #007bff; font-weight: bold; }',
        '        .file { color: #333; }',
        '        a { text-decoration: none; color: inherit; }',
        '        a:hover { text-decoration: underline; }',
        '    </style>',
        '</head>',
        '<body>',
        '    <div class="container">',
        f'        <h1>Quick Share - {html.escape(os.path.basename(base_dir))}</h1>',
        f'        <div class="info">Current Path: {html.escape(relative_path)}</div>',
        '        <div class="actions">',
        f'            <a href="/?download=zip" class="btn">Download All as Zip</a>',
    ]

    # 添加"返回上级"按钮（如果不在根目录）
    if current_dir != base_dir:
        parent_relative = os.path.dirname(relative_path)
        html_parts.append(f'            <a href="{parent_relative}" class="btn">Go Up</a>')

    html_parts.extend([
        '        </div>',
        '        <table>',
        '            <thead>',
        '                <tr>',
        '                    <th>Name</th>',
        '                    <th>Size</th>',
        '                    <th>Modified</th>',
        '                </tr>',
        '            </thead>',
        '            <tbody>',
    ])

    # 添加文件/目录列表
    if not items:
        html_parts.append('                <tr><td colspan="3">No files or directories</td></tr>')
    else:
        for item in items:
            icon = '📁' if item['is_dir'] else '📄'
            css_class = 'dir' if item['is_dir'] else 'file'
            size_str = '-' if item['is_dir'] else format_file_size(item['size'])
            modified_str = item['modified'].strftime('%Y-%m-%d %H:%M')

            # 构建链接路径
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

def stream_directory_as_zip(
    output_stream,
    base_dir: str,
    target_dir: str
) -> None:
    """
    流式生成目录的zip文件。

    Args:
        output_stream: 输出流（HTTP响应的wfile）
        base_dir: 共享的根目录
        target_dir: 要打包的目标目录（可能是子目录）
    """
    # 使用ZipFile的流式写入
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

def generate_error_html(error_message: str) -> str:
    """生成错误页面HTML。"""
    return f"""
    <!DOCTYPE html>
    <html>
    <head>
        <title>Error - Quick Share</title>
        <style>
            body {{ font-family: Arial, sans-serif; margin: 50px; }}
            .error {{ color: red; }}
        </style>
    </head>
    <body>
        <h1>Error</h1>
        <p class="error">{html.escape(error_message)}</p>
    </body>
    </html>
    """
```

### 2.5 cli.py 扩展

```python
def parse_arguments(args=None):
    parser = argparse.ArgumentParser(
        description="Quick Share - Share files or directories via HTTP."
    )

    parser.add_argument(
        "file_path",
        help="Path to the file or directory to share"  # 更新帮助文本
    )

    parser.add_argument(
        "-n", "--max-downloads",
        type=int,
        default=10,
        help="Maximum downloads (files) or sessions (directories) allowed (default: 10)"  # 更新帮助文本
    )

    # ... 其他参数保持不变 ...
```

## 3. 数据流设计

### 3.1 目录列表请求流程

```
User访问 http://IP:PORT/
    ↓
DirectoryShareHandler.do_GET()
    ↓
validate_directory_path('/', shared_dir) → (True, real_path)
    ↓
os.path.isdir(real_path) → True
    ↓
_serve_directory_listing(base_dir, real_path)
    ↓
generate_directory_listing_html() → HTML字符串
    ↓
返回200响应 + HTML内容
```

### 3.2 单文件下载流程

```
User点击文件链接 /subdir/file.txt
    ↓
DirectoryShareHandler.do_GET()
    ↓
validate_directory_path('/subdir/file.txt', shared_dir) → (True, real_path)
    ↓
os.path.isfile(real_path) → True
    ↓
_serve_file(real_path)
    ↓
流式传输文件（8KB chunks）
    ↓
返回200响应 + 文件内容
```

### 3.3 目录Zip下载流程

```
User点击 "Download All as Zip"
    ↓
请求 /?download=zip
    ↓
DirectoryShareHandler.do_GET()
    ↓
检测到 ?download=zip
    ↓
_serve_directory_zip(base_dir, target_dir)
    ↓
设置响应头：Transfer-Encoding: chunked
    ↓
stream_directory_as_zip(wfile, base_dir, target_dir)
    ↓
使用zipfile.ZipFile流式写入
    ↓
逐个文件添加到zip（不加载全部到内存）
    ↓
返回200响应 + zip流
```

### 3.4 会话管理流程

```
User首次访问
    ↓
DirectoryShareHandler.do_GET()
    ↓
server.session_manager.track_session(handler)
    ↓
检查Cookie中的session_id
    ↓
未找到 → 生成新UUID → 设置Set-Cookie响应头
    ↓
检查当前会话数 < max_sessions?
    ↓
是 → 记录新会话 → 允许访问
    ↓
否 → 返回403 "Session limit reached"

User后续访问（同一浏览器）
    ↓
携带Cookie中的session_id
    ↓
识别为已有会话 → 允许访问
```

## 4. 安全机制

### 4.1 路径遍历防护

**威胁场景**:
- `GET /../../../etc/passwd`
- `GET /subdir/../../../secret.txt`
- `GET /%2e%2e/parent/file.txt` (URL编码)
- `GET /./subdir/../../file.txt` (多重遍历)

**防护措施**:
1. URL解码后检测 `..`
2. 使用 `os.path.realpath()` 解析符号链接和相对路径
3. 使用 `os.path.commonpath()` 验证最终路径在共享目录内
4. 拒绝任何不在沙箱内的路径

### 4.2 符号链接处理

**威胁场景**:
- 共享目录内有符号链接指向 `/etc/passwd`
- 符号链接指向共享目录的父目录

**防护措施**:
1. `os.path.realpath()` 会自动解析符号链接的真实路径
2. 验证真实路径必须在共享目录内
3. 如果符号链接指向外部，`validate_directory_path` 返回 False

### 4.3 输入验证

**验证项**:
- 路径必须存在
- 路径必须是文件或目录（不支持其他类型）
- URL解码后再验证（防止编码绕过）
- 拒绝包含特殊字符的路径（如空字节）

### 4.4 日志记录

```python
def log_security_event(event_type: str, details: str):
    """记录安全事件到日志。"""
    timestamp = datetime.now().isoformat()
    print(f"[SECURITY] {timestamp} - {event_type}: {details}", file=sys.stderr)
```

在以下场景记录安全事件：
- 路径遍历尝试
- 符号链接逃逸尝试
- 会话限制达到
- 权限被拒绝

## 5. 性能优化

### 5.1 流式传输

**文件下载**:
- 使用 8KB chunks 流式传输
- 避免一次性加载整个文件到内存

**Zip生成**:
- 使用 `zipfile.ZipFile` 的流式写入模式
- 逐个文件添加到zip，边压缩边传输
- 使用 `Transfer-Encoding: chunked` 避免预先计算总大小

### 5.2 目录列表分页（可选，P2优先级）

如果目录文件数 > 500:
- 实现分页机制（每页100个文件）
- 使用查询参数 `?page=2`
- 在HTML底部添加分页导航

### 5.3 缓存（暂不实现）

- 目录列表不缓存（共享时的快照即可）
- 文件内容不缓存（使用HTTP标准缓存头）

## 6. 测试策略

### 6.1 单元测试

**test_security.py**:
- `test_should_reject_parent_traversal`: 测试 `../` 检测
- `test_should_reject_encoded_traversal`: 测试 URL编码绕过
- `test_should_allow_subdirectory_access`: 测试合法子目录访问
- `test_symlink_escape_detection`: 测试符号链接逃逸检测

**test_server.py**:
- `test_zip_preserves_structure`: 测试zip保留目录结构
- `test_session_based_download_counting`: 测试会话计数
- `test_directory_handler_file_response`: 测试单文件响应
- `test_directory_handler_listing_response`: 测试目录列表响应

**test_main.py**:
- `test_should_detect_directory_path`: 测试目录检测
- `test_should_detect_file_path`: 测试文件检测
- `test_should_error_on_invalid_path`: 测试无效路径

### 6.2 集成测试

**test_integration.py**:
- `test_directory_listing_display`: 创建临时目录，启动服务器，验证HTML
- `test_download_single_file_from_directory`: 测试从目录下载单个文件
- `test_navigate_subdirectories`: 测试子目录导航
- `test_download_directory_as_zip`: 测试zip下载并验证内容
- `test_server_stops_after_session_limit`: 测试会话限制
- `test_single_file_sharing_unchanged`: 回归测试单文件共享

### 6.3 边缘场景测试

- 空目录
- 仅包含子目录的目录
- 特殊字符文件名（Unicode、空格）
- 符号链接（指向内部和外部）
- 权限被拒的文件
- 超大文件（> 1GB）
- 超多文件（> 1000个）

## 7. 向后兼容性

### 7.1 不破坏现有功能

**保证**:
- `validate_file()` 函数保留（或重命名为内部函数）
- `FileShareServer` 和 `FileShareHandler` 保持不变
- 所有现有测试必须通过
- CLI参数含义不变（`-n` 对文件是下载次数，对目录是会话数）

### 7.2 共享代码逻辑

**复用模块**:
- `security.py`: 扩展但不修改现有 `validate_request_path`
- `server.py`: 添加新类但不修改现有类
- `cli.py`: 扩展帮助文本但不改变参数结构

## 8. 依赖和技术栈

### 8.1 标准库依赖

- `http.server`: HTTP服务器
- `zipfile`: Zip文件生成
- `os`, `os.path`: 文件系统操作
- `pathlib`: 现代路径操作
- `urllib.parse`: URL解码
- `html`: HTML转义
- `uuid`: 会话ID生成
- `threading`: 线程管理

### 8.2 无需外部依赖

设计保持轻量级，不引入 jinja2 等外部库：
- HTML模板使用字符串拼接生成
- 会话管理使用内存字典

## 9. 部署和配置

### 9.1 无额外配置

保持现有的零配置设计：
- 自动检测路径类型
- 自动分配端口（或使用 `-p` 指定）
- 自动设置超时和会话限制

### 9.2 命令示例

```bash
# 共享目录（自动检测）
quick-share ./my-project

# 共享目录，指定端口和会话限制
quick-share ./my-project -p 9090 -n 5 -t 10m

# 共享单文件（现有功能不变）
quick-share ./document.pdf
```

## 10. 风险和缓解措施

| 风险 | 影响 | 缓解措施 | 状态 |
|------|------|----------|------|
| 路径遍历漏洞 | 高 | 多层验证 + 单元测试 | 已设计 |
| 符号链接逃逸 | 高 | realpath + commonpath验证 | 已设计 |
| 大目录性能 | 中 | 流式传输 + 分页（可选） | 已设计 |
| 会话管理复杂性 | 低 | 简单cookie机制 | 已设计 |
| 向后兼容破坏 | 中 | 回归测试 + 保留现有类 | 已设计 |

## 11. 实施顺序建议

1. **Phase 1: 基础架构**
   - 扩展 `security.py` (路径验证)
   - 创建 `directory_handler.py` (核心逻辑)

2. **Phase 2: Server层**
   - 添加 `DirectoryShareHandler`
   - 添加 `DirectoryShareServer`

3. **Phase 3: Main层集成**
   - 修改 `main.py` (路径检测和分发)
   - 更新 `cli.py` (帮助文本)

4. **Phase 4: 测试和验证**
   - 单元测试
   - 集成测试
   - 安全测试
   - 回归测试

## 12. 后续扩展点

### 12.1 可选功能（不在MVP范围内）

- 文件搜索
- 排序选项（名称、大小、时间）
- 自定义HTML模板
- 认证和密码保护
- HTTPS支持
- 实时更新（WebSocket）

### 12.2 性能增强

- 目录列表缓存
- Zip预生成（小目录）
- 并发连接限制

---

**设计完成日期**: 2026-01-14
**审核状态**: 待审核
**下一步**: 创建任务拆分文档 (directory-sharing-tasks.md)
