# 需求文档: Directory Sharing

> **生成方式**: 由 /dev:clarify 自动生成
> **生成时间**: 2026-01-14
> **访谈轮次**: 第 3 轮
> **细节级别**: standard
> **可直接用于**: /dev:spec-dev directory-sharing --skip-requirements

## 1. 介绍

扩展 Quick Share 以支持共享整个目录，允许接收者通过 Web 界面浏览目录结构、下载单个文件或下载整个目录的 zip 压缩包。

**目标用户**:
- **主要用户**: 开发者需要与团队成员共享项目文件夹进行协作或代码审查
- **次要用户**: 一般用户需要共享多个相关文件（照片、文档集合等）

**核心价值**:
- 消除手动打包目录为 zip 的繁琐步骤
- 提供灵活的访问方式（浏览单个文件 vs 下载全部）
- 保持 Quick Share 的安全性和简洁性

## 2. 需求与用户故事

### 需求 1: 自动检测文件类型
**用户故事:** As a user, I want to use the same command for both files and directories, so that I don't need to remember different commands.

#### 验收标准（可测试）
- **WHEN** user runs `quick-share ./some-directory`, **THEN** system **SHALL** detect it's a directory and enable directory-sharing mode.
- **WHEN** user runs `quick-share ./document.pdf`, **THEN** system **SHALL** detect it's a file and use existing file-sharing mode.
- **IF** path does not exist, **THEN** system **SHALL** display error message "Path not found: {path}".

### 需求 2: 目录沙箱安全机制
**用户故事:** As a user, I want to share only the specified directory, so that receivers cannot access my system files or parent directories.

#### 验收标准（可测试）
- **WHEN** receiver requests `/../../../etc/passwd`, **THEN** system **SHALL** reject with 403 Forbidden.
- **WHEN** receiver requests `/subdir/../../../secret.txt`, **THEN** system **SHALL** reject with 403 Forbidden.
- **WHEN** receiver requests URL-encoded traversal like `/%2e%2e/secret`, **THEN** system **SHALL** decode and reject.
- **WHEN** receiver requests a file within the shared directory like `/subdir/file.txt`, **THEN** system **SHALL** allow access.
- **WHEN** receiver requests a subdirectory within the shared directory, **THEN** system **SHALL** display its contents.

### 需求 3: Web 文件浏览界面
**用户故事:** As a receiver, I want to see a list of files in the shared directory, so that I can choose which files to download.

#### 验收标准（可测试）
- **WHEN** receiver visits the share URL, **THEN** system **SHALL** display an HTML page with file listing.
- **WHEN** displaying file listing, **THEN** system **SHALL** show filename, size, and last modified time for each file.
- **WHEN** receiver clicks on a file, **THEN** system **SHALL** download that specific file.
- **WHEN** receiver clicks on a subdirectory, **THEN** system **SHALL** navigate into it and show its contents.
- **WHEN** receiver is in a subdirectory, **THEN** system **SHALL** provide a "Go Up" or breadcrumb navigation option.
- **WHEN** displaying the file list, **THEN** system **SHALL** provide a "Download All as Zip" button.

### 需求 4: 整目录下载为 Zip
**用户故事:** As a receiver, I want to download the entire directory as a zip file, so that I can get all files in one operation.

#### 验收标准（可测试）
- **WHEN** receiver clicks "Download All as Zip", **THEN** system **SHALL** stream a zip file containing all files and subdirectories.
- **WHEN** generating zip file, **THEN** system **SHALL** preserve directory structure.
- **WHEN** generating zip file, **THEN** system **SHALL** use streaming compression to avoid loading entire directory into memory.
- **IF** zip generation fails, **THEN** system **SHALL** log the error and return 500 Internal Server Error.

### 需求 5: 按会话计数的限制
**用户故事:** As a user sharing a directory, I want download limits to count sessions instead of individual files, so that receivers can browse multiple files within a single session.

#### 验收标准（可测试）
- **WHEN** user sets `-n 3` for a directory share, **THEN** system **SHALL** allow 3 independent browsing sessions.
- **WHEN** a receiver opens the share URL, **THEN** system **SHALL** create a new session (using cookies or session tokens).
- **WHEN** the same receiver downloads multiple files, **THEN** system **SHALL** count it as 1 session.
- **WHEN** session limit is reached, **THEN** system **SHALL** stop the server and display "Download limit reached" message.
- **WHEN** timeout expires, **THEN** system **SHALL** stop the server regardless of session count.

### 需求 6: 向后兼容单文件共享
**用户故事:** As an existing user, I want single-file sharing to continue working exactly as before, so that my workflows are not disrupted.

