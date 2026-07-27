# Batch 7 两阶段评审：T-017 至 T-020

> 日期：2026-07-27
> 范围：Web catalog/下载、浏览器上传、嵌入式 UI、ZIP、TLS、真实自动 Web 回退
> 结论：**通过，允许进入 Batch 8 人工闸门；Batch 7 范围内无未解决 P0/P1**

## 1. 评审范围

- `crates/quick-share-web/src/access.rs`
- `crates/quick-share-web/src/catalog.rs`
- `crates/quick-share-web/src/http.rs`
- `crates/quick-share-web/src/server.rs`
- `crates/quick-share-web/src/tls.rs`
- `crates/quick-share-web/src/upload.rs`
- `crates/quick-share-web/src/zip_stream.rs`
- `crates/quick-share-web/assets/index.html`
- `crates/quick-share-web/assets/app.css`
- `crates/quick-share-web/assets/app.js`
- `crates/quick-share-web/tests/catalog_download.rs`
- `crates/quick-share-web/tests/upload.rs`
- `crates/quick-share-web/tests/ui_zip_tls.rs`
- `crates/quick-share-web/tests/server_lifecycle.rs`
- `crates/quick-share-cli/src/app.rs`
- `crates/quick-share-cli/src/lib.rs`
- CLI contract/orchestration tests和真实进程 E2E

本批次将 Batch 6 的安全路由边界替换为真实传统 Web 服务。`serve`、`send --web` 和成功扫描后零兼容 receiver 的自动路径均使用同一个生产 Web adapter；发现或 direct 失败后仍禁止自动切换到 Web。

## 2. 阶段一：规范符合性评审

### 2.1 T-017：Web catalog、安全下载和目录 API

结论：**通过**。

- `ShareCatalog` 在服务启动时生成不可变快照，公开随机 128-bit entry ID、父子关系、安全显示名称、类型、大小和允许的 media type；API 不公开源绝对路径；
- catalog 最多 10,000 entries、64 层、单路径 4 KiB、显示 metadata 总计 8 MiB，避免目录枚举和 JSON 响应无界增长；
- symlink root 明确拒绝，目录内 symlink 和 `.quick-share-*` staging 跳过；每次打开下载前重新 canonicalize 并验证仍位于原 canonical root，阻止 catalog 后 symlink 替换逃逸；
- URL 路由只接受不可猜测 catalog ID，不把用户路径拼接到文件系统；`..`、反斜杠、绝对路径、双重编码和 path-shaped ID 不可能成为 source path；
- token 使用 128-bit CSPRNG，服务端只保存域分离 hash，并使用 constant-time compare；token、内存文本和绝对路径的 `Debug`/JSON 路径均已审查为脱敏；
- token expiry、最大下载 session、未知 ID、错误 token 和 exhausted quota 均 fail closed；catalog/preview 不消耗下载 quota，文件和 ZIP 下载在返回 body 前原子计数；
- 文件响应使用 Tokio 流式读取；单 Range 语义支持 `bytes=start-end`、suffix 和 open-ended，拒绝多 Range、越界和非法语法；64-bit size/offset 不截断；
- `Content-Disposition` 同时使用安全 ASCII fallback 和 RFC 5987 UTF-8 filename，控制字符不可进入 header；
- 文本 catalog 保持在最多 1 MiB 的有界内存，不创建明文临时文件；仅允许纯文本、JSON、CSV 和受限图片类型 inline preview，HTML/SVG/脚本类型强制下载；
- 全部响应设置 `Content-Security-Policy`、`X-Content-Type-Options: nosniff`、`Cache-Control: no-store`、`Referrer-Policy: no-referrer` 和 frame 限制。

### 2.2 T-018：Web 上传、配额、认证和冲突处理

结论：**通过**。

