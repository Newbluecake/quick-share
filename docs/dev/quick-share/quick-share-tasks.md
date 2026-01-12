# 任务拆分文档: Quick Share

> **生成方式**: spec-workflow-executor - Stage 3
> **生成时间**: 2026-01-12
> **基于文档**: quick-share-requirements.md, quick-share-design.md

## 1. 任务分组与优先级

### 分组策略
- **Group 1**: 基础设施和工具函数 (可并行)
- **Group 2**: 核心模块 - 网络、安全、服务器 (可并行)
- **Group 3**: CLI 和主流程集成 (依赖 Group 2)
- **Group 4**: 集成测试和优化 (依赖 Group 3)

### 优先级定义
- **P0**: 核心功能，MVP 必需
- **P1**: 重要功能，用户体验关键
- **P2**: 增强功能，锦上添花

---

## 2. 任务列表

### Group 1: 基础设施 (可并行执行)

#### T-001: 项目结构初始化
**优先级**: P0
**依赖**: 无
**推荐模型**: Haiku
**可并行**: 是

**描述**:
创建项目目录结构、配置文件、依赖管理文件

**交付物**:
- `src/` 目录及 `__init__.py`
- `tests/` 目录及 `__init__.py`
- `tests/integration/` 目录及 `__init__.py`
- `requirements.txt` (空文件，仅标准库)
- `requirements-dev.txt` (pytest, pytest-cov, requests)
- `setup.py` 或 `pyproject.toml`
- `.gitignore` (Python 模板)
- `README.md` (基础说明)

**验收标准**:
- 目录结构符合设计文档 Section 2.1
- 可执行 `pip install -e .` 安装开发版本
- 可执行 `pytest` (即使没有测试)

**关联需求**: N/A
**关联功能点**: N/A

---

#### T-002: 工具函数模块 (utils.py)
**优先级**: P0
**依赖**: T-001
**推荐模型**: Haiku
**可并行**: 与 T-003, T-004 并行

**描述**:
实现通用工具函数：文件大小格式化、时长解析

**交付物**:
- `src/utils.py`
  - `format_file_size(bytes: int) -> str`
  - `parse_duration(duration_str: str) -> int`
- `tests/test_utils.py`
  - `test_format_file_size_bytes()`
  - `test_format_file_size_kb()`
  - `test_format_file_size_mb()`
  - `test_format_file_size_gb()`
  - `test_parse_duration_seconds()`
  - `test_parse_duration_minutes()`
  - `test_parse_duration_hours()`
  - `test_parse_duration_zero()`
  - `test_parse_duration_invalid()`

**测试清单** (TDD):
```python
# Red: 先写测试
assert format_file_size(500) == "500 B"
assert format_file_size(1024) == "1.0 KB"
assert format_file_size(1536) == "1.5 KB"
assert format_file_size(1048576) == "1.0 MB"
assert format_file_size(1258291) == "1.2 MB"
assert parse_duration("30s") == 30
assert parse_duration("5m") == 300
assert parse_duration("1h") == 3600
assert parse_duration("0") == 0

# Green: 实现代码通过测试
# Refactor: 优化代码
```

**验收标准**:
- 所有单元测试通过
- 覆盖率 ≥ 90%
- 边界条件测试 (0字节, 负数, 无效格式)

**关联需求**: 需求4, 需求6
**关联功能点**: F-023

---

#### T-003: 日志格式化模块 (logger.py)
**优先级**: P1
**依赖**: T-001, T-002
**推荐模型**: Haiku
**可并行**: 与 T-002, T-004 并行

**描述**:
实现日志输出格式化函数

**交付物**:
- `src/logger.py`
  - `format_startup_message(...) -> str`
  - `format_download_log(...) -> str`
  - `format_shutdown_message(...) -> str`
- `tests/test_logger.py`
  - `test_startup_message_format()`
  - `test_download_log_format()`
  - `test_shutdown_message_format()`

**测试清单**:
```python
# 验证启动信息包含所有必需字段
msg = format_startup_message(
    ip="192.168.1.100",
    port=8000,
    filename="test.txt",
    file_size="1.2 MB",
    max_downloads=10,
    timeout=300
)
assert "192.168.1.100:8000" in msg
assert "test.txt" in msg
assert "1.2 MB" in msg
assert "curl" in msg
assert "wget" in msg

# 验证下载日志格式
log = format_download_log(
    timestamp="2026-01-12 14:30:25",
    client_ip="192.168.1.101",
    method="GET",
    path="/test.txt",
    status_code=200,
    current_count=1,
    max_count=10
)
assert "[2026-01-12 14:30:25]" in log
assert "192.168.1.101" in log
assert "1/10" in log
```