#### 验收标准（可测试）
- **WHEN** user shares a single file, **THEN** system **SHALL** use the existing direct-download behavior.
- **WHEN** user shares a single file, **THEN** system **SHALL** count downloads per file (existing behavior).
- **WHEN** running existing tests for single-file sharing, **THEN** all tests **SHALL** pass without modification.

### 需求 7: 命令行选项适用于目录
**用户故事:** As a user, I want all existing CLI options (port, max downloads, timeout) to work with directories, so that I have consistent control.

#### 验收标准（可测试）
- **WHEN** user runs `quick-share ./mydir -p 9090`, **THEN** system **SHALL** serve directory on port 9090.
- **WHEN** user runs `quick-share ./mydir -n 5 -t 10m`, **THEN** system **SHALL** enforce 5 session limit and 10-minute timeout.
- **WHEN** user runs `quick-share --help`, **THEN** help text **SHALL** indicate the tool supports both files and directories.

## 3. 测试映射表

| 验收条目 | 测试层级 | 预期测试文件 | 预期函数/用例 |
|----------|----------|--------------|---------------|
| 需求1: 自动检测目录 | unit | tests/test_main.py | test_should_detect_directory_path |
| 需求1: 自动检测文件 | unit | tests/test_main.py | test_should_detect_file_path |
| 需求1: 路径不存在 | unit | tests/test_main.py | test_should_error_on_invalid_path |
| 需求2: 拒绝路径遍历 | unit | tests/test_security.py | test_should_reject_parent_traversal |
| 需求2: 拒绝URL编码遍历 | unit | tests/test_security.py | test_should_reject_encoded_traversal |
| 需求2: 允许子目录访问 | unit | tests/test_security.py | test_should_allow_subdirectory_access |
| 需求3: 显示文件列表 | integration | tests/test_integration.py | test_directory_listing_display |
| 需求3: 文件下载 | integration | tests/test_integration.py | test_download_single_file_from_directory |
| 需求3: 子目录导航 | integration | tests/test_integration.py | test_navigate_subdirectories |
| 需求4: Zip下载 | integration | tests/test_integration.py | test_download_directory_as_zip |
| 需求4: 保留目录结构 | unit | tests/test_server.py | test_zip_preserves_structure |
| 需求5: 会话计数 | unit | tests/test_server.py | test_session_based_download_counting |
| 需求5: 会话限制达到 | integration | tests/test_integration.py | test_server_stops_after_session_limit |
| 需求6: 单文件向后兼容 | integration | tests/test_integration.py | test_single_file_sharing_unchanged |
| 需求7: 端口选项 | integration | tests/test_integration.py | test_directory_share_custom_port |
| 需求7: 限制选项 | integration | tests/test_integration.py | test_directory_share_custom_limits |

## 4. 功能验收清单

> 从用户视角列出可感知的功能点，用于防止遗漏边缘场景。
> **规则**：实施阶段只能将 ☐ 改为 ✅，不得删除或修改功能描述。

