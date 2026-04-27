---
feature: remote-trigger
stage: design
version: 1
---

# 技术设计: Remote Trigger

## 1. 架构概览

```
┌─ quick-share 实例 A ─────────────────────────────┐
│  main.py                                           │
│  ├─ 启动时读取 ~/.quick-share/config.json          │
│  ├─ 向 peer 发送 POST /api/peer/hello              │
│  ├─ 启动 HTTP 服务 (现有)                           │
│  └─ 新增 /api/peer/* 端点                          │
│                                                    │
│  server.py (MultiShareHandler)                     │
│  ├─ do_GET / do_POST (现有)                         │
│  └─ + peer API routing                             │
│                                                    │
│  peer_api.py (新)                                   │
│  ├─ handle_peer_hello()                            │
│  ├─ handle_peer_request_upload()                   │
│  ├─ handle_peer_request_download()                 │
│  └─ handle_peer_receive()                          │
│                                                    │
│  file_dialog.py (新)                                │
│  ├─ open_file_dialog() ──→ zenity/kdialog/osascript│
│  └─ open_save_dialog()                             │
│                                                    │
│  peer_client.py (新)                                │
│  ├─ PeerClient.say_hello()                         │
│  ├─ PeerClient.request_upload()                    │
│  ├─ PeerClient.request_download()                  │
│  └─ PeerClient.send_files()                        │
│                                                    │
│  config.py (新)                                     │
│  ├─ load_config() / save_config()                  │
│  └─ get_peer_config()                              │
└────────────────────────────────────────────────────┘
```

## 2. 新模块设计

### 2.1 config.py — 配置文件管理

```
~/.quick-share/config.json
{
  "peer": {
    "address": "192.168.1.5:8080",
    "secret": "shared-secret-key"
  }
}
```

函数:
- `get_config_path() -> Path` — 返回 `~/.quick-share/config.json`
- `load_config() -> dict` — 读取配置，不存在返回空 dict
- `save_config(config: dict)` — 写入配置
- `get_peer_config() -> dict | None` — 返回 peer 配置段或 None
- `set_peer_config(address: str, secret: str)` — 写入 peer 配置

### 2.2 file_dialog.py — 系统对话框

跨平台原生文件对话框，通过 subprocess 调用系统工具:

| 平台 | 工具 | 选择文件 | 选择目录 | 选择保存路径 |
|------|------|---------|---------|------------|
| Linux (GNOME/XFCE) | zenity | `zenity --file-selection --multiple` | `zenity --file-selection --directory` | `zenity --file-selection --save` |
| Linux (KDE) | kdialog | `kdialog --getopenfilename --multiple` | `kdialog --getexistingdirectory` | `kdialog --getsavefilename` |
| macOS | osascript | AppleScript file picker | AppleScript folder picker | AppleScript save dialog |
| Windows | PowerShell | `Add-Type -AssemblyName System.Windows.Forms` | 同上 | 同上 |

降级策略: 如无 GUI 工具，回退为终端 input() 交互输入路径。

函数:
- `_detect_tool() -> str` — 检测可用对话框工具 ("zenity"|"kdialog"|"osascript"|"powershell"|"stdio")
- `open_file_dialog(multiple: bool = True) -> List[str]` — 返回选中文件路径列表，取消返回空
- `open_directory_dialog() -> str | None` — 返回目录路径
- `open_save_dialog(default_name: str = "") -> str | None` — 返回保存路径

### 2.3 peer_client.py — Peer HTTP 客户端

纯标准库 (`http.client` / `urllib.request`) 实现的 HTTP 客户端。

```python
class PeerClient:
    def __init__(self, address: str, secret: str):
        # address: "host:port"
        self.host, self.port = address.split(":")
        self.port = int(self.port)
        self.secret = secret
    
    def _request(self, method, path, body=None, timeout=30) -> dict:
        """发送 HTTP 请求，返回 JSON 响应"""
    
    def say_hello(self, my_host: str, my_port: int) -> dict:
        """POST /api/peer/hello 告知对方我们的地址"""
    
    def request_upload(self, timeout=120) -> dict:
        """POST /api/peer/request-upload 请求对方选择文件上传"""
    
    def request_download(self, files: list[dict], timeout=120) -> dict:
        """POST /api/peer/request-download 请求对方准备接收文件"""
    
    def send_files(self, file_paths: list[str], target_url: str) -> dict:
        """POST /api/peer/receive 发送文件（multipart）"""
```

超时: request_upload/request_download 需要较长超时 (120s)，因为等待用户操作对话框。

### 2.4 peer_api.py — Peer API 端点

无状态函数，由 handler 调用。

```python
def handle_peer_api(handler, server_config: dict) -> bool:
    """路由 /api/peer/* 请求。返回 True 表示已处理。"""

def verify_peer_secret(handler, server_config: dict) -> bool:
    """验证 X-Peer-Secret 头"""

def handle_peer_hello(handler, server_config: dict):
    """处理注册: 记录 peer 信息，返回自身信息"""

def handle_peer_request_upload(handler, server_config: dict):
    """弹出文件选择对话框 → 读取文件 → POST 到请求方的 /api/peer/receive"""

def handle_peer_request_download(handler, server_config: dict):
    """弹出保存路径对话框 → 回复路径让请求方发送文件"""

def handle_peer_receive(handler, server_config: dict):
    """接收 multipart 文件并保存"""
```

