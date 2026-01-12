# 技术设计文档: Quick Share

> **生成方式**: spec-workflow-executor - Stage 2
> **生成时间**: 2026-01-12
> **基于需求**: quick-share-requirements.md

## 1. 技术栈选择

### 1.1 编程语言决策

**选择: Python 3.8+**

**理由**:
- **快速开发**: Python 标准库已包含 HTTP 服务器，无需额外依赖
- **跨平台**: 天然支持 Windows/Linux/macOS
- **简洁性**: 命令行工具开发效率高
- **依赖管理**: 可使用 PyInstaller 打包成单一可执行文件
- **网络库成熟**: `socket`、`http.server` 标准库完善

**对比 Go**:
| 特性 | Python | Go | 决策 |
|------|--------|-----|------|
| 开发速度 | 快 | 中 | ✅ Python |
| 运行性能 | 中 | 快 | 对于文件分享场景性能足够 |
| 打包大小 | 较大(~10MB) | 小(~5MB) | 可接受 |
| 标准库HTTP | ✅ | ✅ | 平手 |
| 网络检测 | 需第三方库 | 标准库 | Go略优但差距小 |

### 1.2 核心依赖

**最小化依赖原则** - 优先使用标准库:

```yaml
runtime:
  - Python: 3.8+

standard_library:
  - http.server: HTTP 服务器
  - socketserver: TCP 服务器基础
  - socket: 网络接口检测
  - argparse: 命令行参数解析
  - pathlib: 路径操作
  - logging: 日志输出
  - threading: 超时控制
  - time: 时间处理
  - os: 系统操作
  - sys: 系统参数

optional_dependencies:
  - netifaces: (备选) 网络接口检测增强
  - psutil: (备选) 更准确的网卡优先级

packaging:
  - PyInstaller: 打包成可执行文件
```

**依赖决策**:
- 初期仅使用标准库实现
- 如网络检测逻辑复杂，可按需添加 `netifaces`
- 避免重量级框架 (Flask/FastAPI)

## 2. 模块架构

### 2.1 整体结构

```
quick-share/
├── src/
│   ├── __init__.py
│   ├── main.py                 # 入口点
│   ├── cli.py                  # 命令行参数解析
│   ├── network.py              # 网络接口检测
│   ├── server.py               # HTTP 服务器
│   ├── security.py             # 路径安全验证
│   ├── logger.py               # 日志格式化
│   └── utils.py                # 工具函数（文件大小格式化等）
├── tests/
│   ├── __init__.py
│   ├── test_cli.py
│   ├── test_network.py
│   ├── test_server.py
│   ├── test_security.py
│   ├── test_logger.py
│   ├── test_utils.py
│   └── integration/
│       ├── __init__.py
│       ├── test_download.py
│       └── test_limits.py
├── setup.py                    # 安装配置
├── requirements.txt            # 依赖列表
├── requirements-dev.txt        # 开发依赖（pytest等）
├── README.md
└── .gitignore
```

### 2.2 模块职责

#### 2.2.1 main.py - 应用入口

```python
"""
主流程编排:
1. 解析命令行参数
2. 验证文件路径
3. 检测局域网IP
4. 查找可用端口
5. 启动HTTP服务
6. 输出下载链接
7. 监听下载事件和超时
8. 优雅退出
"""
```

**关键函数**:
- `main()`: 主入口
- `validate_file(path)`: 验证文件是否存在且为文件
- `format_output(ip, port, filename, file_size)`: 格式化输出信息

#### 2.2.2 cli.py - 命令行接口

```python
"""
命令行参数定义:
- positional: file_path (文件路径)
- -p, --port: 指定端口 (可选)
- -n, --max-downloads: 最大下载次数 (默认10)
- -t, --timeout: 运行时长 (默认5m, 支持 5m/1h/0)
- --help: 帮助信息
"""
```

**关键函数**:
- `parse_arguments()`: 解析命令行参数
- `parse_duration(duration_str)`: 解析时长字符串 (5m -> 300秒)

#### 2.2.3 network.py - 网络检测