**验收标准**:
- 输出格式符合设计文档 Section 2.2.6
- 所有单元测试通过

**关联需求**: 需求4, 需求7
**关联功能点**: F-006, F-007, F-015, F-016

---

### Group 2: 核心模块 (可并行执行)

#### T-004: 网络检测模块 (network.py)
**优先级**: P0
**依赖**: T-001
**推荐模型**: Sonnet
**可并行**: 与 T-005, T-006 并行

**描述**:
实现局域网IP自动检测，支持多网卡环境，过滤无效IP

**交付物**:
- `src/network.py`
  - `get_local_ip() -> str`
  - `is_valid_lan_ip(ip: str) -> bool`
  - `get_interface_priority(interface_name: str) -> int` (可选)
- `tests/test_network.py`
  - `test_detect_local_ip()`
  - `test_skip_loopback()`
  - `test_skip_link_local()`
  - `test_valid_lan_ip_192()`
  - `test_valid_lan_ip_10()`
  - `test_valid_lan_ip_172()`
  - `test_invalid_ip_127()`
  - `test_no_valid_ip_error()`

**测试清单** (TDD):
```python
# Mock socket 测试
@patch('socket.socket')
def test_get_local_ip_success(mock_socket):
    mock_socket.return_value.getsockname.return_value = ('192.168.1.100', 0)
    ip = get_local_ip()
    assert ip == '192.168.1.100'

# 验证过滤逻辑
assert is_valid_lan_ip('192.168.1.100') == True
assert is_valid_lan_ip('10.0.0.1') == True
assert is_valid_lan_ip('172.16.0.1') == True
assert is_valid_lan_ip('127.0.0.1') == False
assert is_valid_lan_ip('169.254.1.1') == False

# 无有效IP时抛出异常
@patch('socket.socket')
def test_no_valid_ip_raises_error(mock_socket):
    mock_socket.return_value.getsockname.return_value = ('127.0.0.1', 0)
    with pytest.raises(RuntimeError, match="Cannot detect valid LAN IP"):
        get_local_ip()
```

**实现提示**:
- 优先使用 `socket.socket(AF_INET, SOCK_DGRAM).connect()` 方法
- 检测到 127.0.0.1 或 169.254.x.x 时抛出异常
- 不需要遍历所有网卡 (socket方法已自动选择最佳路由)

**验收标准**:
- 所有单元测试通过
- 能正确检测局域网IP (192.168.x.x, 10.x.x.x, 172.16-31.x.x)
- 能正确拒绝无效IP

**关联需求**: 需求1
**关联功能点**: F-001, F-002

---

#### T-005: 安全验证模块 (security.py)
**优先级**: P0
**依赖**: T-001
**推荐模型**: Sonnet
**可并行**: 与 T-004, T-006 并行

**描述**:
实现路径安全验证，防御路径遍历攻击

**交付物**:
- `src/security.py`
  - `validate_request_path(request_path: str, allowed_basename: str) -> tuple[bool, str]`
  - `is_path_traversal_attack(path: str) -> bool`
- `tests/test_security.py`
  - `test_only_basename_allowed()`
  - `test_path_traversal_blocked_double_dot()`
  - `test_path_traversal_blocked_absolute_path()`
  - `test_url_encoded_attack_blocked()`
  - `test_invalid_path_404()`
  - `test_query_string_removed()`

**测试清单** (TDD):
```python
# 正常路径
is_valid, path = validate_request_path("/test.txt", "test.txt")
assert is_valid == True
assert path == "test.txt"

# 带查询字符串
is_valid, path = validate_request_path("/test.txt?download=1", "test.txt")
assert is_valid == True

# 路径遍历 - 双点
is_valid, _ = validate_request_path("/../etc/passwd", "test.txt")
assert is_valid == False

# 路径遍历 - URL编码
is_valid, _ = validate_request_path("/%2e%2e/etc/passwd", "test.txt")
assert is_valid == False

# 绝对路径
is_valid, _ = validate_request_path("/etc/passwd", "test.txt")
assert is_valid == False

# 错误文件名
is_valid, _ = validate_request_path("/other.txt", "test.txt")
assert is_valid == False
```