- Axum multipart field 按 chunk 流式写入，不在内存中聚合完整文件；单请求最多 128 files；
- `max_body_bytes`、单文件、请求总量、并发上传和滑动窗口请求数均有硬上限；所有长度运算使用 checked arithmetic；
- token 在读取 multipart body 前验证；有效 token 后先取得 rate/concurrency permit，再校验可选上传密码，因此错误密码尝试同样消耗速率预算；
- `serve --upload-password` 和 `QUICK_SHARE_UPLOAD_PASSWORD` 已接入生产服务；密码 1–1024 bytes、拒绝控制字符，内部只保存域分离 hash，所有 intent/options diagnostics 显示 `[REDACTED]`；环境变量方式可避免 shell history；
- 上传先写 output root 下同文件系统 `.quick-share-web-upload`，每个请求使用随机临时文件；只有完整 field 和全部 request 检查通过后才 rename commit；
- guard 在 multipart error、客户端断开、quota、认证失败和 commit 前错误时清理所有 staging；真实 final 在 commit 前不可见；
- filename 必须是 basename，拒绝空名称、`.`、`..`、正反斜杠、控制字符、Windows trailing dot/space 和保留设备名；
- 默认 `Rename` 冲突策略复用 core 的 portable conflict resolver，不静默覆盖；最终目标和 output root 在 commit 前重新做 symlink/canonical containment 校验；
- 上传成功/失败可发送有界进度事件；上传不消耗下载 quota；
- UI 仅在上传启用时显示上传面板，仅在服务确实配置上传密码时显示密码字段。

### 2.3 T-019：嵌入式 Web UI、ZIP、二维码和 Web TLS

结论：**通过**。

- HTML/CSS/JS 通过 `include_str!` 编入唯一 binary，不引用 CDN、Web font、analytics、远程脚本或外部网络资源；
- UI 支持目录展开/折叠、单文件下载、允许类型预览、Download All、拖放/文件选择上传及进度；所有 catalog 文本通过 `textContent`/DOM node 写入，无 `innerHTML`；
- CSS 提供 desktop/mobile breakpoint、focus-visible、reduced-motion、暗色模式和触摸尺寸；真实 Chrome desktop 与 iPhone 14 emulation 均通过，控制台 0 error，WCAG 2.1 AA 自动审计 0 issue；
- ZIP 在 blocking worker 中流式生成，通过容量固定的 channel 发送，最多 1 个并发 ZIP、8 个排队；body drop 会取消 worker；
- ZIP 保留 Unicode、空目录和层级；对 Windows 非法字符、trailing dot/space、保留名及冲突执行 portable 清理；打开每个 source 前继续执行 catalog containment 校验；
- HTTPS 是默认且无需磁盘证书文件：每次启动生成临时 self-signed ECDSA certificate/key；终端明确提示浏览器 warning、证书 SHA-256 和“不要安装为 CA”；
- 支持用户 PEM certificate chain/private key；缺失配对、错误 PEM 或与 `--allow-http` 混用均 fail closed；
- HTTP 只能由 `--allow-http`/显式配置构造 `ExplicitHttp`，运行时打印 token/content 可被网络路径观察的醒目警告；
- URL、QR、curl、wget、证书 SAN 和监听 socket 使用实际 bind IP/port，不输出虚构 endpoint；
- timeout、download quota、Ctrl+C 和显式 cancellation 均触发 graceful shutdown，停止 listener、ZIP worker/上传任务并释放端口。

### 2.4 T-020：真实自动 Web 回退端到端流程

结论：**通过**。

- `ProductionWeb` 实现 `WebShareAdapter`，替换 Batch 6 的 `DeferredWeb`；paths 和有界内存 text 均可启动真实 Web 分享；
- `--web` 完全跳过 discovery；自动路径仍只有 `ScanResult::is_definitive_empty()` 能调用 Web adapter；
- 发现 receiver 后只执行 encrypted direct，Web listener 不启动；receiver 拒绝、确认超时、连接/传输/完整性失败均返回稳定非零错误且无 Web；
- discovery error/partial empty 返回明确 `--web` 提示，绝不把错误或部分扫描解释为零 peer；
- 终端输出 traditional Web 模式、实际监听地址、token URL、expiry、下载限制、HTTPS/self-signed 或 explicit HTTP 安全状态；
- Linux 真实进程已覆盖零 receiver 自动 HTTPS、显式 Web、显式 HTTP、direct-only、拒绝不回退、文本内存分享和 Ctrl+C/配额关停；
- Windows 11 原生生产 binary 已覆盖自签名 HTTPS、Unicode 文件下载、quota 自动关停和端口释放。

## 3. 阶段二：代码质量与安全评审

结论：**通过；Batch 7 范围内无未解决 P0/P1。**

### 3.1 路径、身份与暴露面

