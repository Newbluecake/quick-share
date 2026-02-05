---
feature: console-download-progress
complexity: standard
generated_by: architect-planner
generated_at: 2025-02-05T11:20:00Z
version: 1
---

# 技术设计文档: Console Download Progress

> **功能标识**: console-download-progress
> **复杂度**: standard
> **生成方式**: architect-planner
> **生成时间**: 2025-02-05

## 1. 系统架构设计

### 1.1 整体架构

```
┌─────────────────────────────────────────────────────────────┐
│                      HTTP Request                           │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│              BaseHTTPRequestHandler                         │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  DownloadProgressTracker (线程安全)                    │  │
│  │  - bytes_transferred: int                             │  │
│  │  - start_time: float                                  │  │
│  │  - client_ip: str                                     │  │
│  │  - file_size: int                                     │  │
│  └───────────────────────────────────────────────────────┘  │
│                           │                                  │
│                           ▼                                  │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  Progress Callback (每 chunk)                         │  │
│  │  - format_download_start()                           │  │
│  │  - format_download_progress()                        │  │
│  │  - format_download_complete()                        │  │
│  │  - format_download_error()                           │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                     Console Output                          │
│  [2025-02-05 10:30:45] ⬇️  192.168.1.100 - file.zip (2.5MB) │
│  [2025-02-05 10:30:46] ⬇️  192.168.1.100 - 1.2MB / 2.5MB (48%) │
│  [2025-02-05 10:30:47] ✅ 192.168.1.100 - Completed (2.5MB in 2s) │
└─────────────────────────────────────────────────────────────┘
```

### 1.2 设计原则

1. **非侵入式**: 通过回调函数集成，最小化对现有代码的修改
2. **线程安全**: 支持并发下载，每个连接独立的进度跟踪器
3. **性能优先**: 日志输出不阻塞文件传输
4. **异常友好**: 优雅处理客户端中断，避免堆栈跟踪污染输出

---

## 2. 核心组件设计

### 2.1 DownloadProgressTracker 类

**职责**: 跟踪单个下载会话的进度状态

```python
class DownloadProgressTracker:
    """Track download progress for a single client connection."""

    def __init__(self, client_ip: str, filename: str, file_size: int):
        """
        Initialize progress tracker.

        Args:
            client_ip: Client IP address
            filename: Download filename
            file_size: Total file size in bytes
        """
        self.client_ip = client_ip
        self.filename = filename
        self.file_size = file_size
        self.bytes_transferred = 0
        self.start_time = time.time()
        self.is_complete = False

    def update(self, chunk_size: int) -> bool:
        """
        Update progress after each chunk.

        Args:
            chunk_size: Size of transferred chunk

        Returns:
            True if should log progress (every N chunks to avoid spam)
        """
        self.bytes_transferred += chunk_size
        # Log every 10 chunks (~80KB) to avoid excessive output
        return self.bytes_transferred % (CHUNK_SIZE * 10) == 0 or self.bytes_transferred == self.file_size

    def complete(self):
        """Mark download as complete."""
        self.is_complete = True

    def get_progress_percentage(self) -> float:
        """Calculate progress percentage."""
        if self.file_size == 0:
            return 0.0
        return (self.bytes_transferred / self.file_size) * 100
```

**线程安全性**:
- 每个连接创建独立的 tracker 实例
- 不共享状态，无需锁机制
- ThreadingHTTPServer 保证每个请求在独立线程中处理

### 2.2 日志格式化函数

**位置**: `src/logger.py`

#### 2.2.1 format_download_start

```python
def format_download_start(
    timestamp: str,
    client_ip: str,
    filename: str,
    file_size: str
) -> str:
    """
    Format download start log entry.

    Args:
        timestamp: Formatted timestamp [YYYY-MM-DD HH:MM:SS]
        client_ip: Client IP address
        filename: Download filename
        file_size: Human-readable file size (e.g., "2.5MB")

    Returns:
        Formatted log string

    Example:
        [2025-02-05 10:30:45] ⬇️  192.168.1.100 - file.zip (2.5MB)
    """
```

#### 2.2.2 format_download_progress

```python
def format_download_progress(
    timestamp: str,
    client_ip: str,
    bytes_transferred: int,
    total_bytes: int,
    percentage: float
) -> str:
    """
    Format download progress log entry.

    Args:
        timestamp: Formatted timestamp
        client_ip: Client IP address
        bytes_transferred: Transferred bytes
        total_bytes: Total file size in bytes
        percentage: Progress percentage (0-100)

    Returns:
        Formatted log string

    Example:
        [2025-02-05 10:30:46] ⬇️  192.168.1.100 - 1.2MB / 2.5MB (48%)
    """
```

#### 2.2.3 format_download_complete