```python
"""
职责:
1. 检测本机所有网络接口
2. 过滤无效IP (127.0.0.1, 169.254.x.x, 虚拟网卡)
3. 按优先级排序 (以太网 > WiFi > 其他)
4. 返回最佳局域网IP
"""
```

**关键函数**:
- `get_local_ip()`: 获取局域网IP (返回 str 或抛出异常)
- `get_all_interfaces()`: 获取所有网络接口信息
- `is_valid_lan_ip(ip)`: 判断IP是否为有效局域网地址
- `get_interface_priority(interface_name)`: 获取网卡优先级

**实现策略**:
```python
# 策略1: 使用 socket 连接外部地址获取本地IP
def get_local_ip_via_socket():
    """通过创建UDP socket连接到外部IP (不实际发送数据) 获取本地IP"""
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.connect(("8.8.8.8", 80))
        return s.getsockname()[0]

# 策略2: 遍历所有网络接口 (备选方案)
def get_local_ip_via_interfaces():
    """遍历所有接口，过滤并排序"""
    # 使用 netifaces 或解析 ip addr / ifconfig
```

#### 2.2.4 server.py - HTTP 服务器

```python
"""
职责:
1. 查找可用端口 (8000-8099)
2. 启动 HTTP 服务器
3. 处理 HTTP 请求
4. 下载计数和限制
5. 超时自动关闭
"""
```

**核心类设计**:

```python
class FileShareHandler(http.server.BaseHTTPRequestHandler):
    """
    自定义请求处理器
    - 处理 GET 请求
    - 路径安全验证
    - 文件流式传输
    - 日志输出
    """

    # 类变量 (共享状态)
    file_path = None          # 被分享的文件路径
    file_basename = None       # 文件basename
    download_count = 0         # 当前下载次数
    max_downloads = 10         # 最大下载次数
    server_instance = None     # 服务器实例 (用于关闭)

    def do_GET(self):
        """处理 GET 请求"""

    def log_message(self, format, *args):
        """自定义日志格式"""

class FileShareServer:
    """
    文件分享服务器包装类
    - 查找可用端口
    - 启动服务器
    - 超时控制
    - 优雅关闭
    """

    def __init__(self, file_path, port=None, max_downloads=10, timeout=300):
        pass

    def find_available_port(self, start_port=8000, end_port=8099):
        """查找可用端口"""

    def start(self):
        """启动服务器"""

    def shutdown(self):
        """优雅关闭"""
```

**关键函数**:
- `find_available_port(start, end)`: 查找可用端口
- `is_port_available(port)`: 检查端口是否可用
- `start_server(file_path, ip, port, max_downloads, timeout)`: 启动服务器

#### 2.2.5 security.py - 安全验证

```python
"""
职责:
1. 验证请求路径是否安全
2. 防止路径遍历攻击
3. 仅允许访问指定文件basename
"""
```

**关键函数**:
- `validate_request_path(request_path, allowed_basename)`: 验证请求路径
- `is_path_traversal_attack(path)`: 检测路径遍历攻击
- `normalize_path(path)`: 路径标准化

**安全策略**:
```python
def validate_request_path(request_path, allowed_basename):
    """
    规则:
    1. 移除 query string (? 后的部分)
    2. URL 解码
    3. 移除前导 /
    4. 检查是否包含 .. 或 绝对路径标志
    5. 检查是否等于 allowed_basename

    返回: (is_valid: bool, normalized_path: str)
    """
```

#### 2.2.6 logger.py - 日志格式化

```python
"""
职责:
1. 格式化下载日志
2. 格式化服务器输出
3. 统一日志样式
"""
```

**日志格式**:
```
启动信息:
════════════════════════════════════════
 Quick Share - File Ready
════════════════════════════════════════
File: test.txt
Size: 1.2 MB
URL:  http://192.168.1.100:8000/test.txt

Download Commands:
  curl http://192.168.1.100:8000/test.txt -O
  wget http://192.168.1.100:8000/test.txt

Server running. Press Ctrl+C to stop.
Max downloads: 10 | Timeout: 5m
════════════════════════════════════════

下载日志:
[2026-01-12 14:30:25] 192.168.1.101 - GET /test.txt - 200 OK - Download 1/10
[2026-01-12 14:30:28] Download completed

关闭信息:
Server stopped. Total downloads: 3/10
```