**验收标准**:
- 所有单元测试通过
- 覆盖 OWASP 路径遍历攻击向量
- URL 解码后仍能正确验证

**关联需求**: 需求3
**关联功能点**: F-004, F-005

---

#### T-006: HTTP 服务器模块 (server.py)
**优先级**: P0
**依赖**: T-001, T-005
**推荐模型**: Sonnet
**可并行**: 与 T-004 并行 (但需等待 T-005)

**描述**:
实现HTTP服务器：端口查找、请求处理、下载计数、超时控制

**交付物**:
- `src/server.py`
  - `find_available_port(start=8000, end=8099, custom_port=None) -> int`
  - `is_port_available(port: int) -> bool`
  - `class FileShareHandler(BaseHTTPRequestHandler)`
  - `class FileShareServer`
- `tests/test_server.py`
  - `test_default_port_8000()`
  - `test_port_auto_increment()`
  - `test_all_ports_occupied_error()`
  - `test_custom_port()`
  - `test_custom_port_occupied_error()`
  - `test_handler_valid_request()`
  - `test_handler_invalid_path()`
  - `test_download_count_increment()`

**测试清单** (TDD):
```python
# 端口检测
assert is_port_available(8000) == True  # 假设未占用

@patch('socket.socket')
def test_port_occupied(mock_socket):
    mock_socket.return_value.bind.side_effect = OSError()
    assert is_port_available(8000) == False

# 端口查找
port = find_available_port()
assert 8000 <= port <= 8099

# 自定义端口
port = find_available_port(custom_port=9000)
assert port == 9000

# 所有端口占用
@patch('server.is_port_available', return_value=False)
def test_all_ports_occupied(mock_check):
    with pytest.raises(RuntimeError, match="No available port"):
        find_available_port()

# HTTP 请求处理 (集成测试见 T-010)
# 这里仅测试 Handler 逻辑
```

**实现提示**:
- 使用 `ThreadingHTTPServer` 支持并发
- Handler 使用类变量共享状态 (download_count, max_downloads)
- 文件使用流式传输 (8KB chunks)
- 超时使用 `threading.Timer`

**验收标准**:
- 所有单元测试通过
- 能自动查找可用端口
- 能正确处理端口占用情况

**关联需求**: 需求2, 需求5, 需求6
**关联功能点**: F-003, F-010, F-011, F-012, F-013, F-014

---

### Group 3: 集成与主流程 (依赖 Group 2)

#### T-007: 命令行参数解析 (cli.py)
**优先级**: P0
**依赖**: T-001, T-002
**推荐模型**: Haiku
**可并行**: 与 T-008 并行

**描述**:
实现命令行参数解析，支持所有配置选项

**交付物**:
- `src/cli.py`
  - `parse_arguments() -> argparse.Namespace`
  - `validate_arguments(args) -> None`
- `tests/test_cli.py`
  - `test_help_message()`
  - `test_parse_args_defaults()`
  - `test_parse_args_custom_port()`
  - `test_parse_args_custom_downloads()`
  - `test_parse_args_custom_timeout()`
  - `test_missing_file_path_error()`

**测试清单** (TDD):
```python
# 默认参数
args = parse_arguments(['test.txt'])
assert args.file_path == 'test.txt'
assert args.port is None
assert args.max_downloads == 10
assert args.timeout == '5m'

# 自定义参数
args = parse_arguments(['test.txt', '-p', '9000', '-n', '3', '-t', '10m'])
assert args.port == 9000
assert args.max_downloads == 3
assert args.timeout == '10m'

# 帮助信息
with pytest.raises(SystemExit):
    parse_arguments(['--help'])
```

**验收标准**:
- 所有单元测试通过
- 帮助信息清晰完整
- 参数验证正确

**关联需求**: 需求8
**关联功能点**: F-017, F-018, F-021

---

#### T-008: 主入口模块 (main.py)
**优先级**: P0
**依赖**: T-002, T-003, T-004, T-005, T-006, T-007
**推荐模型**: Sonnet
**可并行**: 否 (依赖所有核心模块)

**描述**:
实现主流程编排：参数解析 → 文件验证 → IP检测 → 端口查找 → 启动服务器

**交付物**:
- `src/main.py`
  - `validate_file(file_path: str) -> tuple[Path, int]`
  - `main() -> None`
