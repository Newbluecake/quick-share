# 需求文档: Quick Share

> **生成方式**: 由 /dev:clarify 自动生成
> **生成时间**: 2026-01-12
> **访谈轮次**: 第 3 轮
> **细节级别**: standard
> **可直接用于**: /dev:spec-dev quick-share --skip-requirements

## 1. 介绍

Quick Share 是一个命令行工具，用于在局域网内快速启动HTTP服务分享单个文件，自动生成适合 curl/wget 的下载链接。

**目标用户**:
- 主要用户：开发者、技术人员、运维工程师
- 使用场景：服务器间文件传输、临时分享日志文件、快速分享构建产物、局域网内设备间文件共享

**核心价值**:
- 解决在没有 SCP、SFTP 权限或配置复杂时的临时文件分享需求
- 提供一键启动的极简体验，无需手动配置 IP、端口、路径映射
- 自动生成复制即用的下载命令，减少人工输入错误

## 2. 需求与用户故事

### 需求 1: 自动检测局域网 IP 地址
**用户故事:** As a 用户, I want 工具自动检测我的局域网IP, so that 我不需要手动查看 ifconfig/ipconfig 输出。

#### 验收标准（可测试）
- **WHEN** 用户执行 `quick-share <file>`, **THEN** 系统 **SHALL** 自动检测本机的局域网 IPv4 地址。
- **IF** 存在多个网络接口, **THEN** 系统 **SHALL** 按优先级选择: 以太网 > WiFi > 其他。
- **IF** 检测到的IP是 127.0.0.1 或虚拟网卡IP (docker/vmware), **THEN** 系统 **SHALL** 跳过该IP。
- **IF** 无法检测到有效的局域网IP, **THEN** 系统 **SHALL** 显示错误信息并退出。

### 需求 2: 自动选择可用端口
**用户故事:** As a 用户, I want 工具自动选择一个可用端口, so that 我不需要担心端口冲突。

#### 验收标准（可测试）
- **WHEN** 系统启动HTTP服务, **THEN** 系统 **SHALL** 默认尝试使用端口 8000。
- **IF** 端口 8000 被占用, **THEN** 系统 **SHALL** 自动尝试 8001, 8002, ..., 8099。
- **IF** 8000-8099 所有端口都被占用, **THEN** 系统 **SHALL** 显示错误信息并退出。
- **IF** 用户通过参数指定了端口 (如 `-p 9000`), **THEN** 系统 **SHALL** 仅尝试该端口，失败则退出。

### 需求 3: 路径安全限制
**用户故事:** As a 用户, I want 确保只有指定文件可被下载, so that 我的系统安全不会被侵犯。

#### 验收标准（可测试）
- **WHEN** 系统接收HTTP请求, **THEN** 系统 **SHALL** 仅允许访问指定文件的basename路径。
- **IF** 请求路径包含 `..` 或绝对路径, **THEN** 系统 **SHALL** 返回 403 Forbidden。
- **IF** 请求路径不等于文件basename, **THEN** 系统 **SHALL** 返回 404 Not Found。
- **WHEN** 正确的文件路径被请求, **THEN** 系统 **SHALL** 返回文件内容并设置正确的 Content-Type 和 Content-Disposition 头。

### 需求 4: 生成下载链接和命令
**用户故事:** As a 用户, I want 工具显示可直接使用的下载链接和命令, so that 我可以快速复制粘贴给其他人。

#### 验收标准（可测试）
- **WHEN** HTTP服务成功启动, **THEN** 系统 **SHALL** 在控制台输出以下信息:
  - 文件名
  - 文件大小（人类可读格式，如 1.2MB）
  - HTTP URL (如 `http://192.168.1.100:8000/file.zip`)
  - curl 下载命令 (如 `curl http://192.168.1.100:8000/file.zip -O`)
  - wget 下载命令 (如 `wget http://192.168.1.100:8000/file.zip`)
- **IF** 服务正在运行, **THEN** 输出 **SHALL** 包含提示信息（如"服务运行中，按 Ctrl+C 停止"）

### 需求 5: 下载次数限制
**用户故事:** As a 用户, I want 设置下载次数上限, so that 文件不会被无限次下载。

#### 验收标准（可测试）
- **WHEN** 用户未指定下载次数限制, **THEN** 系统 **SHALL** 默认允许 10 次下载。
- **IF** 用户通过参数指定下载次数 (如 `-n 3`), **THEN** 系统 **SHALL** 在达到 3 次下载后自动停止服务。
- **WHEN** 每次成功下载, **THEN** 系统 **SHALL** 在控制台显示下载进度和剩余次数。
- **IF** 达到下载次数上限, **THEN** 系统 **SHALL** 显示提示信息并优雅退出。