**关键函数**:
- `format_startup_message(ip, port, filename, file_size, max_downloads, timeout)`: 启动信息
- `format_download_log(timestamp, client_ip, method, path, status_code, current_count, max_count)`: 下载日志

#### 2.2.7 utils.py - 工具函数

```python
"""
职责:
1. 文件大小格式化 (字节 -> 人类可读)
2. 时长解析 (5m -> 300秒)
3. 其他通用工具
"""
```

**关键函数**:
- `format_file_size(bytes)`: 格式化文件大小 (1234567 -> "1.2 MB")
- `parse_duration(duration_str)`: 解析时长字符串 ("5m" -> 300)

## 3. API/接口设计

### 3.1 HTTP API

```
GET /<filename>
  描述: 下载文件

  请求:
    GET /test.txt HTTP/1.1
    Host: 192.168.1.100:8000

  响应 (成功):
    HTTP/1.1 200 OK
    Content-Type: application/octet-stream
    Content-Disposition: attachment; filename="test.txt"
    Content-Length: 1234567

    [文件内容]

  响应 (路径错误):
    HTTP/1.1 404 Not Found

  响应 (路径遍历):
    HTTP/1.1 403 Forbidden
```

### 3.2 模块接口

#### network.py
```python
def get_local_ip() -> str:
    """
    获取局域网IP地址

    Returns:
        str: IPv4 地址 (如 "192.168.1.100")

    Raises:
        RuntimeError: 无法检测到有效的局域网IP
    """
```

#### server.py
```python
def find_available_port(start_port: int = 8000, end_port: int = 8099) -> int:
    """
    查找可用端口

    Args:
        start_port: 起始端口
        end_port: 结束端口

    Returns:
        int: 可用端口号

    Raises:
        RuntimeError: 所有端口都被占用
    """

def start_server(
    file_path: str,
    ip: str,
    port: int,
    max_downloads: int = 10,
    timeout: int = 300
) -> None:
    """
    启动文件分享服务器

    Args:
        file_path: 文件绝对路径
        ip: 监听IP地址
        port: 监听端口
        max_downloads: 最大下载次数
        timeout: 超时时间 (秒, 0表示无限制)
    """
```

#### security.py
```python
def validate_request_path(request_path: str, allowed_basename: str) -> tuple[bool, str]:
    """
    验证请求路径是否安全

    Args:
        request_path: HTTP 请求路径 (如 "/test.txt")
        allowed_basename: 允许访问的文件名 (如 "test.txt")

    Returns:
        (is_valid, normalized_path): 是否有效 和 标准化后的路径
    """
```

## 4. 数据流设计

### 4.1 主流程数据流

```
[用户执行命令]
    ↓
[cli.py: 解析参数]
    ↓ {file_path, port, max_downloads, timeout}
    ↓
[main.py: 验证文件]
    ↓ {file_path, file_size}
    ↓
[network.py: 检测IP]
    ↓ {local_ip}
    ↓
[server.py: 查找端口]
    ↓ {available_port}
    ↓
[logger.py: 输出启动信息]
    ↓
[server.py: 启动HTTP服务]
    ↓
[等待请求循环]
    ↓
[收到 HTTP GET 请求]
    ↓ {request_path, client_ip}
    ↓
[security.py: 验证路径]
    ↓ {is_valid}
    ↓
[如果有效: 流式传输文件]
[如果无效: 返回 403/404]
    ↓
[logger.py: 输出下载日志]
    ↓
[更新下载计数]
    ↓
[检查是否达到限制]
    ↓ {达到上限}
    ↓
[server.py: 优雅关闭]
    ↓
[程序退出]
```

### 4.2 超时控制流

```
[启动服务器]
    ↓
[启动超时定时器线程]
    ↓
[定时器: time.sleep(timeout)]
    ↓
[时间到达]
    ↓
[调用 server.shutdown()]
    ↓
[优雅退出]
```

### 4.3 Ctrl+C 信号处理