- 文件系统访问只能从 immutable catalog entry 到 canonical source；HTTP path/query 不能成为 filesystem path；
- source root、entry ID、display path、Content-Disposition 和 ZIP path 使用不同类型/构造边界，不复用未经验证字符串；
- catalog API 使用专用 public DTO，不序列化内部 `CatalogEntry`，使未来新增内部字段也不会自动泄露 absolute path；
- token 是 URL bearer，页面无外部依赖且设置 no-referrer/no-store；上传密码仅作为 token 之外的可选第二道门，不代替高熵 token；
- HTTP 明文风险必须显式 opt-in；没有“TLS 初始化失败后自动降级 HTTP”的错误恢复路径。

### 3.2 资源、并发与关停

- catalog entries/display bytes、preview、Range、multipart body/file/total、request files、upload concurrency/rate、ZIP concurrency/queue/channel 全部有硬上限；
- 文件下载/上传和 ZIP 均流式运行；大文件测试确认响应 body 首帧产生时未聚合完整 body；
- blocking ZIP 与 async listener 分离，body drop cancellation 不依赖 worker 自然完成；
- token quota 使用原子 compare-exchange，多个并发下载不能超发；quota watch 在服务启动前已携带 exhausted 状态，避免零额度或启动竞态；
- shutdown task覆盖 timeout、quota 和 cancellation，所有生命周期测试最终重新 bind 同一端口成功。

### 3.3 前端与响应安全

- CSP 禁止外部脚本、object、frame 和 base URI；静态 JS 不使用 HTML string injection；
- 不允许把 HTML/SVG 等主动内容作为 inline preview；下载 header 使用 `nosniff` 和 attachment；
- UI 中错误内容来自固定状态或 HTTP status，不把服务器任意正文写入 DOM；
- QR 和命令由受控 scheme/IP/port/随机 hex token 生成，不含 peer filename 或上传正文。

### 3.4 评审中发现并已修复

| 级别 | 发现 | 修复与回归 |
|---|---|---|
| P1 | 初版 catalog 只在构建时确认路径；分享后文件被替换为 symlink 时可能产生 TOCTOU 逃逸 | 每次 download/preview/ZIP 打开前重新 canonicalize，并同时验证 canonical root、expected canonical path 与 containment；新增 post-catalog symlink swap Red 测试 |
| P1 | token quota shutdown 的初版 watch 初始状态可能错过“启动时已耗尽” | watch 初值直接来自原子 remaining 状态；zero/exhausted quota 生命周期测试确认 listener 自动停止并释放端口 |
| P1 | 可选上传密码最初只存在 Web service API，未接入 production CLI；并且错误密码发生在 rate permit 前，可被持有 token 的客户端高速尝试 | 增加 `serve --upload-password`/`QUICK_SHARE_UPLOAD_PASSWORD`、脱敏 `UploadPassword` 类型；rate/concurrency permit 前移到密码校验之前；集成测试确认两次错误密码 + 一次成功后下一请求被 429 限制 |
| P1 | 上传 staging/final 初版需要进一步阻止 output root 或 ancestor 在运行中被换成 symlink | service 保存 canonical output root，commit 前重验 parent/root containment；失败只清理 staging，不暴露 final |
| P2 | catalog response 最初直接 clone 内部 entry，虽然 absolute fields 标记 skip，但扩大未来误序列化风险和瞬时内存 | 改为专用 `CatalogEntryResponse`，只复制公开字段；内部 entry 不再派生 `Serialize` |
| P2 | 10,000 个合法但超长 display path 可形成过大的 catalog JSON | 新增 8 MiB display metadata 总上限和 checked accounting |
| P2 | 上传密码输入框在未配置密码时仍显示，容易让用户误以为必须填写 | catalog API 增加 `uploadPasswordRequired`，UI 默认隐藏并只在需要时显示/设为 required |
| P2 | ZIP 中 Unix 合法名称可能在 Windows 解压时成为非法/保留路径，且清理后可能碰撞 | 加入逐 component portable sanitization、保留名处理和 deterministic rename |

## 4. 验证结果

```text
cargo fmt --all -- --check                                           PASS
cargo clippy --workspace --all-targets --all-features -- -D warnings PASS
cargo test --workspace --all-targets --all-features                  PASS
  148 tests listed; 147 passed + 1 true-host mDNS ignored
cargo check --workspace --all-targets --all-features \
  --target x86_64-pc-windows-gnu                                    PASS
cargo check --manifest-path fuzz/Cargo.toml --locked                 PASS
cargo build --workspace --release                                   PASS
cargo audit                                                          PASS
  304 dependencies; no advisory
python -m pytest -q                                                  PASS (339 tests)
git diff --check                                                    PASS
```