### 需求 6: 运行时长限制
**用户故事:** As a 用户, I want 设置服务运行时长上限, so that 服务不会一直占用端口。

#### 验收标准（可测试）
- **WHEN** 用户未指定运行时长, **THEN** 系统 **SHALL** 默认在 5 分钟后自动停止。
- **IF** 用户通过参数指定运行时长 (如 `-t 10m`), **THEN** 系统 **SHALL** 在 10 分钟后自动停止。
- **WHEN** 达到时长上限, **THEN** 系统 **SHALL** 显示提示信息并优雅退出。
- **IF** 用户通过参数禁用时长限制 (如 `-t 0`), **THEN** 服务 **SHALL** 持续运行直到手动停止或达到下载次数限制。

### 需求 7: 下载日志显示
**用户故事:** As a 用户, I want 看到谁在什么时间下载了文件, so that 我可以追踪文件的传播情况。

#### 验收标准（可测试）
- **WHEN** 有客户端发起下载请求, **THEN** 系统 **SHALL** 在控制台显示：
  - 时间戳 (如 `[2026-01-12 14:30:25]`)
  - 客户端IP地址
  - 请求方法和路径
  - 响应状态码
  - 已下载次数 / 总次数限制 (如 `1/10`)
- **IF** 下载成功完成, **THEN** 日志 **SHALL** 包含 "Download completed" 提示。

### 需求 8: 命令行参数支持
**用户故事:** As a 用户, I want 通过命令行参数自定义行为, so that 我可以灵活控制服务配置。

#### 验收标准（可测试）
- **WHEN** 用户执行 `quick-share --help`, **THEN** 系统 **SHALL** 显示帮助信息，包括所有可用参数。
- **IF** 用户提供 `-p/--port <port>` 参数, **THEN** 系统 **SHALL** 使用指定端口。
- **IF** 用户提供 `-n/--max-downloads <count>` 参数, **THEN** 系统 **SHALL** 设置下载次数上限。
- **IF** 用户提供 `-t/--timeout <duration>` 参数, **THEN** 系统 **SHALL** 设置运行时长（支持 `5m`, `1h` 等格式）。
- **IF** 文件路径参数缺失或无效, **THEN** 系统 **SHALL** 显示错误信息和使用示例。

## 3. 测试映射表

| 验收条目 | 测试层级 | 预期测试文件 | 预期函数/用例 |
|----------|----------|--------------|---------------|
| 需求1: 自动检测局域网IP | unit | tests/test_network.* | test_detect_local_ip, test_skip_loopback, test_skip_virtual_interface |
| 需求1: 多网卡优先级 | unit | tests/test_network.* | test_interface_priority |
| 需求1: 无有效IP时报错 | unit | tests/test_network.* | test_no_valid_ip_error |
| 需求2: 默认端口8000 | unit | tests/test_server.* | test_default_port_8000 |
| 需求2: 端口冲突自动递增 | unit | tests/test_server.* | test_port_auto_increment |
| 需求2: 所有端口占用时报错 | unit | tests/test_server.* | test_all_ports_occupied_error |
| 需求2: 自定义端口 | unit | tests/test_server.* | test_custom_port |
| 需求3: 仅允许basename路径 | unit | tests/test_security.* | test_only_basename_allowed |
| 需求3: 路径遍历攻击防护 | unit | tests/test_security.* | test_path_traversal_blocked |
| 需求3: 不存在路径返回404 | unit | tests/test_security.* | test_invalid_path_404 |
| 需求3: 正确路径返回文件 | integration | tests/integration/test_download.* | test_successful_download |
| 需求4: 输出格式验证 | unit | tests/test_output.* | test_output_format_contains_url_and_commands |
| 需求5: 默认下载次数10 | unit | tests/test_limits.* | test_default_max_downloads_10 |
| 需求5: 自定义下载次数 | unit | tests/test_limits.* | test_custom_max_downloads |
| 需求5: 达到次数上限自动退出 | integration | tests/integration/test_limits.* | test_auto_shutdown_on_max_downloads |
| 需求6: 默认运行时长5分钟 | unit | tests/test_limits.* | test_default_timeout_5min |
| 需求6: 自定义运行时长 | unit | tests/test_limits.* | test_custom_timeout |
| 需求6: 达到时长上限自动退出 | integration | tests/integration/test_limits.* | test_auto_shutdown_on_timeout |
| 需求6: 禁用时长限制 | unit | tests/test_limits.* | test_disable_timeout |
| 需求7: 下载日志格式 | unit | tests/test_logging.* | test_log_format |
| 需求8: 帮助信息显示 | unit | tests/test_cli.* | test_help_message |
| 需求8: 命令行参数解析 | unit | tests/test_cli.* | test_parse_args |

