---
feature: console-download-progress
complexity: standard
generated_by: architect-planner
generated_at: 2025-02-05T11:25:00Z
version: 1
---

# 任务拆分文档: Console Download Progress

> **功能标识**: console-download-progress
> **复杂度**: standard
> **生成方式**: architect-planner
> **生成时间**: 2025-02-05

## 任务概览

| 任务组 | 任务数 | 总估算 | 并行度 |
|-------|-------|--------|--------|
| 基础设施 | 2 | 30m | 可并行 |
| 日志格式化 | 3 | 45m | 可并行 |
| 文件下载进度 | 2 | 60m | 串行 |
| 目录下载进度 | 1 | 45m | 串行 |
| 测试与验证 | 2 | 60m | 可并行 |
| **总计** | **10** | **4h** | - |

---

## 依赖关系

```
T-001 (DownloadProgressTracker)
    ↓
T-002 (format_download_start)
    ↓
T-003 (format_download_progress) ──┐
    ↓                              │
T-004 (format_download_complete)   │
    ↓                              │
T-005 (format_download_error) ─────┤
    ↓                              │
T-006 (_stream_file integration)   │
    ↓                              │
T-007 (_stream_file_with_headers)  │
    ↓                              ↓
T-008 (stream_directory_as_zip)    │
    ↓                              ↓
T-009 (Unit tests) ←───────────────┘
    ↓
T-010 (Integration tests)
```

---

## 详细任务列表

### 任务组 1: 基础设施

#### T-001: 实现 DownloadProgressTracker 类

**优先级**: P0
**复杂度**: Simple
**估算**: 15m
**文件**: `src/server.py`（新增）

**描述**:
创建线程安全的下载进度跟踪器类，用于跟踪单个下载会话的进度状态。

**验收标准**:
- [ ] 创建 `DownloadProgressTracker` 类
- [ ] 实现 `__init__(client_ip, filename, file_size)` 方法
- [ ] 实现 `update(chunk_size)` 方法，返回是否应该记录日志
- [ ] 实现 `complete()` 方法
- [ ] 实现 `get_progress_percentage()` 方法
- [ ] 添加类文档字符串

**代码位置**: 在 `src/server.py` 顶部添加（在 CHUNK_SIZE 常量之后）

**依赖**: 无

---

#### T-002: 添加时间戳格式化工具函数

**优先级**: P0
**复杂度**: Simple
**估算**: 15m
**文件**: `src/logger.py`（新增）

**描述**:
创建统一的时间戳格式化函数，用于所有日志输出。

**验收标准**:
- [ ] 创建 `get_timestamp()` 函数，返回 `[YYYY-MM-DD HH:MM:SS]` 格式
- [ ] 添加函数文档字符串
- [ ] 添加单元测试（可选）

**依赖**: 无

---

### 任务组 2: 日志格式化函数

#### T-003: 实现 format_download_start 和 format_download_progress

**优先级**: P0
**复杂度**: Simple
**估算**: 15m
**文件**: `src/logger.py`

**描述**:
实现下载开始和进度日志的格式化函数。

**验收标准**:
- [ ] 实现 `format_download_start(timestamp, client_ip, filename, file_size)` 函数
- [ ] 实现 `format_download_progress(timestamp, client_ip, bytes_transferred, total_bytes, percentage)` 函数
- [ ] 使用 emoji 符号（⬇️）
- [ ] 包含时间戳、IP、文件信息、进度
- [ ] 添加函数文档字符串
- [ ] 人类可读的文件大小格式（调用 `format_file_size()`）

**示例输出**:
```
[2025-02-05 10:30:45] ⬇️  192.168.1.100 - file.zip (2.5MB)
[2025-02-05 10:30:46] ⬇️  192.168.1.100 - 1.2MB / 2.5MB (48%)
```

**依赖**: T-002

---

#### T-004: 实现 format_download_complete 和 format_download_interrupted

**优先级**: P0
**复杂度**: Simple
**估算**: 15m
**文件**: `src/logger.py`

**描述**:
实现下载完成和中断日志的格式化函数。

**验收标准**:
- [ ] 实现 `format_download_complete(timestamp, client_ip, filename, total_bytes, duration_sec)` 函数
- [ ] 实现 `format_download_interrupted(timestamp, client_ip, filename, bytes_transferred, total_bytes)` 函数
- [ ] 使用 emoji 符号（✅ 和 ⚠️）
- [ ] 包含耗时信息（完成时）
- [ ] 添加函数文档字符串