| ID | 功能点 | 验收步骤 | 优先级 | 关联任务 | 通过 |
|----|--------|----------|--------|----------|------|
| F-001 | 共享目录显示文件列表 | 1. 运行 `quick-share ./testdir` 2. 访问生成的URL 3. 看到文件列表HTML页面 | P0 | 待分配 | ☐ |
| F-002 | 从目录下载单个文件 | 1. 在文件列表点击文件名 2. 文件开始下载 3. 文件内容正确 | P0 | 待分配 | ☐ |
| F-003 | 下载整个目录为Zip | 1. 点击"Download All as Zip"按钮 2. Zip文件开始下载 3. 解压后目录结构完整 | P0 | 待分配 | ☐ |
| F-004 | 拒绝路径遍历攻击 | 1. 尝试访问 `/../../../etc/passwd` 2. 返回403错误 3. 日志记录安全事件 | P0 | 待分配 | ☐ |
| F-005 | 子目录导航 | 1. 点击子目录名称 2. 进入子目录看到其文件列表 3. 点击"返回上级"回到父目录 | P0 | 待分配 | ☐ |
| F-006 | 会话计数正确 | 1. 设置 `-n 2` 2. 两个不同浏览器/会话访问 3. 第三个会话被拒绝 | P0 | 待分配 | ☐ |
| F-007 | 单文件共享不受影响 | 1. 运行 `quick-share file.txt` 2. 直接下载文件（非文件列表） 3. 行为与之前版本相同 | P0 | 待分配 | ☐ |
| F-008 | 边缘场景：空目录 | 1. 共享空目录 2. 显示"No files"消息 3. 仍显示"Download as Zip"选项（生成空zip） | P1 | 待分配 | ☐ |
| F-009 | 边缘场景：大文件列表 | 1. 共享包含1000+文件的目录 2. 页面加载合理时间内完成 3. 考虑分页或虚拟滚动 | P1 | 待分配 | ☐ |
| F-010 | 边缘场景：特殊文件名 | 1. 共享包含特殊字符文件名的目录（空格、Unicode等） 2. 文件名正确显示 3. 下载正常工作 | P1 | 待分配 | ☐ |
| F-011 | 边缘场景：符号链接 | 1. 共享包含符号链接的目录 2. 系统处理符号链接（跟随或忽略） 3. 不允许通过符号链接逃逸沙箱 | P1 | 待分配 | ☐ |
| F-012 | 错误处理：磁盘空间不足 | 1. 在zip生成时磁盘空间耗尽 2. 返回友好错误消息 3. 日志记录详细错误 | P2 | 待分配 | ☐ |
| F-013 | UI：文件大小显示 | 1. 文件列表显示人类可读的大小（KB、MB、GB） 2. 总目录大小显示在页面顶部 | P1 | 待分配 | ☐ |
| F-014 | UI：排序选项 | 1. 可按文件名、大小、修改时间排序 2. 默认按文件名字母顺序 | P2 | 待分配 | ☐ |
| F-015 | CLI：帮助文本更新 | 1. `quick-share --help` 显示更新的帮助 2. 明确说明支持文件和目录 | P1 | 待分配 | ☐ |

## 5. 技术约束与要求

### 5.1 技术栈
- **语言/框架**: Python 3.8+
- **Web服务器**: 继续使用现有的 HTTP server（`http.server.HTTPServer` 或类似）
- **依赖库**:
  - `zipfile` (标准库) 用于 zip 生成
  - 考虑使用模板引擎（如 `jinja2`）生成 HTML，或使用简单的字符串模板

### 5.2 集成点
- **现有模块**:
  - `src/security.py`: 扩展路径验证逻辑以支持目录沙箱
  - `src/server.py`: 扩展 HTTP 请求处理以支持目录列表和 zip 下载
  - `src/main.py`: 添加路径类型检测逻辑
  - `src/cli.py`: 更新帮助文本
- **新模块**（可选）:
  - `src/directory_handler.py`: 处理目录列表、zip 生成等逻辑

### 5.3 数据存储
- **会话管理**:
  - 使用 HTTP cookies 或简单的 session token 机制跟踪会话
  - 在内存中维护活跃会话计数器（简单的 dict 或 set）
  - 不需要持久化存储（轻量级设计）

### 5.4 性能要求
- **响应时间**:
  - 文件列表页面应在 2 秒内加载（对于 < 1000 个文件）
  - 单个文件下载立即开始流式传输
- **并发量**: 支持多个并发会话（限制由用户设置的 `-n` 参数决定）
- **数据量**:
  - 支持任意大小的目录（使用流式 zip 生成）
  - 文件列表页面考虑分页（如果文件数 > 500）

### 5.5 安全要求
- **路径遍历防护**:
  - 强化 `security.py` 的验证逻辑
  - 所有路径必须规范化（resolve symlinks, remove `..`）
  - 验证最终路径在共享目录内
- **符号链接处理**:
  - 检测符号链接指向的真实路径
  - 如果指向共享目录外，拒绝访问
- **输入验证**:
  - URL 解码后再进行路径验证
  - 拒绝多重编码攻击
- **日志记录**: 记录所有安全事件（路径遍历尝试、访问被拒等）

## 6. 排除项（明确不做）

- **上传功能**: 这是单向共享工具，不支持接收者上传文件
- **文件编辑/删除**: 接收者不能修改、删除或重命名文件（只读访问）
- **认证/密码保护**: 保持轻量级设计，任何有链接的人都可以访问
- **父目录访问**: 严格限制在共享目录及其子目录内，不允许向上导航到父目录
- **用户权限管理**: 不实现复杂的权限系统
- **文件搜索功能**: 不提供全文搜索或文件名搜索（接收者可使用浏览器的 Ctrl+F）
- **实时同步**: 共享的是目录的快照，不支持实时更新