## 4. 功能验收清单

> 从用户视角列出可感知的功能点，用于防止遗漏边缘场景。
> **规则**：实施阶段只能将 ☐ 改为 ✅，不得删除或修改功能描述。

| ID | 功能点 | 验收步骤 | 优先级 | 关联任务 | 通过 |
|----|--------|----------|--------|----------|------|
| F-001 | 自动检测局域网IP | 1. 执行 `quick-share test.txt` 2. 验证输出包含 192.168.x.x 或 10.x.x.x 格式的IP | P0 | 待分配 | ☐ |
| F-002 | 多网卡环境正确选择IP | 1. 在有以太网和WiFi的设备上执行 2. 验证优先选择以太网IP | P0 | 待分配 | ☐ |
| F-003 | 自动选择可用端口 | 1. 执行 `quick-share test.txt` 2. 验证默认使用8000端口 3. 占用8000后再次执行，验证使用8001 | P0 | 待分配 | ☐ |
| F-004 | 仅允许下载指定文件 | 1. 启动服务分享 test.txt 2. 尝试访问 /etc/passwd 3. 验证返回403或404 | P0 | 待分配 | ☐ |
| F-005 | 路径遍历攻击防护 | 1. 启动服务 2. 尝试访问 `/../etc/passwd` 3. 验证被拦截 | P0 | 待分配 | ☐ |
| F-006 | 显示curl下载命令 | 1. 执行 `quick-share test.txt` 2. 验证输出包含 `curl http://...` 命令 | P0 | 待分配 | ☐ |
| F-007 | 显示wget下载命令 | 1. 执行 `quick-share test.txt` 2. 验证输出包含 `wget http://...` 命令 | P0 | 待分配 | ☐ |
| F-008 | curl命令可直接使用 | 1. 复制输出的curl命令 2. 在另一台机器执行 3. 验证文件下载成功 | P0 | 待分配 | ☐ |
| F-009 | 浏览器可访问下载 | 1. 复制HTTP URL 2. 在浏览器打开 3. 验证触发文件下载 | P0 | 待分配 | ☐ |
| F-010 | 默认下载次数10次 | 1. 执行 `quick-share test.txt` 2. 下载10次 3. 验证第11次时服务已停止 | P1 | 待分配 | ☐ |
| F-011 | 自定义下载次数 | 1. 执行 `quick-share test.txt -n 3` 2. 下载3次 3. 验证服务自动停止 | P1 | 待分配 | ☐ |
| F-012 | 默认5分钟自动停止 | 1. 执行 `quick-share test.txt` 2. 等待5分钟 3. 验证服务自动停止 | P1 | 待分配 | ☐ |
| F-013 | 自定义运行时长 | 1. 执行 `quick-share test.txt -t 1m` 2. 等待1分钟 3. 验证服务自动停止 | P1 | 待分配 | ☐ |
| F-014 | 禁用时长限制 | 1. 执行 `quick-share test.txt -t 0` 2. 验证服务持续运行（需手动停止） | P1 | 待分配 | ☐ |
| F-015 | 显示下载日志 | 1. 启动服务 2. 另一台机器下载文件 3. 验证控制台显示时间戳、IP、状态码 | P1 | 待分配 | ☐ |
| F-016 | 显示下载进度 | 1. 启动服务 `-n 3` 2. 每次下载后验证显示 "1/3", "2/3", "3/3" | P1 | 待分配 | ☐ |
| F-017 | 自定义端口 | 1. 执行 `quick-share test.txt -p 9000` 2. 验证使用9000端口 | P1 | 待分配 | ☐ |
| F-018 | 帮助信息显示 | 1. 执行 `quick-share --help` 2. 验证显示所有可用参数 | P1 | 待分配 | ☐ |
| F-019 | 边缘场景：文件不存在 | 1. 执行 `quick-share nonexistent.txt` 2. 验证显示错误信息并退出 | P1 | 待分配 | ☐ |
| F-020 | 边缘场景：文件路径为目录 | 1. 执行 `quick-share /home/user/` 2. 验证显示错误信息（不支持目录） | P1 | 待分配 | ☐ |
| F-021 | 边缘场景：无参数执行 | 1. 执行 `quick-share` 2. 验证显示使用说明 | P2 | 待分配 | ☐ |
| F-022 | 边缘场景：Ctrl+C优雅退出 | 1. 启动服务 2. 按 Ctrl+C 3. 验证服务优雅关闭 | P2 | 待分配 | ☐ |
| F-023 | 显示文件大小（人类可读） | 1. 分享一个 1.5MB 文件 2. 验证输出显示 "1.5 MB" 而非字节数 | P2 | 待分配 | ☐ |