**示例输出**:
```
[2025-02-05 10:30:47] ✅ 192.168.1.100 - Completed: file.zip (2.5MB in 2.3s)
[2025-02-05 10:30:46] ⚠️  192.168.1.100 - Interrupted: file.zip (1.2MB / 2.5MB transferred)
```

**依赖**: T-002

---

#### T-005: 实现 format_download_error

**优先级**: P0
**复杂度**: Simple
**估算**: 15m
**文件**: `src/logger.py`

**描述**:
实现下载错误日志的格式化函数。

**验收标准**:
- [ ] 实现 `format_download_error(timestamp, client_ip, filename, error_message)` 函数
- [ ] 使用 emoji 符号（❌）
- [ ] 包含清晰的错误描述
- [ ] 添加函数文档字符串

**示例输出**:
```
[2025-02-05 10:30:45] ❌ 192.168.1.100 - Error: file.zip - File not found
```

**依赖**: T-002

---

### 任务组 3: 文件下载进度集成

#### T-006: 修改 FileShareHandler._stream_file 方法

**优先级**: P0
**复杂度**: Medium
**估算**: 30m
**文件**: `src/server.py:86-102`

**描述**:
修改 `_stream_file` 方法，集成进度跟踪和日志输出。

**验收标准**:
- [ ] 导入必要的日志格式化函数
- [ ] 在方法开始时创建 `DownloadProgressTracker` 实例
- [ ] 在发送 headers 前输出开始日志
- [ ] 在 while 循环中每 N 个 chunk 输出进度日志
- [ ] 捕获 `BrokenPipeError` 和 `ConnectionResetError`，输出中断日志
- [ ] 捕获其他异常，输出错误日志
- [ ] 下载完成时输出完成日志（包含耗时）
- [ ] 不向上抛出异常（让 handler 自然结束）
- [ ] 不打印堆栈跟踪

**修改点**:
1. 导入语句（在文件顶部）
2. `_stream_file` 方法体
3. 异常处理逻辑

**依赖**: T-001, T-003, T-004, T-005

---

#### T-007: 修改 DirectoryShareHandler._stream_file_with_headers 方法

**优先级**: P0
**复杂度**: Medium
**估算**: 30m
**文件**: `src/server.py:369-386`

**描述**:
修改 `_stream_file_with_headers` 方法，应用与 `_stream_file` 相同的进度跟踪逻辑。

**验收标准**:
- [ ] 复用 T-006 的实现逻辑
- [ ] 确保目录共享中的文件下载也支持进度显示
- [ ] 保持会话 cookie 设置功能不变
- [ ] 测试目录共享中的单文件下载

**依赖**: T-006

---

### 任务组 4: 目录下载进度

#### T-008: 修改 stream_directory_as_zip 支持 ZIP 下载进度

**优先级**: P0
**复杂度**: Medium
**估算**: 45m
**文件**: `src/directory_handler.py:249-275`

**描述**:
修改 `stream_directory_as_zip` 函数，添加可选的进度回调，支持 ZIP 下载进度显示。

**验收标准**:
- [ ] 添加可选的 `progress_callback` 参数（默认 None）
- [ ] 当提供回调时，在函数开始时输出开始日志
- [ ] 在写入每个文件后更新进度（近似）
- [ ] 在函数结束时输出完成日志
- [ ] 不提供回调时，行为与之前完全一致
- [ ] 处理 ZIP 写入异常（复用现有逻辑）
- [ ] 使用未压缩的总文件大小作为进度基数

**修改点**:
1. 函数签名（添加 progress_callback 参数）
2. 函数体（添加进度跟踪逻辑）
3. 异常处理（保持现有逻辑）

**依赖**: T-003, T-004

---

#### T-009: 在 DirectoryShareHandler._serve_directory_zip 中集成 ZIP 进度

**优先级**: P0
**复杂度**: Simple
**估算**: 10m
**文件**: `src/server.py:388-406`

**描述**:
修改 `_serve_directory_zip` 方法，调用 `stream_directory_as_zip` 时传递进度回调函数。

**验收标准**:
- [ ] 定义内联回调函数（或使用 lambda）
- [ ] 回调函数输出格式化的进度日志
- [ ] 仅在有回调需求时传递（简化实现：始终传递）
- [ ] 测试目录 ZIP 下载时的进度输出

**依赖**: T-008

---

### 任务组 5: 测试与验证

#### T-010: 编写单元测试