## 3. 数据流

### 3.1 上传 (rz 模式): B 触发 A 上传

```
 B (请求方)                           A (响应方)
    │                                    │
    ├─ POST /api/peer/request-upload ──→│
    │  X-Peer-Secret: key                │
    │                                    ├─ 验证密钥
    │                                    ├─ 打印 "收到来自 B 的上传请求"
    │                                    ├─ 弹出文件选择对话框
    │                                    ├─ 用户选择文件
    │                                    ├─ 读取文件内容
    │                                    │
    │←─ A 调用 B 的 /api/peer/receive ──┤
    │   multipart 文件数据               │
    │                                    │
    ├─ 保存文件 ──────────────────────→│
    ├─ {"status":"ok","files":[...]} ──→│
    │                                    │
    │←─ {"status":"ok","files":[...]} ──┤
```

### 3.2 下载 (sz 模式): B 发送文件给 A

```
 B (请求方)                           A (响应方)
    │                                    │
    ├─ POST /api/peer/request-download─→│
    │  X-Peer-Secret: key                │
    │  {"files":[{name,size},...]}        │
    │                                    ├─ 验证密钥
    │                                    ├─ 打印 "收到来自 B 的文件传输"
    │                                    ├─ 弹出保存路径对话框
    │                                    ├─ 用户选择路径
    │                                    │
    │←─ {"status":"ok","path":"..."} ────┤
    │                                    │
    ├─ POST /api/peer/receive ─────────→│
    │  multipart 文件数据                │
    │                                    ├─ 保存到选定路径
    │←─ {"status":"ok","files":[...]} ───┤
```

## 4. 修改现有模块

### 4.1 cli.py

新增 `config` 子命令:

```bash
quick-share config --peer 192.168.1.5:8080 --secret mykey
quick-share config --show
```

实现方式: 参照 `is_update_command()` 模式，`is_config_command()` 检测并路由到 `handle_config_command()`。

新增参数:
- `--peer ADDR` — 远程实例地址（仅 config 子命令）
- `--secret KEY` — 共享密钥（仅 config 子命令）
- `--show` — 显示配置

### 4.2 server.py

**MultiShareHandler 修改:**
- `do_POST`: 在现有上传处理之前，先检查 `/api/peer/` 前缀并路由到 peer_api

**MultiShareServer 修改:**
- 新增属性: `peer_secret`, `peer_config`
- `start()`: 注入 peer 配置到 httpd

### 4.3 main.py

启动流程修改（在服务器启动后）:

```python
# 读取 peer 配置
peer_config = get_peer_config()
if peer_config:
    client = PeerClient(peer_config["address"], peer_config["secret"])
    try:
        result = client.say_hello(local_ip, port)
        print(f"✓ Connected to peer: {peer_config['address']}")
    except Exception as e:
        print(f"⚠ Peer unreachable: {peer_config['address']} - {e}")
```

### 4.4 logger.py

新增消息格式化函数:
- `format_peer_connected(address)` — `"✓ Connected to peer: 192.168.1.5:8080"`
- `format_peer_unreachable(address, error)` — `"⚠ Peer unreachable: ..."`
- `format_peer_upload_request(peer_addr)` — `"↗ Received upload request from ..."`
- `format_peer_download_request(peer_addr, file_count)` — `"↘ Received download request from ... (N files)"`
- `format_peer_transfer_complete(direction, filename, size)` — 传输完成

## 5. 安全设计

- 所有 `/api/peer/*` 端点验证 `X-Peer-Secret` 头
- 密钥明码存储在配置文件，权限由操作系统保护 (文件权限 600)
- 如未配置密钥，拒绝所有 peer 请求
- 不记录完整密钥到日志
- 文件保存沿用 `upload_handler.py` 的路径遍历防护

## 6. 错误处理

| 场景 | 行为 |
|------|------|
| Peer 不可达 (启动时) | 打印警告，继续本地服务 |
| Peer 密钥错误 | 返回 403，记录警告 |
| 对话框取消 | 返回 `{"status": "cancelled"}` |
| 对话框中无 GUI | 降级为终端输入 |
| 传输中断 | 返回错误，两端清理 |
| 路径遍历攻击 | 返回 403（复用现有验证）|

## 7. 文件清单

| 文件 | 类型 | 说明 |
|------|------|------|
| `src/config.py` | 新增 | 配置文件管理 |
| `src/file_dialog.py` | 新增 | 系统对话框 |
| `src/peer_client.py` | 新增 | Peer HTTP 客户端 |
| `src/peer_api.py` | 新增 | Peer API 端点 |
| `src/cli.py` | 修改 | 新增 config 子命令 |
| `src/server.py` | 修改 | 集成 peer API 路由 |
| `src/main.py` | 修改 | 启动时连接 peer |
| `src/logger.py` | 修改 | 新增日志格式 |