```
[用户按下 Ctrl+C]
    ↓
[捕获 KeyboardInterrupt]
    ↓
[调用 server.shutdown()]
    ↓
[输出关闭信息]
    ↓
[退出程序]
```

## 5. 错误处理策略

### 5.1 错误分类

| 错误类型 | 示例 | 处理方式 | 退出码 |
|---------|------|---------|--------|
| 参数错误 | 文件路径缺失 | 显示帮助信息 | 1 |
| 文件错误 | 文件不存在 | 显示错误并退出 | 1 |
| 网络错误 | 无法检测IP | 显示错误并退出 | 1 |
| 端口错误 | 所有端口被占用 | 显示错误并退出 | 1 |
| 运行时错误 | 文件被删除 | 记录日志，返回404 | 0 |
| 安全错误 | 路径遍历攻击 | 记录日志，返回403 | 0 |

### 5.2 错误消息设计

```python
错误消息原则:
1. 清晰描述问题
2. 提供解决建议
3. 不暴露系统路径
4. 用户友好的语言

示例:
❌ 错误: Error opening /home/user/secret/file.txt: FileNotFoundError
✅ 正确: Error: File 'file.txt' not found. Please check the file path.

❌ 错误: All ports in range 8000-8099 are occupied
✅ 正确: Error: Cannot find available port (8000-8099 all occupied).
       Try specifying a custom port: quick-share file.txt -p 9000
```

### 5.3 异常处理结构

```python
# main.py
def main():
    try:
        args = parse_arguments()
        validate_file(args.file_path)
        ip = get_local_ip()
        port = find_available_port(args.port)
        start_server(...)
    except FileNotFoundError as e:
        print(f"Error: File '{args.file_path}' not found.", file=sys.stderr)
        sys.exit(1)
    except RuntimeError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print("\nServer stopped by user.")
        sys.exit(0)
```

## 6. 跨平台兼容性设计

### 6.1 路径处理

```python
# 使用 pathlib.Path 而非字符串拼接
from pathlib import Path

file_path = Path(args.file_path).resolve()  # 自动处理 Windows/Unix 路径
filename = file_path.name                    # 跨平台的 basename
```

### 6.2 网络接口检测

```python
# 策略1: socket 连接 (跨平台)
def get_local_ip_via_socket():
    # 适用于 Windows/Linux/macOS

# 策略2: 平台特定命令 (备选)
def get_local_ip_via_command():
    if sys.platform == 'win32':
        # ipconfig
    else:
        # ip addr 或 ifconfig
```

### 6.3 信号处理

```python
# Windows 不支持 SIGALRM, 使用线程定时器
import threading

def timeout_handler():
    server.shutdown()

timer = threading.Timer(timeout, timeout_handler)
timer.daemon = True
timer.start()
```

## 7. 测试策略

### 7.1 单元测试覆盖

| 模块 | 测试文件 | 覆盖目标 |
|------|---------|---------|
| cli.py | test_cli.py | 参数解析、帮助信息、时长解析 |
| network.py | test_network.py | IP检测、接口过滤、优先级排序 |
| server.py | test_server.py | 端口查找、请求处理、计数逻辑 |
| security.py | test_security.py | 路径验证、攻击防御 |
| logger.py | test_logger.py | 日志格式化 |
| utils.py | test_utils.py | 文件大小格式化 |

### 7.2 集成测试场景

| 测试文件 | 场景 |
|---------|------|
| test_download.py | 完整下载流程、curl/wget 兼容性 |
| test_limits.py | 下载次数限制、超时自动关闭 |

### 7.3 Mock 策略

```python
# 测试网络模块时 Mock socket
from unittest.mock import patch, MagicMock

@patch('socket.socket')
def test_get_local_ip(mock_socket):
    mock_socket.return_value.getsockname.return_value = ('192.168.1.100', 12345)
    assert get_local_ip() == '192.168.1.100'

# 测试服务器时使用真实 HTTP 客户端
import requests
def test_successful_download():
    # 启动服务器在后台线程
    # 使用 requests.get() 下载
    # 验证文件内容
```

## 8. 性能优化设计

### 8.1 文件流式传输