**优先级**: P0
**复杂度**: Medium
**估算**: 30m
**文件**: `tests/test_server.py`, `tests/test_logger.py`（新增或修改）

**描述**:
为核心组件编写单元测试，确保功能正确性。

**验收标准**:
- [ ] 测试 `DownloadProgressTracker.update()` 方法
  - 验证进度计算正确
  - 验证日志返回值（每 N 个 chunk 返回 True）
- [ ] 测试 `DownloadProgressTracker.get_progress_percentage()` 方法
  - 验证百分比计算（0-100）
  - 验证边界情况（file_size=0）
- [ ] 测试日志格式化函数
  - 验证输出格式符合规范
  - 验证 emoji 符号正确
  - 验证时间戳格式
- [ ] 测试异常处理
  - 模拟 BrokenPipeError
  - 模拟 FileNotFoundError

**依赖**: T-001, T-003, T-004, T-005, T-006

---

#### T-011: 编写集成测试和手动验证

**优先级**: P0
**复杂度**: Medium
**估算**: 30m
**文件**: `tests/integration/test_download_progress.py`（新增）

**描述**:
编写端到端集成测试，并执行手动验证场景。

**验收标准**:
- [ ] 集成测试：单文件下载
  - 启动服务器
  - 下载文件
  - 验证输出包含开始、进度、完成日志
- [ ] 集成测试：并发下载
  - 同时启动 3 个下载
  - 验证 3 条独立的进度日志
  - 验证日志不混乱
- [ ] 集成测试：下载中断
  - 开始下载后中断连接
  - 验证输出中断日志（⚠️）
- [ ] 手动验证：目录 ZIP 下载
  - 共享目录
  - 下载 ZIP 文件
  - 验证输出 ZIP 下载进度
- [ ] 手动验证：Windows 兼容性
  - 在 PowerShell 中测试 emoji 显示
  - 验证 UTF-8 编码正确

**依赖**: T-006, T-007, T-009

---

## 并行分组

### 组 A: 基础设施（可并行）
- T-001: DownloadProgressTracker 类
- T-002: 时间戳格式化工具

**预计耗时**: 15m

---

### 组 B: 日志格式化（可并行，依赖组 A）
- T-003: format_download_start 和 format_download_progress
- T-004: format_download_complete 和 format_download_interrupted
- T-005: format_download_error

**预计耗时**: 15m
**依赖**: T-002

---

### 组 C: 核心集成（串行）
- T-006: _stream_file 集成
- T-007: _stream_file_with_headers 集成

**预计耗时**: 60m
**依赖**: 组 B

---

### 组 D: ZIP 进度（串行，依赖组 C）
- T-008: stream_directory_as_zip 修改
- T-009: _serve_directory_zip 集成

**预计耗时**: 55m
**依赖**: 组 C

---

### 组 E: 测试验证（可并行）
- T-010: 单元测试
- T-011: 集成测试和手动验证

**预计耗时**: 30m
**依赖**: 组 D

---

## 执行计划

### 第 1 批（15m）: 基础设施
执行 T-001, T-002（并行）
完成后确认

### 第 2 批（15m）: 日志格式化
执行 T-003, T-004, T-005（并行）
完成后确认

### 第 3 批（60m）: 文件下载进度
执行 T-006, T-007（串行）
完成后确认

### 第 4 批（55m）: 目录下载进度
执行 T-008, T-009（串行）
完成后确认

### 第 5 批（30m）: 测试验证
执行 T-010, T-011（并行）
完成后确认

**总耗时**: 约 3 小时（考虑并行）

---

## 验收标准总结

### 功能验收
- [ ] 下载开始时显示日志（⬇️ emoji + IP + 文件名 + 大小）
- [ ] 下载过程中显示进度（字节/百分比）
- [ ] 下载完成时显示日志（✅ emoji + 总字节数 + 耗时）
- [ ] 下载中断时显示日志（⚠️ emoji + 已传输字节数）
- [ ] 下载失败时显示日志（❌ emoji + 错误原因）
- [ ] 支持并发下载的独立进度显示
- [ ] 目录 ZIP 下载显示进度

### 性能验收
- [ ] 日志输出不阻塞文件传输
- [ ] 日志频率控制（每 80KB 而非 8KB）
- [ ] 支持 10+ 并发连接

### 兼容性验收
- [ ] Linux 终端正常显示
- [ ] Windows PowerShell 正常显示
- [ ] UTF-8 编码正确显示 emoji 和中文

---

**维护者**: AI Team
**版本**: v1.0.0
**更新日期**: 2025-02-05
