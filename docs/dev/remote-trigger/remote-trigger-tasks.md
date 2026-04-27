---
feature: remote-trigger
stage: tasks
version: 1
complexity: standard
generated_by: architect-planner
---

# 任务拆分: Remote Trigger

## 并行分组

```
Group 1 (并行, 无依赖)
├─ T-001: src/config.py — 配置文件管理
├─ T-002: src/file_dialog.py — 系统对话框
└─ T-008: src/logger.py — peer 日志格式

Group 2 (并行, 依赖 Group 1)
├─ T-003: src/peer_client.py — Peer HTTP 客户端 (依赖 T-001)
└─ T-005: src/cli.py — config 子命令 (依赖 T-001)

Group 3 (依赖 Group 1 + Group 2)
└─ T-004: src/peer_api.py — Peer API 端点 (依赖 T-001, T-002, T-003)

Group 4 (并行, 依赖 Group 3)
├─ T-006: src/server.py — 集成 peer API (依赖 T-004)
└─ T-007: src/main.py — 启动连接 peer (依赖 T-003, T-005)

Group 5 (依赖所有)
└─ T-009: 集成验证
```

## 任务列表

### T-001: src/config.py — 配置文件管理

**目标**: 实现 `~/.quick-share/config.json` 的读写管理。

**实现内容**:
- `get_config_path()` — 返回配置文件路径，确保目录存在
- `load_config()` — 读取 JSON 配置，不存在返回 `{}`
- `save_config(config)` — 原子写入（先写临时文件，再 os.replace）
- `get_peer_config()` — 返回 `config.get("peer")` 或 `None`
- `set_peer_config(address, secret)` — 写入 peer 段

**验证**: 手动测试读写、空文件、JSON 格式错误容错。

**依赖**: 无

---

### T-002: src/file_dialog.py — 系统对话框

**目标**: 跨平台弹出系统原生文件对话框。

**实现内容**:
- `_detect_tool()` — 按优先级探测可用工具: zenity > kdialog > osascript > powershell
- `open_file_dialog(multiple=True)` — 弹出文件多选对话框，返回路径列表
- `open_directory_dialog()` — 弹出目录选择对话框
- `open_save_dialog(default_name="")` — 弹保存路径对话框
- 降级: 无 GUI 工具时使用 `input()` 交互输入路径
- 子进程错误处理: 超时、用户取消、工具崩溃

**验证**: 在有 GUI 和无 GUI 环境下分别测试。

**依赖**: 无

---

### T-003: src/peer_client.py — Peer HTTP 客户端

**目标**: 封装与远程 quick-share 实例的 HTTP 通信。

**实现内容**:
- `PeerClient` 类
  - `__init__(self, address: str, secret: str)` — 解析 host:port，存 secret
  - `_post(path, body, timeout)` — 通用 POST，返回 JSON dict
  - `_get(path, timeout)` — 通用 GET
  - `say_hello(my_host, my_port)` — `POST /api/peer/hello`
  - `request_upload(timeout=120)` — `POST /api/peer/request-upload`
  - `request_download(files_info, timeout=120)` — `POST /api/peer/request-download`
  - `send_files(file_paths, target_url)` — `POST` multipart 文件到指定 URL
- 使用 `urllib.request` (标准库)
- 自动附加 `X-Peer-Secret` 头
- 连接错误处理（超时、拒绝连接、DNS 失败）

**验证**: 单元测试 mock HTTP 响应。

**依赖**: T-001 (需要 config 模块定义数据格式)

---

### T-004: src/peer_api.py — Peer API 端点

**目标**: 处理 `/api/peer/*` 请求的核心逻辑。

**实现内容**:
- `verify_peer_secret(handler, server_config)` — 验证 `X-Peer-Secret` 头
- `handle_peer_api(handler, server_config)` — 路由分发:
  - `/api/peer/hello` → `_handle_peer_hello()`
  - `/api/peer/request-upload` → `_handle_peer_request_upload()`
  - `/api/peer/request-download` → `_handle_peer_request_download()`
  - `/api/peer/receive` → `_handle_peer_receive()`
- `_handle_peer_hello()` — 记录 peer 地址，返回 `{"status":"ok","version":"..."}`
- `_handle_peer_request_upload()`:
  1. 验证密钥
  2. 打印日志 "收到来自 X 的上传请求"
  3. 调用 `open_file_dialog()` / `open_directory_dialog()`
  4. 如果取消: 返回 `{"status":"cancelled"}`
  5. 读取文件，使用 `PeerClient.send_files()` 发回请求方
  6. 返回 `{"status":"ok","files":[...]}`