**字段说明**：
- **ID**：功能点唯一标识（F-XXX）
- **功能点**：从用户视角描述可感知的功能
- **验收步骤**：可测试的操作步骤和预期结果
- **优先级**：P0（核心功能）/ P1（重要功能）/ P2（一般功能）
- **关联任务**：对应 tasks.md 中的任务ID（T-XXX），由 spec-dev 填充
- **通过**：☐ 未完成 / ✅ 已完成（实施阶段只能更新此字段）

## 5. 技术约束与要求

### 5.1 技术栈
- **语言/框架**: Python 3.8+ 或 Go 1.18+（实施时选择，推荐Python便于快速开发）
- **依赖库**:
  - HTTP服务器：Python `http.server` 或 Go `net/http`
  - 网络接口检测：Python `netifaces`/`psutil` 或 Go `net` 标准库
  - 命令行参数解析：Python `argparse` 或 Go `flag`/`cobra`
- **打包方式**: 单一可执行文件（PyInstaller 或 Go build）

### 5.2 集成点
- **外部 API**: 无需调用外部服务
- **内部模块**:
  - `network`: 网络接口检测、IP地址获取
  - `server`: HTTP服务器启动、请求处理
  - `security`: 路径验证、安全检查
  - `cli`: 命令行参数解析
  - `logger`: 日志格式化输出

### 5.3 数据存储
- **数据库**: 无需数据库
- **缓存**: 无需缓存
- **临时数据**: 内存中维护下载次数计数器、启动时间戳

### 5.4 性能要求
- **响应时间**: 小文件(<10MB)下载延迟 <100ms
- **并发量**: 支持至少 5 个并发下载请求
- **数据量**: 支持分享最大 10GB 的单个文件（受内存限制，流式传输）
- **启动时间**: 从执行命令到显示URL <1秒

### 5.5 安全要求
- **认证**: 无（信任局域网环境）
- **授权**: 仅允许下载指定文件，拒绝其他路径访问
- **数据保护**:
  - 严格验证请求路径，防止路径遍历攻击
  - 不记录敏感信息到日志
  - 不在错误信息中暴露系统路径

## 6. 排除项（明确不做）

- **目录分享**：不支持分享整个目录的文件列表，仅支持单个文件
- **多文件分享**：不支持一次性分享多个文件（可作为 future feature）
- **二维码生成**：暂不实现二维码功能（可作为 future feature）
- **外网访问**：不支持公网访问，不涉及 NAT 穿透、动态DNS
- **文件上传**：只读服务器，不支持文件上传
- **身份认证**：不实现用户名/密码或Token认证
- **HTTPS支持**：仅支持 HTTP（局域网环境安全性可接受）
- **断点续传**：不支持 HTTP Range 请求（可作为 future feature）
- **压缩传输**：不支持 gzip 压缩（可作为 future feature）

## 7. 风险与挑战

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 多网卡环境下IP选择错误 | 中 | 实现网卡优先级逻辑，排除虚拟网卡，提供手动指定IP参数（future feature） |
| 端口范围（8000-8099）全部被占用 | 低 | 显示清晰错误信息，提示用户使用 `-p` 参数指定端口 |
| 大文件（>1GB）下载时内存占用过高 | 中 | 使用流式传输而非一次性加载到内存 |
| 文件在服务运行期间被删除 | 低 | 启动时验证文件存在，请求时返回404并记录错误日志 |
| 跨平台兼容性（Windows/Linux/macOS） | 高 | 使用跨平台库，在多平台进行集成测试 |
| 防火墙阻止局域网访问 | 中 | 在文档中提供防火墙配置说明，启动时提示用户检查防火墙 |

## 8. 相关文档

- **简报版本**: quick-share/docs/dev/quick-share/quick-share-brief.md
- **访谈记录**: 由 /dev:clarify 生成于 2026-01-12

## 9. 下一步行动

### 方式1：使用 spec-dev 继续（推荐）

在新会话中执行：
```bash
/dev:spec-dev quick-share --skip-requirements
```

这将：
1. ✅ 跳过阶段1（requirements 已完成）
2. 🎯 直接进入阶段2（技术设计）
   - 选择编程语言（Python vs Go）
   - 确定HTTP服务器实现方案
   - 设计模块结构和接口
3. 📋 然后阶段3（任务拆分）
   - 拆分为可并行的开发任务
   - 生成 tasks.md
4. 💻 最后 TDD 实施
   - 测试驱动开发
   - 逐个任务实现

### 方式2：手动进行设计

如果你想自己设计技术方案，可以：
1. 基于本 requirements 编写 design.md
2. 然后执行 `/dev:spec-dev quick-share --stage 3` 进行任务拆分
