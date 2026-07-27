---
feature: rust-rewrite
stage: tasks
complexity: complex
generated_by: spec-dev
generated_at: 2026-07-26T09:22:01+08:00
version: 2
status: awaiting-approval
planning: batch
execution: batch
---

# 任务拆分: Quick Share Rust 全量重写

## 1. 执行规则

### 1.1 TDD 铁律

每个实施任务必须按以下顺序进行：

1. **Red**：先添加能够表达验收行为、且当前明确失败的测试；
2. **Green**：实现让测试通过的最小生产代码；
3. **Refactor**：在测试保护下消除重复、收紧接口和完善错误语义；
4. **Review 1**：检查需求和设计符合性；
5. **Review 2**：检查代码质量、安全性、性能和跨平台风险；
6. **Verify**：运行该任务测试、受影响集成测试、格式化和 lint；
7. **Gate**：按批次向用户展示结果，确认后进入下一组。

禁止先实现后补测试。安全相关任务必须提供负向测试，不得只验证 happy path。

### 1.2 完成定义

单个任务只有同时满足以下条件才算完成：

- 任务列出的测试全部通过；
- `cargo fmt --check` 通过；
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 通过；
- 无新增未解释的 `unsafe`；
- 公共协议或配置变化已同步文档和 fixture；
- 错误信息不泄露密钥、token 或不必要的绝对路径；
- 复杂度为 complex，完成规范符合性与代码质量两阶段评审。

### 1.3 并行原则

- 同一批次中标记“可并行”的任务可在隔离分支/worktree 中实施；
- 修改同一 crate 公共接口的任务默认串行；
- TLS、传输状态机和恢复日志具有强依赖，禁止为追求并行而复制协议类型；
- 合并并行任务后必须运行 workspace 全量测试。

## 2. 里程碑与依赖图

```text
Batch 0  可行性 Spike（正式实现前 Go/No-Go）
├─ SP-001 身份协议：mTLS / 应用签名 / Noise XX
├─ SP-002 三平台 mDNS 与跨机器发现
├─ SP-003 浏览器 HTTPS 与移动扫码体验
├─ SP-004 分块恢复与崩溃一致性
└─ SP-005 剪贴板与单文件构建矩阵

                 ↓ Batch 0 必须单独确认

Batch 1  基础工程
└─ T-001 Workspace、质量门和 CI             ← SP-001..005

Batch 2  纯领域基础（可并行）
├─ T-002 QSP/1 协议类型与版本协商           ← T-001,SP-001
├─ T-003 配置、平台目录与安全身份存储        ← T-001,SP-001
├─ T-004 路径、manifest 与符号链接安全       ← T-001
└─ T-005 CLI 契约、快捷入口与退出码          ← T-001

Batch 3  本地能力（部分并行）
├─ T-006 mDNS 发现与 peer 去重              ← T-002,T-003,SP-002
├─ T-007 staging、块存储与恢复日志           ← T-002,T-004,SP-004
└─ T-008 剪贴板和平台适配                    ← T-003,SP-005

Batch 4  安全连接（串行主链）
├─ T-009 已选身份方案、SAS 与指纹固定         ← T-002,T-003,SP-001
└─ T-010 offer、确认和可信设备状态机          ← T-006,T-009

Batch 5  设备直传（串行主链）
├─ T-011 接收端分块上传与完整性校验           ← T-007,T-010
├─ T-012 发送端并发调度、进度和取消           ← T-011
└─ T-013 断点续传、幂等和失败重试             ← T-012

Batch 6  CLI 产品流程
├─ T-014 receive/devices 终端交互             ← T-010,T-013
├─ T-015 send 自动选择与安全回退               ← T-006,T-013
└─ T-016 文本与剪贴板传输                      ← T-008,T-013

Batch 7  传统 Web 模式
├─ T-017 Web catalog 与安全下载 API            ← T-004,T-007
├─ T-018 Web 上传、配额和冲突处理               ← T-017
├─ T-019 嵌入式 Web UI、ZIP、二维码和 TLS       ← T-017,T-018,SP-003
└─ T-020 自动 Web 回退端到端验证                ← T-015,T-019

Batch 8  分发与迁移
├─ T-021 安全自更新                            ← T-003,T-005
├─ T-022 安装脚本与跨平台 Release CI            ← T-001,T-021
└─ T-023 Python 功能迁移、切换和清理             ← T-020,T-022

Batch 9  发布验收
└─ T-024 跨平台、性能、安全和发布总验收          ← 全部
```

关键路径：

```text
SP-001..005 → Go/No-Go → T-001 → T-002/T-003/T-004
→ T-009 → T-010 → T-011 → T-012 → T-013
→ T-015 → T-020 → T-023 → T-024
```

## 3. 任务列表

### SP-001：身份协议安全原型与选型

**目标**：用最小可丢弃原型比较三种候选方案，选择能够满足首次配对、可信设备和加密直传的正式安全承载。

**候选方案**：

1. rustls 自签名 mTLS + 受限未知设备 verifier；
2. 标准服务端 TLS + 设备公钥应用层签名；
3. Noise XX + 自定义有界 framing。

**实验内容**：