- `_handle_peer_request_download()`:
  1. 验证密钥
  2. 打印日志
  3. 调用 `open_save_dialog()`
  4. 如果取消: 返回 `{"status":"cancelled"}`
  5. 返回 `{"status":"ok","path":"..."}`
- `_handle_peer_receive()`:
  1. 验证密钥
  2. 解析 multipart body
  3. 保存文件到指定路径
  4. 返回 `{"status":"ok","files":[...]}`

**关键细节**:
- `_handle_peer_request_upload` 需要知道请求方的地址来回传文件。请求方应在请求体中包含 `{"reply_host": "...", "reply_port": ...}`。
- `_handle_peer_receive` 支持 `save_path` 参数（来自 request-download 流程）和默认 `cwd` 参数（来自 request-upload 流程）。

**依赖**: T-001, T-002, T-003

---

### T-005: src/cli.py — config 子命令

**目标**: 添加 `quick-share config` 命令。

**实现内容**:
- 新增 `is_config_command()` 检测函数
- 新增 `handle_config_command()` 函数，解析并执行:
  - `quick-share config --peer 192.168.1.5:8080 --secret mykey` → 写入配置
  - `quick-share config --show` → 显示配置（密钥脱敏）
  - 无参数 → 显示使用帮助
- 在 `main()` 中，`update` 检查之后添加 `config` 检查
- 验证地址格式 (host:port)

**依赖**: T-001

---

### T-006: src/server.py — 集成 peer API

**目标**: 在现有 HTTP handler 中集成 `/api/peer/*` 路由。

**实现内容**:
- `MultiShareHandler.do_POST`: 在现有逻辑前，检测 `self.path.startswith("/api/peer/")`，如果是则路由到 `handle_peer_api()`
- `MultiShareServer.__init__`: 新增可选参数 `peer_secret=None`
- `MultiShareServer.start()`: 将 `peer_secret` 注入 `self.httpd`
- 同样添加到 `UploadHandler` (独立上传模式也支持 peer API)
- peer API 不占用 session 配额

**依赖**: T-004

---

### T-007: src/main.py — 启动连接 peer

**目标**: 启动时自动读取配置并连接 peer。

**实现内容**:
- 在服务器启动成功后，调用 `get_peer_config()`
- 如果配置存在:
  1. 创建 `PeerClient`
  2. 调用 `say_hello(local_ip, port)`
  3. 成功: 打印连接成功日志
  4. 失败: 打印警告（不中断服务）
- 将 `peer_secret` 传递给 `MultiShareServer` / `UploadServer`

**依赖**: T-003, T-005

---

### T-008: src/logger.py — peer 日志格式

**目标**: 添加 peer 相关事件的日志输出。

**实现内容**:
- `format_peer_connected(address)` — "✓ Connected to peer: 192.168.1.5:8080"
- `format_peer_unreachable(address, error)` — "⚠ Peer unreachable: ..."
- `format_peer_upload_request(peer_addr)` — "↗ Upload request from ..."
- `format_peer_download_request(peer_addr, file_count)` — "↘ Download request from ... (N files)"
- `format_peer_transfer_complete(direction, filename, size)` — "✓ Sent/Received: file (size)"
- `format_peer_auth_failed(peer_addr)` — "✗ Auth failed from ..."

**依赖**: 无

---

### T-009: 集成验证

**目标**: 端到端验证完整流程。

**验证内容**:
1. 配置 peer: `quick-share config --peer localhost:8090 --secret test`
2. 启动实例 A: `quick-share test.txt -p 8080` → 检查 peer 连接日志
3. 启动实例 B: `quick-share test.txt -p 8090` → 检查双向连接
4. curl 触发上传: `curl -X POST http://localhost:8080/api/peer/request-upload -H "X-Peer-Secret: test" -d '{"reply_host":"127.0.0.1","reply_port":8090}'`
5. curl 触发下载: `curl -X POST http://localhost:8080/api/peer/request-download -H "X-Peer-Secret: test" -d '{"files":[{"name":"a.txt","size":100}]}'`
6. 验证密钥错误时返回 403
7. 验证现有文件共享功能不受影响

**依赖**: T-001 到 T-008 全部