- `tests/test_main.py` (单元测试)
  - `test_validate_file_success()`
  - `test_validate_file_not_found()`
  - `test_validate_file_is_directory()`
  - `test_main_integration()` (使用 Mock)

**测试清单** (TDD):
```python
# 文件验证
@pytest.fixture
def temp_file(tmp_path):
    file = tmp_path / "test.txt"
    file.write_text("hello")
    return file

def test_validate_file_success(temp_file):
    path, size = validate_file(str(temp_file))
    assert path.name == "test.txt"
    assert size == 5

def test_validate_file_not_found():
    with pytest.raises(FileNotFoundError):
        validate_file("/nonexistent.txt")

def test_validate_file_is_directory(tmp_path):
    with pytest.raises(ValueError, match="is a directory"):
        validate_file(str(tmp_path))

# 主流程集成 (Mock 所有外部调用)
@patch('main.get_local_ip', return_value='192.168.1.100')
@patch('main.find_available_port', return_value=8000)
@patch('main.FileShareServer')
def test_main_success(mock_server, mock_port, mock_ip, temp_file):
    with patch('sys.argv', ['quick-share', str(temp_file)]):
        main()
    mock_server.assert_called_once()
```

**验收标准**:
- 所有单元测试通过
- 错误处理完整 (文件不存在、IP检测失败、端口占用)
- 优雅处理 KeyboardInterrupt (Ctrl+C)

**关联需求**: 所有需求
**关联功能点**: F-019, F-020, F-022

---

### Group 4: 集成测试与优化 (依赖 Group 3)

#### T-009: 端到端下载测试
**优先级**: P0
**依赖**: T-008
**推荐模型**: Sonnet
**可并行**: 与 T-010 并行

**描述**:
实现真实 HTTP 客户端的端到端集成测试

**交付物**:
- `tests/integration/test_download.py`
  - `test_successful_download()`
  - `test_download_with_curl_compatibility()`
  - `test_download_with_browser_user_agent()`
  - `test_concurrent_downloads()`
  - `test_file_content_integrity()`

**测试清单**:
```python
import requests
import threading
import subprocess

def test_successful_download(temp_file):
    # 启动服务器在后台线程
    server_thread = threading.Thread(
        target=start_server,
        args=(temp_file, '127.0.0.1', 8000, 10, 0)
    )
    server_thread.daemon = True
    server_thread.start()
    time.sleep(0.5)  # 等待启动

    # 使用 requests 下载
    response = requests.get('http://127.0.0.1:8000/test.txt')
    assert response.status_code == 200
    assert response.content == temp_file.read_bytes()

def test_curl_compatibility(temp_file):
    # 使用真实 curl 命令测试
    result = subprocess.run(
        ['curl', 'http://127.0.0.1:8000/test.txt', '-o', '/tmp/downloaded.txt'],
        capture_output=True
    )
    assert result.returncode == 0
    assert Path('/tmp/downloaded.txt').read_bytes() == temp_file.read_bytes()
```

**验收标准**:
- 所有集成测试通过
- 能使用 curl/wget/浏览器成功下载
- 文件内容完整性校验通过

**关联需求**: 需求3, 需求4
**关联功能点**: F-006, F-007, F-008, F-009

---

#### T-010: 限制功能集成测试
**优先级**: P1
**依赖**: T-008
**推荐模型**: Sonnet
**可并行**: 与 T-009 并行

**描述**:
测试下载次数限制和超时自动关闭功能

**交付物**:
- `tests/integration/test_limits.py`
  - `test_auto_shutdown_on_max_downloads()`
  - `test_auto_shutdown_on_timeout()`
  - `test_download_progress_display()`
  - `test_disable_timeout()`

**测试清单**:
```python
def test_max_downloads_limit(temp_file):
    # 启动服务器，max_downloads=3
    server = start_server_in_thread(temp_file, max_downloads=3, timeout=0)

    # 下载3次
    for i in range(3):
        response = requests.get('http://127.0.0.1:8000/test.txt')
        assert response.status_code == 200

    # 第4次应该失败 (服务器已关闭)
    time.sleep(0.5)
    with pytest.raises(requests.ConnectionError):
        requests.get('http://127.0.0.1:8000/test.txt')

@pytest.mark.timeout(5)
def test_timeout_shutdown(temp_file):
    # 启动服务器，timeout=2秒
    server = start_server_in_thread(temp_file, max_downloads=100, timeout=2)

    # 1秒后仍可访问
    time.sleep(1)
    response = requests.get('http://127.0.0.1:8000/test.txt')
    assert response.status_code == 200

    # 3秒后应该关闭
    time.sleep(2.5)
    with pytest.raises(requests.ConnectionError):
        requests.get('http://127.0.0.1:8000/test.txt')
```