- 两个独立进程首次握手、双端 SAS、接受一次和接受并信任；
- 公钥/证书固定后的正常重连；
- MITM 转发、设备名称复制、公钥替换、重放、过期 nonce；
- 未确认设备尝试上传数据；
- 库 API 复杂度、peer identity 可获取性、HTTP/2 兼容性和错误关闭行为；
- 原型中禁止实现自有密码算法，只使用成熟库。

**产物**：

- `docs/dev/rust-rewrite/spikes/SP-001-identity-protocol.md`；
- 一份替代 ADR，明确选择、拒绝方案、威胁模型和剩余风险；
- 可复现的进程级攻击测试；
- `spikes/identity/` 可丢弃代码。

**Go 条件**：至少一个方案能证明双方持有长期身份密钥、SAS transcript 一致、pinning 后替换攻击失败，且实现不依赖全局跳过验证。

**No-Go 条件**：只能通过“接受任意证书/公钥”工作，或无法可靠获取/绑定 peer identity。No-Go 时暂停并由用户决定引入配对码、预共享密钥或调整安全目标。

**跨机器协作**：原型本机进程测试通过后，可请用户在第二台机器运行一次性测试二进制，验证真实局域网、时钟差和平台 TLS 行为。

---

### SP-002：三平台 mDNS 与跨机器发现验证

**目标**：验证 mDNS 候选库和 `_quickshare._tcp.local.` 在真实机器上的广播、扫描、多网卡和失败表现。

**实验矩阵**：

- Linux ↔ Linux；
- Linux ↔ Windows；
- Linux ↔ macOS；
- 条件允许时 Windows ↔ macOS；
- IPv4、IPv6、多网卡、VPN、虚拟网卡、Wi-Fi 客户端隔离；
- 系统防火墙允许、明确拒绝和静默丢包；
- 设备休眠/唤醒、网络切换、重复广播和名称冲突。

**产物**：

- `docs/dev/rust-rewrite/spikes/SP-002-mdns-matrix.md`；
- `spikes/discovery/` 广播端和扫描端；
- 实测发现延迟、丢失率、接口列表和错误分类；
- 明确记录无法区分“零设备”和“静默丢包”的限制；
- mDNS 库选择或替代方案 ADR。

**Go 条件**：至少在目标三平台完成构建，并在可获得的两台真实机器上双向发现；明确错误不被映射成零结果；`--peer` 能绕过发现。

**受限环境处理**：当前 Agent 环境不能凭空模拟真实 Windows/macOS 局域网。缺少平台时标记为“待用户协作验证”，不得伪造通过。用户已表示可以提供第二台机器，执行到本任务时应给出精确、短小、可回滚的双机测试步骤。

---

### SP-003：传统 Web HTTPS 与移动扫码体验验证

**目标**：验证无中心服务条件下，自签名 HTTPS、用户证书、本地 CA 和显式 HTTP 的真实浏览器体验，形成产品决策。

**实验内容**：

- 临时 HTTPS 服务、随机 128 bit URL token 和二维码；
- Chrome、Edge、Safari，以及可获得的 Android/iOS 浏览器；
- 首次打开证书警告、继续访问步骤、刷新和二维码深链；
- `curl`、`wget` 的安全命令；
- 用户自有证书加载；
- 显式 `--allow-http` 的风险提示和局域网监听范围；
- 不安装 CA、不接入公网时是否存在可接受的流程。

**产物**：

- `docs/dev/rust-rewrite/spikes/SP-003-web-tls-ux.md`；
- 桌面/移动测试矩阵；
- 默认 HTTPS、默认 HTTP、本地 CA 或引入外部证书服务的产品 ADR；
- 清晰列出无法同时满足的约束。

**Go 条件**：用户明确接受一种默认策略及其安全/体验代价。技术测试通过但产品策略未确认，仍不得进入正式 Web 实现。

**跨机器协作**：可请用户用第二台电脑或手机扫描测试二维码，并反馈浏览器、系统、警告步骤和最终可达性。

---

### SP-004：分块恢复与随机崩溃一致性原型

**目标**：在不涉及网络协议的前提下证明“块数据 + 原子日志 + staging”能够从随机中断恢复且不产生伪完整文件。

**实验内容**：

- 4 MiB 块、乱序写入、重复块和摘要冲突；
- 在数据写、fsync、状态临时写、rename 和最终提交前后注入崩溃；
- 进程重启后扫描 journal 并补齐缺失块；
- 状态 JSON 截断、旧状态覆盖新数据、磁盘空间不足；
- 1 GiB 稀疏文件和大量小文件；
- 完成前最终路径不可见，完成后 BLAKE3 一致。

**产物**：

- `docs/dev/rust-rewrite/spikes/SP-004-resume-consistency.md`；
- `spikes/resume/` fault-injection harness；
- 数据/日志持久化顺序 ADR；
- 明确“单文件原子、目录任务不保证整体原子”的边界。

**Go 条件**：随机故障测试可重复通过；任何完成状态均对应已验证数据；损坏状态只能安全失败或重建，不能错误拼接文件。

---

### SP-005：剪贴板与单文件构建能力矩阵

**目标**：确定剪贴板库及其对 Windows、macOS、X11、Wayland、headless、glibc 和 musl 单文件产物的影响。