Python 回归第一次执行出现旧 `test_download_progress` 的日志线程/capture 时序波动：338 passed、1 failed，完成日志在断言后立即出现。该 case 单独重跑通过，随后完整 339 tests 重跑通过；本批未修改 Python download progress 实现。

Linux 生产 Web E2E：

- 默认 self-signed HTTPS：UI/catalog、单 Range、Unicode 下载、浏览器上传和 Unicode/空目录 ZIP 全链通过；
- Chrome desktop 与 iPhone 14 emulation：页面可用，Console 0 error，WCAG 2.1 AA 自动报告 0 issue；
- definitive zero receiver：自动启动 HTTPS Web；
- receiver 存在：只执行 direct transfer，未启动 Web；
- receiver 拒绝：sender exit 4 且无 Web listener；
- explicit `--allow-http`：服务可用且输出醒目明文警告；
- `send --web --text`：正文仅在有界内存 catalog 中，恶意 shell 文本未执行；
- `serve --upload-password`：错误密码 401、正确密码 201、final payload 一致，日志不含密码；
- timeout、quota 和 Ctrl+C：listener/worker 清理并可重新绑定端口。

Windows 11 x86_64 真机：

- Web 专项测试通过；
- production CLI self-signed HTTPS listener 可用；
- Unicode 文件名通过 HTTPS 下载后 byte-for-byte 一致；
- max-download quota 达到后 server 自动退出并释放端口；
- Windows GNU workspace 全 target/all feature cross-check 通过；
- 当前 Public profile 对跨子网入站的系统限制保持不变，Quick Share 没有修改 firewall/profile。

Fuzz：

- `noise_records` 使用固定 `nightly-2026-07-01` / `cargo-fuzz 0.13.2`；
- 最终专项运行 1,483,097 inputs / 21 seconds，无 crash、panic、OOM 或 sanitizer finding；
- Web path/range/multipart/ZIP 安全边界由确定性和故障生命周期测试覆盖；更长期 parser fuzz 仍属于 T-024 总验收。

## 5. 已知限制与后续硬门

| 级别 | 项目 | 后续处理 |
|---|---|---|
| Release Blocker | `snow 0.10.0` 仍无可引用的正式第三方安全审计 | T-024 前取得审计证据/额外专家评审，或迁移到满足发布标准的实现 |
| 平台门 | macOS Intel/Apple Silicon 尚未真机验证 | T-022/T-024 完成 TLS、listener、mDNS、clipboard、staging、symlink、installer/updater 矩阵 |
| 预期 UX | 临时 self-signed HTTPS 会触发浏览器 warning | 保持明确 fingerprint/警告；不安装临时 CA；生产域名可使用用户证书 |
| 安全提示 | `--upload-password VALUE` 可能进入 shell history/process argv | CLI 同时支持 `QUICK_SHARE_UPLOAD_PASSWORD` 且 help 建议优先使用；所有应用日志和 Debug 均脱敏 |
| 环境限制 | 当前 Windows 机器为 Public profile，跨子网入站受 firewall policy 阻止 | 继续只读诊断和明确提示，不自动修改 profile/firewall |
| 发布前 UX | direct core 支持 durable resume，同进程 direct 支持 reconnect；跨 sender CLI invocation 的任务选择/恢复 UX 尚未完成 | T-024 前补齐或明确产品边界，不宣称任意进程重启后自动猜测旧任务 |
| 平台门 | Linux X11/Wayland 真桌面 clipboard、X11 ownership、Windows 无 Developer Mode symlink notice 尚待专项真机 | T-024 平台矩阵验证 |

## 6. 最终结论

T-017、T-018、T-019、T-020 已满足传统 Web catalog/下载、浏览器上传、离线 UI、ZIP、TLS 和真实自动回退目标。Web 默认 HTTPS，HTTP 无隐式降级；路径、token、上传、ZIP、响应 header、前端 DOM 和资源生命周期均有明确安全边界。两阶段评审发现的 TOCTOU、quota 启动竞态、生产上传密码缺口及密码限速问题均已修复并回归。

Batch 7 可以关闭并停在 Batch 8 人工闸门。此结论不是发布批准：安全 updater、跨平台 release/installer、Python 独立切换、macOS 真机和总验收仍由 T-021 至 T-024 交付。