```python
def format_download_complete(
    timestamp: str,
    client_ip: str,
    filename: str,
    total_bytes: int,
    duration_sec: float
) -> str:
    """
    Format download completion log entry.

    Args:
        timestamp: Formatted timestamp
        client_ip: Client IP address
        filename: Download filename
        total_bytes: Total transferred bytes
        duration_sec: Transfer duration in seconds

    Returns:
        Formatted log string

    Example:
        [2025-02-05 10:30:47] ✅ 192.168.1.100 - Completed: file.zip (2.5MB in 2.3s)
    """
```

#### 2.2.4 format_download_interrupted

```python
def format_download_interrupted(
    timestamp: str,
    client_ip: str,
    filename: str,
    bytes_transferred: int,
    total_bytes: int
) -> str:
    """
    Format download interruption log entry.

    Args:
        timestamp: Formatted timestamp
        client_ip: Client IP address
        filename: Download filename
        bytes_transferred: Transferred bytes before interruption
        total_bytes: Total file size

    Returns:
        Formatted log string

    Example:
        [2025-02-05 10:30:46] ⚠️  192.168.1.100 - Interrupted: file.zip (1.2MB / 2.5MB transferred)
    """
```

#### 2.2.5 format_download_error

```python
def format_download_error(
    timestamp: str,
    client_ip: str,
    filename: str,
    error_message: str
) -> str:
    """
    Format download error log entry.

    Args:
        timestamp: Formatted timestamp
        client_ip: Client IP address
        filename: Download filename
        error_message: Error description

    Returns:
        Formatted log string

    Example:
        [2025-02-05 10:30:45] ❌ 192.168.1.100 - Error: file.zip - File not found
    """
```

---

## 3. 接口设计

### 3.1 FileShareHandler._stream_file 修改

**位置**: `src/server.py:86-102`

**修改前**:
```python
def _stream_file(self, file_path: str, filename: str):
    """Stream a file to the client in chunks."""
    file_size = os.path.getsize(file_path)

    self.send_response(200)
    self.send_header('Content-Type', 'application/octet-stream')
    self.send_header('Content-Disposition', f'attachment; filename="{filename}"')
    self.send_header('Content-Length', str(file_size))
    self.end_headers()

    # Stream file in chunks
    with open(file_path, 'rb') as f:
        while True:
            chunk = f.read(CHUNK_SIZE)
            if not chunk:
                break
            self.wfile.write(chunk)
```

**修改后**:
```python
def _stream_file(self, file_path: str, filename: str):
    """Stream a file to the client in chunks with progress tracking."""
    from .logger import format_download_start, format_download_progress, format_download_complete, format_download_interrupted
    import datetime

    file_size = os.path.getsize(file_path)
    client_ip = self.client_address[0]

    self.send_response(200)
    self.send_header('Content-Type', 'application/octet-stream')
    self.send_header('Content-Disposition', f'attachment; filename="{filename}"')
    self.send_header('Content-Length', str(file_size))
    self.end_headers()

    # Initialize progress tracker
    tracker = DownloadProgressTracker(client_ip, filename, file_size)

    # Log download start
    from .directory_handler import format_file_size
    timestamp = datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')
    print(format_download_start(timestamp, client_ip, filename, format_file_size(file_size)))

    # Stream file in chunks
    try:
        with open(file_path, 'rb') as f:
            while True:
                chunk = f.read(CHUNK_SIZE)
                if not chunk:
                    break

                self.wfile.write(chunk)

                # Update progress
                if tracker.update(len(chunk)):
                    percentage = tracker.get_progress_percentage()
                    timestamp = datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')
                    print(format_download_progress(
                        timestamp,
                        client_ip,
                        tracker.bytes_transferred,
                        file_size,
                        percentage
                    ))

        # Log completion
        tracker.complete()
        duration = time.time() - tracker.start_time
        timestamp = datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')
        print(format_download_complete(timestamp, client_ip, filename, file_size, duration))

    except (BrokenPipeError, ConnectionResetError) as e:
        # Client disconnected
        timestamp = datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')
        print(format_download_interrupted(
            timestamp,
            client_ip,
            filename,
            tracker.bytes_transferred,
            file_size
        ))
    except Exception as e:
        # Other errors
        timestamp = datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')
        print(format_download_error(timestamp, client_ip, filename, str(e)))
```

### 3.2 DirectoryShareHandler._stream_file_with_headers 修改

**位置**: `src/server.py:369-386`

应用与 `_stream_file` 相同的修改逻辑，确保目录共享中的文件下载也支持进度显示。

### 3.3 stream_directory_as_zip 修改

**位置**: `src/directory_handler.py:249-275`

**挑战**: ZIP 流式写入的进度跟踪较为复杂，需要在每个文件写入后更新进度。