**验收标准**:
- 所有集成测试通过
- 下载次数限制准确
- 超时自动关闭准确 (误差 ±1秒)

**关联需求**: 需求5, 需求6
**关联功能点**: F-010, F-011, F-012, F-013, F-014, F-016

---

#### T-011: 打包和部署配置
**优先级**: P1
**依赖**: T-008
**推荐模型**: Haiku
**可并行**: 与 T-009, T-010 并行

**描述**:
配置项目打包，生成可执行文件

**交付物**:
- `setup.py` 或 `pyproject.toml` (完善)
- `build.sh` 脚本 (PyInstaller 打包)
- `README.md` (使用说明)
- `CHANGELOG.md` (版本记录)

**配置清单**:
```python
# setup.py
setup(
    name='quick-share',
    version='1.0.0',
    packages=['src'],
    entry_points={
        'console_scripts': [
            'quick-share=src.main:main',
        ],
    },
    python_requires='>=3.8',
)

# build.sh
#!/bin/bash
pyinstaller --onefile \
            --name quick-share \
            --add-data "README.md:." \
            src/main.py
```

**验收标准**:
- 可执行 `pip install -e .` 安装
- 可执行 `quick-share` 命令
- 可打包成单一可执行文件 (Linux/macOS/Windows)
- README 包含安装和使用说明

**关联需求**: 技术约束 Section 5.1
**关联功能点**: N/A

---

#### T-012: 文档和示例完善
**优先级**: P2
**依赖**: T-011
**推荐模型**: Haiku
**可并行**: 是

**描述**:
完善用户文档、示例、故障排查指南

**交付物**:
- `README.md` (完整版)
  - 功能介绍
  - 安装方法
  - 使用示例
  - 参数说明
  - 常见问题
- `docs/TROUBLESHOOTING.md`
  - 防火墙配置
  - 网络问题排查
  - 常见错误码
- `examples/` 目录
  - 示例文件
  - 使用场景演示

**验收标准**:
- 文档清晰完整
- 包含至少 3 个使用示例
- 包含常见问题解答

**关联需求**: N/A
**关联功能点**: N/A

---

## 3. 任务依赖关系图