**实验内容**：

- 文本 read/write 和进程退出后的 clipboard 生命周期；
- Linux X11、Wayland 和无图形环境；
- Windows、macOS 编译与最小运行；
- glibc 与 musl 构建、动态库检查和产物大小；
- clipboard 不可用时 stdout/`--output` 降级；
- 比较 `arboard`、其他纯 Rust 候选和平台命令方案。

**产物**：

- `docs/dev/rust-rewrite/spikes/SP-005-clipboard-matrix.md`；
- 平台/构建能力矩阵；
- clipboard backend ADR；
- 明确哪些发布目标支持原生剪贴板，哪些只提供安全降级。

**Go 条件**：Windows/macOS 至少可构建，Linux 至少有一个常见桌面 backend 可用；headless 必须可运行核心程序并可靠降级，剪贴板不能成为文件传输的启动依赖。

---

### Batch 0 Go/No-Go 汇总

五项 Spike 完成后生成：

```text
docs/dev/rust-rewrite/spikes/batch-0-go-no-go.md
```

汇总必须列出：

- 证据和复现命令；
- 已验证平台与未验证平台；
- 获选方案和被拒绝方案；
- 需求需要调整的地方；
- 剩余 P0/P1 风险；
- 建议 Go、Conditional Go 或 No-Go。

在用户批准该汇总前，禁止执行 T-001。

### T-001：建立 Rust workspace、开发工具和 CI 基线

**状态**：✅ 已完成并通过两阶段评审（`T-001-review.md`）

**目标**：创建可构建、可测试、可发布单一二进制的 Rust 工程骨架，不改变现有 Python 发布入口。

**需求映射**：R-017、技术约束 4.1/4.2/4.7。

**Red**：

- 添加 CI/脚本契约测试，验证 workspace 必须包含预定 crates、edition、MSRV 和唯一发布 binary；
- 添加 CLI 冒烟测试，当前因二进制不存在而失败。

**Green**：

- 创建根 `Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`；
- 创建 `quick-share-cli/core/protocol/discovery/transfer/web/platform` crates；
- 主二进制实现最小 `--version` 和 `--help`；
- 配置 `cargo fmt`、Clippy、单元测试、依赖审计和三平台 CI 骨架；
- 保留 Python 构建和测试，避免提前切换发布。