**修改策略**:
1. 在函数签名中添加可选的 `progress_callback` 参数
2. 在写入每个文件后调用回调
3. 回调函数输出进度到 console

**修改后**:
```python
def stream_directory_as_zip(
    output_stream,
    base_dir: str,
    target_dir: str,
    progress_callback=None
) -> None:
    """
    Stream directory as zip file with optional progress callback.

    Args:
        output_stream: Output stream (HTTP response wfile)
        base_dir: Shared root directory
        target_dir: Target directory to zip (may be subdirectory)
        progress_callback: Optional callback function(current_bytes, total_bytes)
    """
    import datetime

    if progress_callback:
        from .logger import format_download_start, format_download_progress, format_download_complete
        from .utils import get_directory_info
        import time

        # Estimate total size (not accurate for compressed zip, but good enough for progress)
        dir_info = get_directory_info(target_dir)
        total_size = dir_info['total_size']

        client_ip = "unknown"  # Not available in this context
        dir_name = os.path.basename(base_dir)
        timestamp = datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')

        print(format_download_start(timestamp, client_ip, f"{dir_name}.zip", format_file_size(total_size)))

        start_time = time.time()
        bytes_processed = 0

    # Use streaming zip writing
    with zipfile.ZipFile(output_stream, 'w', zipfile.ZIP_DEFLATED) as zipf:
        for root, dirs, files in os.walk(target_dir):
            for file in files:
                file_path = os.path.join(root, file)

                # Calculate relative path within zip
                arcname = os.path.relpath(file_path, base_dir)

                try:
                    zipf.write(file_path, arcname)

                    if progress_callback:
                        # Update progress (approximate)
                        try:
                            file_size = os.path.getsize(file_path)
                            bytes_processed += file_size

                            # Log every 10 files or every 10MB
                            if bytes_processed % (10 * 1024 * 1024) < 1024 * 1024:
                                percentage = min((bytes_processed / total_size) * 100, 99)
                                timestamp = datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')
                                print(format_download_progress(
                                    timestamp,
                                    client_ip,
                                    bytes_processed,
                                    total_size,
                                    percentage
                                ))
                        except OSError:
                            pass

                except (OSError, PermissionError):
                    # Skip files we can't read
                    continue

    if progress_callback:
        duration = time.time() - start_time
        timestamp = datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')
        print(format_download_complete(timestamp, client_ip, f"{dir_name}.zip", bytes_processed, duration))
```

---

## 4. 数据设计

### 4.1 进度数据结构

```python
# 简化设计：每个连接独立的 tracker 实例
class DownloadProgressTracker:
    # Attributes
    client_ip: str           # 客户端 IP
    filename: str            # 文件名
    file_size: int           # 文件总大小（字节）
    bytes_transferred: int   # 已传输字节
    start_time: float        # 开始时间（epoch timestamp）
    is_complete: bool        # 是否完成
```

### 4.2 时间戳格式

```
格式: YYYY-MM-DD HH:MM:SS
示例: 2025-02-05 10:30:45
```

### 4.3 日志输出格式

| 事件 | 格式 |
|------|------|
| 开始 | `[时间] ⬇️  IP - 文件名 (大小)` |
| 进度 | `[时间] ⬇️  IP - 已传/总数 (百分比%)` |
| 完成 | `[时间] ✅ IP - Completed: 文件名 (大小 in 耗时)` |
| 中断 | `[时间] ⚠️  IP - Interrupted: 文件名 (已传/总数)` |
| 错误 | `[时间] ❌ IP - Error: 文件名 - 原因` |

---

## 5. 异常处理设计

### 5.1 异常分类

| 异常类型 | 处理策略 | 日志级别 |
|---------|---------|---------|
| `BrokenPipeError` | 客户端中断，友好提示 | INFO (⚠️) |
| `ConnectionResetError` | 连接重置，友好提示 | INFO (⚠️) |
| `FileNotFoundError` | 文件不存在，错误日志 | ERROR (❌) |
| `PermissionError` | 权限错误，错误日志 | ERROR (❌) |
| `OSError` | 其他 IO 错误，错误日志 | ERROR (❌) |

### 5.2 异常处理流程

```python
try:
    # Stream file with progress tracking
    ...
except BrokenPipeError:
    # Client disconnected (Ctrl+C, network loss)
    log_interrupted(tracker)
except ConnectionResetError:
    # Connection reset by peer
    log_interrupted(tracker)
except (FileNotFoundError, PermissionError) as e:
    # File access errors
    log_error(str(e))
except Exception as e:
    # Unexpected errors (log but don't crash)
    log_error(f"Unexpected error: {str(e)}")
```