```
┌─────────────────────────────────────────────────────────────────┐
│ Group 1: 基础设施 (可并行)                                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  T-001: 项目初始化                                               │
│     ├─→ T-002: utils.py (工具函数)                               │
│     ├─→ T-003: logger.py (日志) ← T-002                          │
│     ├─→ T-004: network.py (网络检测)                              │
│     ├─→ T-005: security.py (安全验证)                             │
│     └─→ T-006: server.py (HTTP服务器) ← T-005                     │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────────────┐
│ Group 2: 核心集成 (部分并行)                                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  T-007: cli.py (CLI参数) ← T-002                                 │
│     ↓                                                           │
│  T-008: main.py (主流程) ← T-002, T-003, T-004, T-005, T-006, T-007│
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────────────┐
│ Group 3: 测试与打包 (可并行)                                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  T-009: 端到端下载测试 ← T-008                                     │
│  T-010: 限制功能测试 ← T-008                                      │
│  T-011: 打包配置 ← T-008                                         │
│     ↓                                                           │
│  T-012: 文档完善 ← T-011                                         │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## 4. 并行执行计划

### 执行批次

**Batch 1** (并行执行):
- T-001: 项目初始化

**Batch 2** (并行执行，依赖 T-001):
- T-002: utils.py
- T-004: network.py
- T-005: security.py

**Batch 3** (并行执行，依赖 Batch 2):
- T-003: logger.py (依赖 T-002)
- T-006: server.py (依赖 T-005)
- T-007: cli.py (依赖 T-002)

**Batch 4** (串行执行，依赖 Batch 3):
- T-008: main.py (依赖所有核心模块)

**Batch 5** (并行执行，依赖 T-008):
- T-009: 端到端测试
- T-010: 限制功能测试
- T-011: 打包配置

**Batch 6** (串行执行，依赖 T-011):
- T-012: 文档完善

---

## 5. 任务与需求映射

| 任务 | 关联需求 | 关联功能点 |
|------|---------|-----------|
| T-001 | N/A | N/A |
| T-002 | 需求4, 需求6 | F-023 |
| T-003 | 需求4, 需求7 | F-006, F-007, F-015, F-016 |
| T-004 | 需求1 | F-001, F-002 |
| T-005 | 需求3 | F-004, F-005 |
| T-006 | 需求2, 需求5, 需求6 | F-003, F-010, F-011, F-012, F-013, F-014 |
| T-007 | 需求8 | F-017, F-018, F-021 |
| T-008 | 所有需求 | F-019, F-020, F-022 |
| T-009 | 需求3, 需求4 | F-006, F-007, F-008, F-009 |
| T-010 | 需求5, 需求6 | F-010, F-011, F-012, F-013, F-014, F-016 |
| T-011 | 技术约束 | N/A |
| T-012 | N/A | N/A |

---

## 6. 风险与注意事项

### 高风险任务

| 任务 | 风险 | 缓解措施 |
|------|------|---------|
| T-004 | socket方法在某些环境下可能失败 | 提供降级方案 (遍历接口) |
| T-005 | 路径遍历攻击向量可能遗漏 | 参考 OWASP 测试用例，全面覆盖 |
| T-006 | 并发下载时计数器可能不准确 | 使用线程锁保护共享变量 |
| T-009 | 集成测试依赖真实网络环境 | 使用 127.0.0.1 localhost 测试 |

### 关键决策点

1. **T-004**: 如果 socket 方法不稳定，是否引入 `netifaces` 依赖？
   - 决策: 先实现 socket 方法，测试失败再考虑

2. **T-006**: 是否支持 Python 3.7？
   - 决策: 仅支持 3.8+ (ThreadingHTTPServer 在 3.7 需额外实现)

3. **T-011**: 打包工具选择 PyInstaller 还是其他？
   - 决策: PyInstaller (成熟稳定，跨平台)

---

## 7. 评审检查点

### 每个任务完成后

**Stage 1: 规范符合性评审** (task-compliance-checker)
- 是否实现了所有交付物？
- 测试是否覆盖了所有验收标准？
- 是否遵循 TDD 流程 (Red → Green → Refactor)？
- 代码是否符合需求文档的约束？

**Stage 2: 代码质量评审** (code-architecture-reviewer)
- 代码是否清晰可读？
- 是否有适当的错误处理？
- 是否有代码重复 (DRY 原则)？
- 是否有安全漏洞？
- 测试覆盖率是否足够？

---

## 8. 完成标准

### 整体验收标准 (release-acceptance-reviewer)

**功能完整性**:
- 所有 P0 任务 100% 完成
- 所有 P1 任务 ≥ 90% 完成
- 所有功能点 (F-001 ~ F-023) 标记为 ✅

**质量标准**:
- 单元测试覆盖率 ≥ 85%
- 集成测试全部通过
- 无 Critical/High 优先级 Bug

**文档完整性**:
- README.md 包含安装和使用说明
- 所有公共函数有 docstring
- 至少 3 个使用示例

**跨平台验证**:
- Linux 测试通过
- macOS 测试通过 (可选)
- Windows 测试通过 (可选)

---

## 9. 时间估算

| 任务 | 预估时间 | 复杂度 |
|------|---------|--------|
| T-001 | 15min | 简单 |
| T-002 | 30min | 简单 |
| T-003 | 30min | 简单 |
| T-004 | 1h | 中等 |
| T-005 | 1h | 中等 |
| T-006 | 2h | 复杂 |
| T-007 | 30min | 简单 |
| T-008 | 1.5h | 中等 |
| T-009 | 1h | 中等 |
| T-010 | 1h | 中等 |
| T-011 | 30min | 简单 |
| T-012 | 30min | 简单 |
| **总计** | **~10h** | |

**并行执行后预估**: ~5-6h (考虑并行和评审时间)

---

## 10. 下一步行动

任务拆分已完成，等待用户确认后进入 **Execution Phase (阶段4/5)**

确认后将执行:
1. 创建 feature 分支
2. 按 Batch 顺序执行任务
3. 每个任务遵循 TDD: Red → Green → Refactor
4. 每个任务完成后进行两阶段评审 (complexity=standard)
5. 每个 Batch 完成后请求用户确认

---

**文档维护**:
- **创建时间**: 2026-01-12
- **最后更新**: 2026-01-12
- **版本**: v1.0
- **状态**: Stage 3 已完成，待用户确认