```python
# 避免一次性读取大文件到内存
def do_GET(self):
    with open(self.file_path, 'rb') as f:
        self.send_response(200)
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Length', str(file_size))
        self.end_headers()

        # 分块传输 (8KB per chunk)
        while True:
            chunk = f.read(8192)
            if not chunk:
                break
            self.wfile.write(chunk)
```

### 8.2 并发连接支持

```python
# 使用 ThreadingHTTPServer 支持并发
from http.server import ThreadingHTTPServer

server = ThreadingHTTPServer((ip, port), FileShareHandler)
```

### 8.3 启动时间优化

```python
# 避免不必要的初始化
# 1. 延迟导入重量级库
# 2. 先输出信息，后启动监听
# 3. 并行检测IP和端口
```

## 9. 安全加固

### 9.1 路径安全

```python
# 防御措施:
1. 使用 Path.resolve() 规范化路径
2. 检查路径是否包含 ".."
3. 仅允许访问指定 basename
4. URL 解码后再验证
5. 拒绝绝对路径请求
```

### 9.2 错误信息脱敏

```python
# 不在 HTTP 响应中暴露系统路径
def do_GET(self):
    if not is_valid:
        self.send_error(404, "File not found")  # 不显示实际路径
```

### 9.3 日志安全

```python
# 不记录敏感信息
def log_message(self, format, *args):
    # 仅记录: 时间、客户端IP、请求路径、状态码
    # 不记录: 系统路径、环境变量、错误堆栈
```

## 10. 部署与打包

### 10.1 打包策略

```bash
# 使用 PyInstaller 打包成单一可执行文件
pyinstaller --onefile --name quick-share src/main.py

# 输出:
#   dist/quick-share        (Linux/macOS)
#   dist/quick-share.exe    (Windows)
```

### 10.2 依赖管理

```txt
# requirements.txt (运行时依赖)
# (仅标准库，无需额外依赖)

# requirements-dev.txt (开发依赖)
pytest>=7.0.0
pytest-cov>=3.0.0
pytest-timeout>=2.0.0
requests>=2.28.0  # 集成测试用
pyinstaller>=5.0.0
```

### 10.3 安装方式

```bash
# 方式1: 直接运行 (开发)
python src/main.py test.txt

# 方式2: 安装为命令 (pip install -e .)
quick-share test.txt

# 方式3: 使用打包后的可执行文件
./dist/quick-share test.txt
```

## 11. 技术风险与缓解

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|---------|
| socket 方法在某些网络环境下检测IP错误 | 中 | 高 | 提供手动指定IP参数 (future feature) |
| ThreadingHTTPServer 在 Python 3.7 不可用 | 低 | 中 | 检测版本，降级到 HTTPServer |
| PyInstaller 打包大小过大 | 低 | 低 | 可接受 (~10MB) |
| 防火墙阻止连接 | 高 | 中 | 文档说明，启动时提示检查防火墙 |
| 大文件传输中断 | 中 | 中 | 目前不支持断点续传 (future feature) |

## 12. 未来扩展设计

### 12.1 预留扩展点

```python
# 1. IP 手动指定
parser.add_argument('--ip', help='Manually specify IP address')

# 2. 二维码生成
def generate_qrcode(url):
    # 使用 qrcode 库生成二维码

# 3. 多文件分享
# 修改为接受多个文件参数，生成 ZIP

# 4. HTTPS 支持
# 使用 ssl.wrap_socket()
```

### 12.2 配置文件支持 (future)

```yaml
# ~/.quick-share.yaml
defaults:
  port: 8000
  max_downloads: 10
  timeout: 5m

network:
  prefer_interface: eth0
  fallback_ip: 192.168.1.100
```

## 13. 下一步行动

设计文档已完成，下一步进入 **Stage 3: Task Breakdown (任务拆分)**

将生成:
- `quick-share-tasks.md`: 详细任务列表
- 任务依赖关系
- 并行执行分组
- 每个任务的测试清单

---

**文档维护**:
- **创建时间**: 2026-01-12
- **最后更新**: 2026-01-12
- **版本**: v1.0
- **状态**: Stage 2 已完成，待用户确认