## 7. 风险与挑战

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 路径遍历安全漏洞 | 高 | 1. 强化 security.py 验证逻辑<br>2. 添加全面的安全测试（包括 fuzzing）<br>3. 使用 `os.path.realpath()` 和 `os.path.commonpath()` 验证 |
| 大目录性能问题 | 中 | 1. 实现分页或虚拟滚动（文件数 > 500 时）<br>2. 异步加载文件列表<br>3. 添加性能测试 |
| Zip 生成内存消耗 | 中 | 1. 使用 `zipfile.ZipFile` 的流式写入模式<br>2. 逐个文件添加到 zip，不一次性加载全部<br>3. 考虑压缩级别权衡（速度 vs 大小） |
| 符号链接逃逸沙箱 | 高 | 1. 检测符号链接<br>2. 解析符号链接真实路径<br>3. 验证真实路径在共享目录内<br>4. 添加符号链接相关测试 |
| 向后兼容性破坏 | 中 | 1. 保持现有 API 和行为不变<br>2. 所有现有测试必须通过<br>3. 添加回归测试套件 |
| 会话管理复杂性 | 低 | 1. 使用简单的 cookie-based session<br>2. 内存中跟踪会话（不持久化）<br>3. 服务器重启时会话清空（可接受） |

## 8. 相关文档

- **简报版本**: docs/dev/directory-sharing/directory-sharing-brief.md
- **访谈记录**: 由 /dev:clarify 生成于 2026-01-14

## 9. 下一步行动

### 方式1：使用 spec-dev 继续（推荐）

在新会话中执行：
```bash
/dev:spec-dev directory-sharing --skip-requirements
```

这将：
1. ✅ 跳过阶段1（requirements 已完成）
2. 🎯 直接进入阶段2（技术设计）
3. 📋 然后阶段3（任务拆分）
4. 💻 最后 TDD 实施

### 方式2：手动进行设计

如果你想自己设计技术方案，可以：
1. 基于本 requirements 编写 design.md
2. 然后执行 `/dev:spec-dev directory-sharing --stage 3` 进行任务拆分

---

## 附录 A: 用户界面示意

### 文件列表页面（HTML）

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 Quick Share - my-project/
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

📁 Total: 15 files, 3.2 MB

[Download All as Zip]

Path: /my-project

┌──────────────────────────────────────┐
│ Name          │ Size    │ Modified    │
├──────────────────────────────────────┤
│ 📁 src/       │ -       │ 2026-01-10  │
│ 📁 tests/     │ -       │ 2026-01-12  │
│ 📄 README.md  │ 5.2 KB  │ 2026-01-14  │
│ 📄 setup.py   │ 1.1 KB  │ 2026-01-05  │
└──────────────────────────────────────┘

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Limits: 10 sessions or 5m timeout
Press Ctrl+C on the server to stop
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### 服务器日志输出

```
Sharing: /home/user/my-project (directory)
Total: 15 files, 3.2 MB
--------------------------------------------------
Browse Link: http://192.168.1.10:8000/

Command for receiver (browse):
  Open in browser: http://192.168.1.10:8000/

Command for receiver (download all):
  wget http://192.168.1.10:8000/?download=zip
  curl -O http://192.168.1.10:8000/?download=zip
--------------------------------------------------
Limits: 10 sessions or 5m timeout
Press Ctrl+C to stop sharing manually

[2026-01-14 10:30:15] Session 1 started: 192.168.1.20
[2026-01-14 10:30:22] Download: /README.md (5.2 KB) - Session 1
[2026-01-14 10:30:45] Session 2 started: 192.168.1.25
[2026-01-14 10:31:10] Security: Rejected path traversal - /../etc/passwd
```

## 附录 B: 安全验证伪代码

```python
def validate_directory_access(requested_path: str, shared_directory: str) -> bool:
    """
    Validate that requested path is within shared directory sandbox.

    Returns True if access is allowed, False otherwise.
    """
    # 1. URL decode the requested path
    decoded_path = urllib.parse.unquote(requested_path)

    # 2. Check for obvious traversal patterns
    if '..' in decoded_path:
        log_security_event("Path traversal attempt", decoded_path)
        return False

    # 3. Build full path
    full_path = os.path.join(shared_directory, decoded_path.lstrip('/'))

    # 4. Resolve to real path (handles symlinks, normalizes ..)
    try:
        real_path = os.path.realpath(full_path)
        real_shared = os.path.realpath(shared_directory)
    except Exception as e:
        log_security_event("Path resolution error", str(e))
        return False

    # 5. Verify real path is within shared directory
    if not real_path.startswith(real_shared):
        log_security_event("Path escapes sandbox", real_path)
        return False

    # 6. Verify path exists
    if not os.path.exists(real_path):
        return False

    return True
```