**验证**：

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
pytest
```

**依赖**：SP-001 至 SP-005 全部完成，并通过 Batch 0 Go/No-Go 闸门。

**风险/评审重点**：crate 边界不能形成循环依赖；核心 crate 不得依赖 CLI/Web。

---

### T-002：定义 QSP/1 协议类型、限制与版本协商

**状态**：✅ 已完成并通过 Batch 2 两阶段评审（`Batch-2-review.md`）

**目标**：用纯数据类型固定 wire contract 和错误模型，为网络实现提供单一事实来源。

**需求映射**：R-002、R-004、R-006、R-014、技术约束 4.3。

**Red**：

- JSON fixture round-trip 测试；
- major 不兼容、minor 能力降级测试；
- 超长设备名、超大 entry 数、非法 chunk、未知安全字段测试；
- 稳定错误码序列化测试。

**Green**：

- 实现 `DeviceInfo`、`Capabilities`、`TransferOffer`、`ManifestEntry`、`OfferDecision`、`TransferStatus`、`ChunkDescriptor`、`ProtocolError`；
- 定义 major/minor 兼容规则和硬限制；
- 建立 `tests/fixtures/protocol/v1/` fixture。

**验证**：协议单测、fixture 兼容测试、任意 JSON 输入不 panic 的属性测试。

**依赖**：T-001、SP-001。

**风险/评审重点**：避免无界 Vec/String；错误响应不含本地敏感路径。

---

### T-003：实现配置、平台目录、设备身份和可信设备存储

**状态**：✅ 已完成并通过 Batch 2 两阶段评审（`Batch-2-review.md`）

**目标**：实现配置优先级、标准目录、原子状态写入和私钥权限。

**需求映射**：R-005、R-006、R-016。

**Red**：

- 默认配置、TOML、环境变量、CLI override 优先级测试；
- 配置损坏和未知危险字段测试；
- 首次生成身份、重复启动保持身份测试；
- 原子写入失败不破坏旧配置测试；
- Unix 私钥权限测试；Windows ACL 真机验证仅当前用户、SYSTEM 和必要管理员可访问，且不依赖宽松继承；
- 旧 `~/.quick-share/config.json` 仅迁移安全字段的测试。

**Green**：

- 实现 `AppDirs`、`ConfigLoader`、`IdentityStore`、`TrustedDeviceStore`；
- 按 ADR-009 生成持久化 Noise static key；若 Batch 0 未批准 ADR-009，则先回到设计阶段；
- 实现 `config.toml` 和 `trusted-devices.toml` 原子写入；
- 对私钥、token 和设备数据使用脱敏 Debug/Display。

**依赖**：T-001、SP-001。

**风险/评审重点**：身份重建会破坏 trust；私钥不能出现在日志、错误或 `config --show`。

---

### T-004：实现安全路径、manifest 与符号链接模型

**状态**：✅ 已完成并通过 Batch 2 两阶段评审（`Batch-2-review.md`）

**目标**：将用户输入转化为跨平台、安全且可恢复的传输 manifest。

**需求映射**：R-007、R-008、R-011、R-012。

**Red**：

- 文件、多文件、空目录、Unicode 和重复顶层名称测试；
- Unix `..`、绝对路径、NUL、Windows drive/UNC/保留名测试；
- 链接内/外逃逸、损坏链接、循环链接测试；
- 默认不跟随与 `--follow-links` 行为测试；
- Windows 管理员和非管理员/无 Developer Mode 两种符号链接创建与安全降级测试；
- 10,000 entries 资源边界测试；
- property test：任意接收路径不得逃离目标根。

**Green**：

- 实现 `RelativePath` 强类型；
- 实现 `ManifestBuilder` 和 `SourceSnapshot`；
- 实现跨平台名称合法性和冲突策略；
- 设计安全符号链接提交决策：相对且不逃逸才直接创建，危险链接要求确认或保存说明文件。

**依赖**：T-001。

**风险/评审重点**：TOCTOU；canonicalize 不得成为唯一防线；Windows 路径语义。

---

### T-005：实现 CLI 契约、`sc`/`rc` 快捷入口和退出码

**状态**：✅ 已完成并通过 Batch 2 两阶段评审（`Batch-2-review.md`）

**目标**：固定完整命令、快捷命令、帮助文案和错误行为，但暂不连接真实传输。

**需求映射**：R-001、R-003、R-009、R-016。

**Red**：

- `quick-share send/receive/serve/devices/config/update` parser 测试；
- argv[0] 为 `sc`、`rc` 时的注入行为测试；
- `--peer`/`--web` 冲突、`--text`/路径冲突测试；
- 配置错误和稳定退出码测试；
- 非交互终端确认策略测试。

**Green**：

- 使用 clap 实现命令树；
- 实现 `CommandIntent`，让 parser 与执行解耦；
- 实现统一 `AppError → ExitCode` 映射；
- 帮助输出明确自动模式和不会失败后回退 Web 的安全语义。

**依赖**：T-001。

**风险/评审重点**：不得提前把网络逻辑塞进 parser；`sc` 名称冲突由安装层处理。

---

### T-006：实现 mDNS 广播、扫描、去重与错误分类

**状态**：✅ 已完成并通过 Batch 3 两阶段评审（`Batch-3-review.md`）

**目标**：在多网卡环境发现 QSP/1 接收端，并区分“零结果”和“发现失败”。

**需求映射**：R-002、R-003、R-015。

**Red**：

- TXT 编解码、控制字符和长度限制测试；
- 自设备排除、重复地址、设备 ID 去重测试；
- 超时返回部分结果测试；
- multicast 权限失败、防火墙/接口错误与零结果分类测试；
- `enable_addr_auto()` 虚拟地址污染回归测试，生产实现必须显式选择接口 IP；
- mDNS 不跨链路时零结果、`--peer` 仍可直连的测试；
- fake discovery backend 的确定性异步测试。

**Green**：

- 实现 `Discovery` trait、mDNS backend 和 fake backend；
- 枚举并过滤 LAN 接口，按显式 IP 注册 `_quickshare._tcp.local.`；
- 实现 2 秒默认扫描、多接口结果合并和能力预筛选；
- 发现结果保持 unverified，直到 Noise 握手和 `INFO_RESPONSE` 验证。

**依赖**：T-002、T-003、SP-002。

**风险/评审重点**：企业网络、IPv6 zone、多网卡、虚拟接口；不得因单接口失败丢失其他成功结果。

---

### T-007：实现 staging、块存储、原子提交和恢复日志

**状态**：✅ 已完成并通过 Batch 3 两阶段评审（`Batch-3-review.md`）

**目标**：建立不依赖网络的接收存储引擎，保证中断时最终路径不出现伪完整文件。

**需求映射**：R-007、R-013、R-014、R-015。

**Red**：

- 空文件、稀疏大文件模拟、乱序/重复块测试；
- 块长度/摘要不匹配测试；
- 进程在状态写入不同阶段中断的 fault-injection 测试；
- 磁盘不足、权限不足、目标冲突测试；
- staging 与最终目录同文件系统测试；
- 完整性通过前最终文件不可见测试。

**Green**：

- 实现 `TransferStore`、`ChunkWriter`、`ResumeJournal`；
- 块写入采用 offset I/O 和有界 buffer；
- 实现 state 临时写、同步和 rename；
- 实现最终 BLAKE3、冲突决策和提交；
- 提供过期 staging 查询与清理 API，不在本任务自动删除可恢复内容。

**依赖**：T-002、T-004、SP-004。

**风险/评审重点**：fsync 语义、Windows rename、磁盘耗尽、状态与数据先后顺序。

---

### T-008：实现跨平台剪贴板和平台能力降级

**状态**：✅ 已完成并通过 Batch 3 两阶段评审（`Batch-3-review.md`）

**目标**：为文本传输提供可测试的剪贴板抽象和无 GUI 降级行为。

**需求映射**：R-009、R-015。

**Red**：

- clipboard read/write 成功测试；
- Linux 无 DISPLAY/WAYLAND、权限错误和格式不支持测试；
- fallback 到 stdout/文件测试；
- 收到文本绝不执行或自动打开的测试。

**Green**：

- 实现 `Clipboard` trait 和平台 backend；
- 集成候选 clipboard crate，关闭不必要 feature；
- 在 headless/不可用时返回结构化可恢复错误；
- 增加 `TextPayload` 大小上限和安全摘要。

**依赖**：T-003、SP-005。

**风险/评审重点**：Linux 动态依赖与单文件发布；剪贴板宿主生命周期；敏感文本日志。

---

### T-009：实现已选身份方案、SAS 和可信设备指纹固定

**状态**：✅ 已完成并通过 Batch 4 两阶段评审（`Batch-4-review.md`）

**目标**：建立 QSP/1 安全连接基础，并将未知连接与可信连接严格区分。

**需求映射**：R-005、R-006。

**Red**：

- 按 SP-001 选定方案验证双端身份互连和密钥持有证明；
- 未知设备只拥有 offer 权限测试；
- SAS 双端一致测试；
- 可信设备 pin 匹配、static key 替换、名称相同但 key 不同测试；
- 过期握手、截断/超长 frame、重放和 MITM 代理测试；
- 日志不含私钥或完整 token 测试。

**Green**：

- 按 ADR-009 重写实现 Noise XX，不复制 Spike；
- 实现有界 frame、peer static key、握手 transcript、指纹和 SAS；
- 实现 `PeerAuthContext { Unknown, Trusted, Changed }`；
- 将 `snow` adapter 集中到最小模块并写安全不变量；
- 锁定 Noise pattern，不允许运行时协商任意算法。

**依赖**：T-002、T-003、SP-001。

**风险/评审重点**：最高风险任务；`snow` 无正式审计，必须执行独立安全评审、RustSec 审计和 fuzz；不得自行发明密码原语或非标准 Noise pattern。

---

### T-010：实现 offer、终端确认和可信设备授权状态机

**状态**：✅ 已完成并通过 Batch 4 两阶段评审（`Batch-4-review.md`）

**目标**：陌生设备可以提交有限 offer，接收端确认后才获得 transfer 权限。

**需求映射**：R-004、R-005、R-006。

**Red**：

- accept once、accept and trust、reject、timeout 测试；
- 未确认上传、越权查询其他 transfer、重放 offer 测试；
- 可信 auto 和 confirm policy 测试；
- 指纹变化不自动信任测试；
- 并发 offer、队列上限和 rate limit 测试。

**Green**：

- 实现 `INFO_*`、`OFFER_CREATE` 和 `OFFER_STATUS` frame；
- 实现 `OfferManager` 和确认 channel；
- 生成终端所需设备、清单、总大小和 SAS view model；
- 接受并信任时原子更新 trust store；
- 授权 token 绑定 transfer、设备身份、权限和过期时间。

**依赖**：T-006、T-009。

**风险/评审重点**：TOCTOU、重放、确认后 manifest 变化、trusted auto 的资源攻击。

---

### T-011：实现接收端分块上传、校验与完成提交

**状态**：✅ 已完成并通过 Batch 5 两阶段评审（`Batch-5-review.md`）

**目标**：通过已认证 Noise transport 接收已授权 transfer 的数据块并安全落盘。

**需求映射**：R-006、R-007、R-013、R-014、R-015。

**Red**：

- Noise message type、frame length、offset、摘要和 payload 上限测试；
- 分块成功、重复相同块幂等、重复不同块拒绝测试；
- 未知/拒绝/过期设备上传测试；
- cancel、客户端断开、完成摘要错误测试；
- 同时接收任务和文件流 semaphore 测试。

**Green**：

- 实现 `TRANSFER_STATUS`、`CHUNK_DATA/ACK`、`TRANSFER_COMPLETE` 和 `TRANSFER_CANCEL`；
- 将 chunk frame 流入受限 writer，不聚合完整文件；
- 集成 `TransferStore`、取消 token 和进度事件；
- 完成后验证并提交文件、目录与安全链接。

**依赖**：T-007、T-010。

**风险/评审重点**：请求体内存、慢速客户端、资源耗尽、断开后后台任务泄漏。

---

### T-012：实现发送端并发调度、进度与取消

**状态**：✅ 已完成并通过 Batch 5 两阶段评审（`Batch-5-review.md`）

**目标**：发送方能够有界并发地上传文件/目录，并提供准确进度。

**需求映射**：R-007、R-013、R-015。

**Red**：

- 单/多/空文件和目录发送测试；
- 4 路并发上限与背压测试；
- 源文件在传输中改变测试；
- Ctrl+C 和对方取消测试；
- 进度 total/current/speed 事件单调性测试；
- 10,000 小文件不会创建无界任务测试。

**Green**：

- 实现 `TransferSender` 和有界 work queue；
- 分块读取、BLAKE3 和上传流；
- 实现进度事件聚合与终端渲染接口；
- 源快照在发送前后复核；
- 取消传播到文件读取、Noise frame writer 和重试队列。

**依赖**：T-011。

**风险/评审重点**：过多 open files、进度重复统计、CPU hashing 阻塞 async runtime。

---

### T-013：实现断点续传、幂等请求和有限重试

**状态**：✅ 已完成并通过 Batch 5 两阶段评审（`Batch-5-review.md`）

**目标**：网络短暂中断或进程重启后，仅重传缺失块并安全完成。

**需求映射**：R-014、R-015。

**Red**：

- 在每个 chunk 边界断开并恢复测试；
- sender/receiver 重启后恢复测试；
- transfer ID 被其他设备使用测试；
- source snapshot 变化拒绝恢复测试；
- 指数退避、抖动、最大 3 次和不可重试错误测试；
- 状态损坏时安全失败测试。

**Green**：

- 实现 status 缺失块协商；
- sender 根据 receiver bitmap 重建队列；
- 恢复请求绑定原设备 identity；
- 实现错误分类和有限自动重试；
- 增加 resume list/cancel/cleanup 的核心 API。

**依赖**：T-012。

**风险/评审重点**：错误拼接不同版本文件；无限重试；完成块状态丢失。

---

### T-014：实现 `receive`、`devices` 和接收终端体验

**状态**：✅ 已完成并通过 Batch 6 两阶段评审（`Batch-6-review.md`）

**目标**：交付可用的 `rc` 接收流程和可信设备管理命令。

**需求映射**：R-001、R-004、R-005、R-016。

**Red**：

- `rc` 启动信息、确认选项、超时和 Ctrl+C snapshot 测试；
- trusted auto/confirm 配置测试；
- devices list/rename/remove 测试；
- 非 TTY 下陌生请求默认不自动接受测试；
- 默认 Downloads 和 `--output` override 测试。

**Green**：

- 编排 discovery advertisement、Noise listener、offer prompt 和 transfer progress；
- 实现可信设备管理输出；
- 第一次 Ctrl+C graceful，第二次 force；
- 恢复日志在关停前持久化；
- Windows 检测 network profile 和 inbound 可达性；Public profile 被防火墙阻断时给出可操作提示，不静默修改防火墙。

**依赖**：T-010、T-013。

**风险/评审重点**：非交互环境安全默认；终端输出不能与 JSON/脚本模式混淆。

---

### T-015：实现 `send`、设备选择与安全自动回退

**状态**：✅ 已完成并通过 Batch 6 两阶段评审（`Batch-6-review.md`）

**目标**：交付 `sc` 的核心体验，并严格执行“只有零 peer 才回退 Web”。

**需求映射**：R-002、R-003、R-010、R-015。

**Red**：

- 零 peer 调用 Web adapter 测试；
- 发现错误不调用 Web、给出 `--web` 建议测试；
- 有 peer 后拒绝/超时/失败不回退测试；
- `--peer` 强制发送失败退出码 3/7 测试；
- `--web` 完全跳过 discovery 测试；
- 多 peer 选择、单 peer 明确确认测试。

**Green**：

- 实现 send orchestration；
- 接入 discovery、TLS 验证、offer、sender；
- Web 服务先通过 trait stub 接入，真实实现由后续任务完成；
- 输出当前模式和安全状态。

**依赖**：T-006、T-013。

**风险/评审重点**：回退条件必须用封闭 enum 表示，禁止用空 Vec 同时代表错误与无结果。

---

### T-016：实现文本与剪贴板端到端传输

**状态**：✅ 已完成并通过 Batch 6 两阶段评审（`Batch-6-review.md`）

**目标**：支持 `--text`、`--clipboard`、接收复制、stdout 和文件降级。

**需求映射**：R-009。

**Red**：

- 文本 round-trip、Unicode、空文本和大小上限测试；
- clipboard unavailable → stdout 测试；
- `--output` 保存文本测试；
- 控制字符显示和日志脱敏测试；
- 恶意 shell 文本不会执行测试。

**Green**：

- 将文本映射为 `ContentKind::Text`；
- 接收后按 clipboard → output/stdout 策略处理；
- 显示安全摘要，不自动打开链接或执行命令。

**依赖**：T-008、T-013。

**风险/评审重点**：终端转义注入；文本内容不得进入普通日志。

---

### T-017：实现 Web catalog、安全下载和目录 API

**状态**：✅ 已完成并通过 Batch 7 两阶段评审（`Batch-7-review.md`）

**目标**：使用 Axum 重建传统只读分享能力，路径映射不依赖用户可控绝对路径。

**需求映射**：R-010、R-011、R-012。

**Red**：

- 单/多文件、目录列表、单项下载和 Range 请求测试；
- `..`、双重编码、反斜杠、绝对路径、catalog ID 猜测测试；
- symlink root/内部/逃逸测试；
- token 缺失、过期、错误和次数限制测试；
- 大文件响应 body 不聚合内存测试。

**Green**：

- 实现 immutable `ShareCatalog`；
- 使用 entry ID 和安全 relative path 路由；
- 实现流式下载、Range、session/次数和 timeout；
- 设置 CSP、nosniff、缓存和内容处置头。

**依赖**：T-004、T-007。

**风险/评审重点**：路径遍历、Range 放大、Content-Disposition 注入、TOCTOU。

---

### T-018：实现 Web 上传、配额、认证和冲突处理

**状态**：✅ 已完成并通过 Batch 7 两阶段评审（`Batch-7-review.md`）

**目标**：重建 standalone/integrated 浏览器上传，保持有界流式解析和安全落盘。

**需求映射**：R-011、R-012、R-015。

**Red**：

- 单/多文件 multipart 流式上传测试；
- password/token 正确、错误和缺失测试；
- 路径遍历、恶意 filename、Windows 保留名测试；
- body/单文件/总配额超限测试；
- 冲突 rename、磁盘不足、客户端断开测试；
- 上传与下载 quota 语义测试。

**Green**：

- 实现 streaming multipart；
- 上传到同文件系统 staging，完成后提交；
- 实现访问 token、可选上传密码和 rate limit；
- 实现默认 rename 冲突策略和进度事件。

**依赖**：T-017。

**风险/评审重点**：multipart parser、资源耗尽、认证时序、临时文件清理。

---

### T-019：实现嵌入式 Web UI、ZIP、二维码和 Web TLS

**状态**：✅ 已完成并通过 Batch 7 两阶段评审（`Batch-7-review.md`）

**目标**：完成离线 Web 使用体验及 HTTPS 安全默认。

**需求映射**：R-010、R-011、R-012。

**Red**：

- 二进制内嵌静态资源测试；
- 桌面/移动基本响应式 DOM 快照或浏览器测试；
- Download All ZIP 结构、Unicode 和空目录测试；
- ZIP 客户端断开取消 worker 测试；
- 默认 HTTPS、用户证书、`--allow-http` 显式开关测试；
- QR/link/curl/wget 与实际监听地址一致测试。

**Green**：

- 实现无 CDN 的 HTML/CSS/JS UI；
- 实现目录展开、预览、下载、拖放上传和进度；
- blocking ZIP worker + 有界 channel；
- 默认自签名 HTTPS，支持用户证书；
- 生成 QR 和终端命令，明文模式显示醒目警告。

**依赖**：T-017、T-018、SP-003。

**风险/评审重点**：按 SP-003 获批策略实现 Web TLS；同时审查 XSS、ZIP bomb 风格资源耗尽和前端外网依赖。

---

### T-020：完成自动 Web 回退端到端流程

**状态**：✅ 已完成并通过 Batch 7 两阶段评审（`Batch-7-review.md`）

**目标**：将真实 Web server 接入 `sc` 自动模式并验证所有安全边界。

**需求映射**：R-003、R-010、R-011、R-012。

**Red**：

- 两进程测试：零 receiver → HTTPS Web；
- receiver 存在 → direct transfer，不启动 Web；
- receiver 拒绝/断网 → 非零退出且无 Web 监听；
- discovery error → 非零退出且提示 `--web`；
- timeout/quota/Ctrl+C 后端口释放测试。

**Green**：

- 将 `WebFallback` trait 绑定真实实现；
- 输出模式切换原因、监听接口、token URL、有效期和限制；
- 关停后清理服务器、worker 和临时状态。

**依赖**：T-015、T-019。

**风险/评审重点**：意外暴露；多网卡 URL；关停资源泄漏。

---

### T-021：实现签名验证、原子替换和失败回滚的自更新

**状态**：✅ 已完成并通过 Batch 8 两阶段评审（`Batch-8-review.md`）

**目标**：安全实现 `quick-share update`，不沿用 Python/pip 更新路径。

**需求映射**：R-018。

**Red**：

- semver 检查、`--check`、`--yes` 测试；
- 错平台资产、checksum 错误、签名错误测试；
- 下载中断、替换失败和回滚测试；
- Windows 当前 exe 锁定策略的抽象测试；
- 固定仓库/重定向攻击测试。

**Green**：

- 从固定 GitHub Releases 源读取版本元数据；
- 下载匹配 target 的资产、checksum 和签名；
- 验证后使用平台原子替换策略；
- 保留旧版本直到新版本启动/替换成功；
- 更新日志不泄露 token。

**依赖**：T-003、T-005。

**风险/评审重点**：供应链安全、Windows replace、签名根密钥轮换。

---

### T-022：重写安装脚本并建立跨平台 Release CI

**状态**：✅ 已完成并通过 Batch 8 两阶段评审（`Batch-8-review.md`）

**目标**：发布 Linux、macOS、Windows 预编译单文件及安全安装入口。

**需求映射**：R-001、R-017、R-018。

**Red**：

- Shell/Bats 与 PowerShell/Pester 安装契约测试；
- 架构/平台资产映射测试；
- checksum/signature 失败测试；
- 已存在 `sc`/`rc` 不覆盖测试；
- 安装中断保留旧版本测试。

**Green**：

- 更新 `install.sh`、`install.ps1` 下载 Rust 资产；
- 无冲突时创建 alias link/copy；
- Windows 防火墙规则是显式可选步骤，只允许必要程序和 Private profile；卸载时只移除自身规则；
- 使用 cargo-dist 或受控 GitHub Actions 构建矩阵；
- 生成 checksums、签名、SBOM 和 release notes；
- 对产物运行 `--version`、`--help` 和 loopback transfer 冒烟测试。

**依赖**：T-001、T-021。

**风险/评审重点**：安装脚本远程执行安全、目标三元组错误、macOS 签名/隔离属性。

---

### T-023：完成 Python 功能迁移、版本切换和旧实现清理

**状态**：✅ 已完成并通过 Batch 8 两阶段评审（`Batch-8-review.md`）

**目标**：在 Rust 达到验收后将其设为唯一实现，并安全移除 Python 运行时。

**需求映射**：整体迁移、R-011、R-017。

**Red**：

- 建立 Python→Rust 功能迁移矩阵；每项先有 Rust parity/新行为测试；
- README 示例和安装命令可执行测试；
- release 只包含 Rust 产物测试；
- 版本来源唯一性测试。

**Green**：

- 迁移旧测试覆盖：下载、上传、目录、预览、次数、超时、日志、网络、更新和路径安全；
- 更新 README、CHANGELOG、贡献和构建说明；
- 删除 `src/*.py`、Python tests、`setup.py`、`pyproject.toml` Python 配置、requirements 和 PyInstaller build；
- 将 Cargo package version 设为版本唯一来源；
- 若在删除前发布过渡版，严格同步 `src/__init__.py`、`setup.py`、`pyproject.toml` 和 CHANGELOG；
- 添加 v1→v2 配置迁移说明和回滚指引。

**依赖**：T-020、T-022。

**风险/评审重点**：这是不可逆切换任务；必须在单独提交中完成，且前一提交仍可构建 Python 版本。

---

### T-024：跨平台、性能、安全与发布总验收

**目标**：证明 Rust 2.0 候选版本满足需求文档，而不只是在开发机上工作。

**需求映射**：需求文档全部验收项。

**Red/验收基线**：

- 将需求第 3 节所有清单转为自动化或明确的手工验收 case；
- 在验收前运行并记录失败项，禁止默认勾选。

**执行内容**：

1. Linux、macOS、Windows 真机构建与安装；
2. x86_64 与 Apple Silicon 核心流程；
3. 两设备发现、陌生确认、信任、证书变化和拒绝；
4. 1 GiB、10 GiB、10,000 小文件、目录、文本、剪贴板和符号链接；
5. 网络中断、进程重启、磁盘不足、权限、冲突和完整性失败；
6. 零 peer Web 回退、HTTPS、显式 HTTP、上传、ZIP、次数和超时；
7. updater 成功、签名失败、替换失败和回滚；
8. 模糊测试、依赖漏洞/许可证审计、SBOM；
9. 峰值 RSS、CPU、吞吐和恢复耗时基准；
10. 从干净机器执行一键安装并验证 `quick-share`、`sc`、`rc`。

**完成条件**：

- 所有 P0/P1 验收通过；
- 无未处理高危安全问题；
- 三平台 CI 绿色；
- 性能不存在程序级明显瓶颈或无界资源增长；
- 生成 `rust-rewrite-acceptance-report.md`；
- 发布负责人明确批准 v2.0.0 tag。

**依赖**：SP-001 至 SP-005、T-001 至 T-023。

## 4. 批次闸门

默认执行模式为 `execution=batch`。每批完成并通过两阶段评审后，必须停在闸门：

| 批次 | 闸门重点 |
|---|---|
| Batch 0 | 五项 Spike 证据是否足以 Go；身份协议、发现边界、Web TLS 和平台降级是否被用户接受 |
| Batch 1 | workspace 和 CI 边界是否合理 |
| Batch 2 | 协议、配置、路径、CLI 契约是否稳定 |
| Batch 3 | discovery、staging、平台降级是否可测试 |
| Batch 4 | TLS/信任威胁模型和安全评审是否通过 |
| Batch 5 | 直传、完整性、恢复是否真正端到端可用 |
| Batch 6 | `sc`/`rc` 产品体验和回退规则是否符合需求 |
| Batch 7 | Web parity、安全与离线 UI 是否通过 |
| Batch 8 | 发布链、更新和 Python 切换是否可回滚 |
| Batch 9 | 总验收是否允许发布 v2.0.0 |

用户在某批次选择修改时，仅回退受影响任务和依赖任务，不重做已验证的独立任务。

## 5. 估算与范围控制

这是全量跨平台、安全敏感重写，不应按单个普通 feature 估算。建议按九个可交付批次推进：

| 里程碑 | 主要结果 | 相对工作量 |
|---|---|---:|
| M0（Batch 0） | 五项可行性结论和 Go/No-Go ADR | 8% |
| M1（Batch 1–3） | 可测试的 Rust 基础、发现与本地存储 | 22% |
| M2（Batch 4–5） | 安全设备直传与恢复 | 28% |
| M3（Batch 6） | 可用 CLI 产品流程 | 10% |
| M4（Batch 7） | 传统 Web parity | 18% |
| M5（Batch 8–9） | 发布迁移与总验收 | 14% |

范围控制规则：

- 不在首版追加桌面 GUI、移动客户端、互联网中继或 NAT 穿透；
- LocalSend 互操作作为独立后续功能；
- 包管理器发布不阻塞 v2.0.0；
- 当某个平台能力无法安全统一时，优先明确降级而不是伪造一致性；
- 身份协议、恢复格式和 Web 安全策略先由 Batch 0 形成 ADR；正式实现通过 Batch 4/7 闸门后，后续变更必须更新设计和协议版本。