**关键原则**:
- 不向上抛出异常（让 handler 自然结束）
- 不打印堆栈跟踪（污染控制台）
- 所有异常都输出结构化日志

---

## 6. 并发控制设计

### 6.1 并发模型

```
ThreadingHTTPServer
    ├── Thread 1: Request from 192.168.1.100
    │   └── DownloadProgressTracker-1 (独立实例)
    ├── Thread 2: Request from 192.168.1.101
    │   └── DownloadProgressTracker-2 (独立实例)
    └── Thread 3: Request from 192.168.1.102
        └── DownloadProgressTracker-3 (独立实例)
```

### 6.2 线程安全性保证

1. **无共享状态**: 每个连接独立的 tracker 实例
2. **线程局部数据**: `tracker` 存储在函数局部变量中
3. **原子操作**: Python 的 `print()` 函数是线程安全的（内部有 GIL 保护）
4. **无锁设计**: 不需要锁，因为没有共享可变状态

### 6.3 并发测试场景

- 10 个并发下载同一文件
- 5 个并发下载不同文件
- 混合场景：文件下载 + 目录 ZIP 下载

---

## 7. 性能优化

### 7.1 日志输出频率控制

**问题**: 每 8KB 输出一次日志会导致大量输出

**解决方案**:
- 每 10 个 chunk (~80KB) 输出一次进度
- 或者每次更新百分比整数时输出

```python
def update(self, chunk_size: int) -> bool:
    """Update progress and return True if should log."""
    self.bytes_transferred += chunk_size

    # Log every 10 chunks or when complete
    return (self.bytes_transferred % (CHUNK_SIZE * 10) == 0) or (self.bytes_transferred == self.file_size)
```

### 7.2 文件大小格式化缓存

**问题**: 频繁调用 `format_file_size()` 可能影响性能

**解决方案**:
- 在 tracker 初始化时格式化一次
- 缓存结果，避免重复计算

### 7.3 ZIP 压缩进度近似

**挑战**: ZIP 压缩后大小无法准确预估

**解决方案**:
- 使用未压缩的总文件大小作为基数
- 进度百分比可能超过 100%（压缩后更小）
- 在日志中标注 "approximate"

---

## 8. 集成测试场景

### 8.1 单文件下载测试

```bash
# 启动服务
python -m quick_share share testfile.zip

# 客户端下载
wget http://localhost:8000/testfile.zip

# 预期输出
[2025-02-05 10:30:45] ⬇️  127.0.0.1 - testfile.zip (2.5MB)
[2025-02-05 10:30:46] ⬇️  127.0.0.1 - 1.2MB / 2.5MB (48%)
[2025-02-05 10:30:47] ✅ 127.0.0.1 - Completed: testfile.zip (2.5MB in 2.3s)
```

### 8.2 并发下载测试

```bash
# 同时启动 3 个 wget
wget http://localhost:8000/file.zip &
wget http://localhost:8000/file.zip &
wget http://localhost:8000/file.zip &

# 预期输出：3 条独立的进度日志
[2025-02-05 10:30:45] ⬇️  127.0.0.1 - file.zip (2.5MB)
[2025-02-05 10:30:45] ⬇️  127.0.0.1 - file.zip (2.5MB)
[2025-02-05 10:30:45] ⬇️  127.0.0.1 - file.zip (2.5MB)
[2025-02-05 10:30:46] ⬇️  127.0.0.1 - 1.2MB / 2.5MB (48%)
...
```

### 8.3 中断测试

```bash
# 启动下载后按 Ctrl+C
wget http://localhost:8000/largefile.zip
# ^C

# 预期输出
[2025-02-05 10:30:45] ⬇️  127.0.0.1 - largefile.zip (100MB)
[2025-02-05 10:30:46] ⬇️  127.0.0.1 - 10MB / 100MB (10%)
[2025-02-05 10:30:47] ⚠️  127.0.0.1 - Interrupted: largefile.zip (10MB / 100MB transferred)
```

---

## 9. 向后兼容性

### 9.1 现有 API 兼容

- `stream_directory_as_zip` 的 `progress_callback` 参数是可选的
- 不传递回调时，行为与之前完全一致
- 不影响现有调用者

### 9.2 配置兼容

- 不需要新增配置参数
- 始终启用进度日志（符合需求文档的"排除项"）
- 不提供关闭开关

---

## 10. 实施风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| 日志输出影响性能 | 高 | 限制日志频率（每 80KB 而非 8KB） |
| ZIP 进度不准确 | 中 | 使用近似值，在日志中说明 |
| Windows emoji 显示问题 | 低 | UTF-8 编码，现代终端都支持 |
| 并发日志混乱 | 低 | 使用 print() 的原子性保证 |

---

**维护者**: AI Team
**版本**: v1.0.0
**更新日期**: 2025-02-05
