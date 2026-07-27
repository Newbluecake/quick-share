# Rust Rewrite - SDD Context

> **功能标识**: rust-rewrite
> **复杂度**: complex
> **工作流模式**: normal（planning=batch, execution=batch）
> **最后更新**: 2026-07-27T12:19:13+08:00

## 配置参数

```yaml
feature_name: rust-rewrite
planning: batch
execution: batch
complexity: complex
requirements_source: docs/dev/rust-rewrite/rust-rewrite-requirements.md
worktree: existing
branch: devagent/rust-rewrite-86f9r
with_review: false
parallel: auto
```

## 当前状态

| 阶段 | 状态 | 产物 |
|---|---|---|
| 阶段 0：Worktree | 已完成 | 当前已位于隔离 worktree |
| 阶段 1：需求分析 | 已完成、已获用户确认 | `rust-rewrite-requirements.md` |
| 阶段 2：技术设计 | v2 已完成并获批准 | `rust-rewrite-design.md` |
| 阶段 3：任务拆分 | 已完成并获批准 | `rust-rewrite-tasks.md` |
| 阶段 4：环境准备 | 已完成 | 正式 Rust workspace、toolchain、CI |
| 阶段 5：代码实施 | Batch 8 已完成，停在 Batch 9 人工闸门 | T-021 至 T-023、`Batch-8-review.md` |

## Planning 结论摘要

- 采用 Cargo workspace 分层，最终只发布一个 `quick-share` 二进制；
- `sc`、`rc` 是同一二进制的快捷入口；
- 已选择 mDNS/DNS-SD 作为链路本地发现，跨子网使用 `--peer`；
- 已按 SP-001 选择 Noise XX，并保留专项安全评审/审计发布硬门；
- 首次信任必须使用短验证码或一次性配对码，可信设备固定身份指纹；
- 采用 4 MiB 默认分块、BLAKE3、接收日志和 staging 实现恢复；
- 传统 Web 已选择策略 D：默认自签名 HTTPS，HTTP 只能显式 `--allow-http`；
- Rust 与 Python 先并存，最终验收后在单独任务中切换和删除 Python；
- 正式实施仍为 24 个 TDD 任务、9 个批次；前置新增 5 个 Spike 和 1 个 Batch 0 Go/No-Go 闸门；
- 用户可以提供第二台机器，SP-001/SP-002/SP-003 到达真实双机验证时应提供精确测试步骤。

## 关键风险

1. 建议 Noise XX，但 snow 尚无正式安全审计，首次信任仍要求比较 SAS；
2. 自签名 HTTPS 已选策略 D，保留浏览器警告体验成本；
3. mDNS 在企业网络、防火墙和多网卡环境中的可用性；
4. 断点续传状态一致性和磁盘故障恢复；
5. 跨平台符号链接、剪贴板和可执行文件替换差异；
6. 全量重写的范围和最终 Python 切换风险。

## Git 状态说明

Planning 开始时没有检测到其他用户代码变更。当前 worktree 包含 Batch 0 spikes、Batch 1 workspace/CI、Batch 2 领域基础，以及 Batch 3 的发现、staging/恢复和剪贴板实现；Python 实现仍保留且 339 项基线测试通过。当前改动尚未提交。

## 已完成批次

Batch 0 已按 Conditional Go 获批，验证结果：

- SP-001：Linux↔Linux、Windows↔Linux Noise XX/SAS/pinning/encrypted transport 通过；建议 Conditional Go，安全审计为发布硬门；
- SP-002：Linux 双机和 Windows 原生 mDNS 通过；显式 IP 解决自动地址污染；跨子网不发现属预期；
- SP-003：用户批准策略 D；Windows 原生 server 可用，但 Public profile 入站被防火墙阻断，需产品诊断；
- SP-004：Linux `kill -9`、Windows `Stop-Process -Force` 后 256 MiB 恢复通过，含 Windows Unicode 路径；
- SP-005：Linux headless 降级、Windows 原生及 Unicode clipboard 通过；X11/Wayland/macOS 待验证。

验证记录：

- 现有 Python 基线：`339 passed`；
- Spike Rust tests：`18 passed`;
- Spike Clippy：`-D warnings` 通过；
- Windows GNU target：workspace `cargo check` 和 release build 通过；
- Debian 13 x86_64、Windows 11 x86_64 均完成原生运行；
- 所有临时监听服务已停止。

## Batch 2 结果

- T-002：QSP/1 类型、版本协商、硬限制、有界 decoder 和 v1 fixture；
- T-003：标准目录、配置优先级、原子写入、X25519 身份、完整 key pinning；
- T-004：portable `RelativePath`、bounded manifest、source snapshot、symlink 安全分类/降级；
- T-005：完整命令树、`CommandIntent`、`sc`/`rc` argv[0]、稳定退出码；
- 两阶段评审：`Batch-2-review.md`；
- Rust workspace：46 tests、Clippy `-D warnings`、Windows cross-check 全部通过；
- Windows 11 真机：ACL、原子替换、并发 identity、native symlink、CLI shortcuts 通过；
- Python 回归：339 passed。

## Batch 3 结果

- T-006：严格 TXT、显式 LAN IP、mDNS advertise/scan、peer 去重、自身/版本过滤、partial/error 分类和 fake backend；
- T-007：sender-bound staging、offset chunk I/O、BLAKE3、原子 journal/commit、故障恢复、冲突重判和过期查询；
- T-008：native clipboard adapter、1 MiB text payload、安全摘要、stdout/文件 no-clobber 降级；
- 两阶段评审：`Batch-3-review.md`；
- Rust workspace：69 tests listed（68 默认通过、1 个 mDNS true-host 测试手动在 Linux/Windows 均通过）；
- Clippy、Windows cross-check、RustSec audit 和 Linux release build 全部通过；
- Windows 11 真机：mDNS、staging/恢复/atomic overwrite、Unicode clipboard 通过；
- Python 回归：339 passed。

## Batch 4 结果

- T-009：固定 `Noise_XX_25519_ChaChaPoly_BLAKE2s`、XX static key 证明、6 位 transcript SAS、完整 key pinning、`Unknown/Trusted/Changed`、有界 handshake/record/logical frame 和 replay/rekey/MITM 测试；
- T-010：严格 INFO/OFFER control codec、不可变 offer snapshot、确认 channel、trusted auto/confirm、SAS-gated AcceptAndTrust、owner/transfer/identity/permission/expiry-bound authorization、rate/queue/replay bounds；
- 两阶段及独立安全评审：`Batch-4-review.md`；评审中发现的 control semantic validation、自动 timeout、trusted display name 和 message type binding 问题均已修复；
- Rust workspace：91 tests listed（90 默认通过、1 个 mDNS true-host ignored）；Clippy all-features、Windows cross-check、release build、RustSec audit 全部通过；
- `cargo-fuzz` 使用 `nightly-2026-07-01` 覆盖 record/segment/control parser，多轮累计超过 300 万 inputs 无 crash；CI 增加 10 秒 fuzz smoke；
- Windows 11 真机：Noise 12/12、auth 2/2、offer 7/7；Python 回归重跑 339/339；
- `snow 0.10.0` 无正式第三方安全审计，继续作为 T-024 发布硬门；T-011 network driver 必须强制 socket wall-clock timeout，并在任何 Noise/framing error 后关闭 session。

## Batch 5 结果

- T-011：严格 `TRANSFER_STATUS`、`CHUNK_DATA/ACK`、`TRANSFER_COMPLETE/CANCEL`，每帧授权校验，流式 staging、完整摘要、安全文件/目录/链接提交，接收取消 token、进度和资源上限；
- T-012：固定 worker、有界队列、最多 4 路文件工作、blocking 分块读取/哈希、源快照复核、单调进度和双向取消；
- T-013：缺失块位图、sender/receiver 重启恢复、sender/manifest binding、稳定 request ID 幂等、最多 3 次抖动指数退避，以及 resume list/cancel/cleanup；
- 新增 async Noise network driver：handshake/record 均有 wall-clock timeout 和取消，任何 I/O、Noise 或 framing 错误立即 shutdown 并永久关闭 session；
- capability-bound final commit 阻止符号链接祖先竞态；Windows 目录同步和相对 symlink 分隔符差异已通过真机修复；
- 两阶段评审：`Batch-5-review.md`，无未解决 P0/P1；
- Rust workspace：114 tests listed（113 默认通过、1 个 true-host mDNS ignored）；Clippy all-features、Windows cross-check、Linux release 和 RustSec audit 全部通过；
- Windows 11 真机 Batch 5 专项：Noise/network 14、receiver 4、resume 3、sender 9、store 12，共 42/42；
- fuzz：1,489,089 inputs，无 crash、panic、OOM 或 sanitizer finding；Python 回归 339/339。

## Batch 6 结果

- T-014：真实 Noise listener、mDNS advertisement、offer prompt、进度、可信设备命令、非 TTY 安全默认、两阶段 Ctrl+C 和 Windows Public-profile 只读诊断；
- T-015：封闭 `SendRoute`、唯一 definitive-empty Web 回退、真实 TCP + Noise connector/server dispatcher、INFO/能力协商、稳定远端错误分类和断线重连；
- T-016：literal/clipboard 文本直传、Unicode/空文本、无明文源临时文件、authenticated completed text、clipboard → stdout 与 explicit no-clobber file 输出；
- 两阶段评审：`Batch-6-review.md`；评审中发现并修复 direct prompt/offer manager confirmation timeout 双源和远端错误折叠问题，无未解决 Batch 6 P0/P1；
- Rust workspace：129 tests listed（128 默认通过、1 个 true-host mDNS ignored）；Clippy all-features、Windows GNU cross-check、Linux release、RustSec audit 和 fuzz locked check 全部通过；
- Windows 11 真机：direct 3/3、orchestration 7/7、devices 2/2、platform 2/2；production CLI loopback Unicode 文件/文本均通过；
- Windows → Linux production CLI 文件和 native clipboard Unicode 文本通过，payload byte-for-byte 一致，恶意 shell 文本未执行；
- Windows Public profile 诊断文案通过，运行前后 firewall rule hash 一致；Linux → Windows 跨子网入站被当前 firewall policy 阻止，Windows loopback listener/协议已通过，按产品约束不自动修改系统规则；
- fuzz：823,291 inputs / 21 秒，无 crash、panic、OOM 或 sanitizer finding；Python 回归 339/339。

## Batch 7 结果

- T-017：immutable Web catalog、随机 entry ID、目录 API、token expiry/quota、单 Range、流式下载、受限 preview、Content-Disposition 和 symlink/TOCTOU containment；
- T-018：流式 multipart、body/file/total/concurrency/rate 硬上限、同文件系统 staging、rename conflict、可选上传密码、密码错误限速和断开清理；
- T-019：无 CDN 的 embedded responsive UI、目录/预览/上传进度、bounded/cancellable ZIP、默认临时 self-signed HTTPS、用户 PEM、显式 HTTP、QR/curl/wget；
- T-020：`serve`、`send --web` 和 definitive-zero 自动路径接入真实生产 Web server；receiver 存在、拒绝、direct failure、discovery error 均保持严格不回退；
- 两阶段评审：`Batch-7-review.md`；评审中发现并修复 catalog TOCTOU、quota 启动竞态、production 上传密码缺口、密码错误未计入 rate limit、内部 catalog DTO 暴露面和 ZIP portability 问题；Batch 7 无未解决 P0/P1；
- Rust workspace：148 tests listed（147 默认通过、1 个 true-host mDNS ignored）；Clippy all-features、Windows GNU cross-check、Linux release、RustSec audit、fuzz locked check 和 `git diff --check` 全部通过；
- Linux production Web E2E：HTTPS UI/catalog/Range/download/upload/Unicode ZIP、零 receiver 自动回退、explicit HTTP、direct-only、receiver 拒绝无 Web、memory text 和 Ctrl+C/quota cleanup 均通过；
- Chrome desktop/iPhone 14 emulation：console 0 error，WCAG 2.1 AA 自动审计 0 issue；
- Windows 11 真机：Web 专项、production self-signed HTTPS、Unicode 下载和 quota shutdown 通过；Public-profile firewall 限制保持只读诊断；
- fuzz：1,483,097 inputs / 21 seconds，无 crash、panic、OOM 或 sanitizer finding；Python 最终回归 339/339。

## Batch 8 结果

- T-021：固定GitHub仓库/redirect allowlist、bounded streaming download、strict SHA256SUMS、pinned Ed25519 signature、semver/check/yes、candidate preflight、durable backup、Windows-aware self replace和rollback；
- T-022：Rust raw assets/archives、Linux GNU+musl、macOS Intel+Apple Silicon、Windows MSVC matrix、released-binary loopback smoke、SPDX SBOM、checksum/signature、GitHub provenance和commit-pinned Actions；
- Shell/Bats installer：fixed origin、Ed25519/SHA-256、same-dir random staging、rollback和`sc`/`rc` no-clobber，8/8；
- PowerShell/Pester installer：checksum、rollback、built-in `sc.exe` conflict、managed alias ownership、explicit Private-profile program firewall和UTF-8 output，Windows真机11/11；
- T-023：建立Python→Rust 339-test迁移矩阵；README/CHANGELOG/CONTRIBUTING/SECURITY/build/migration全部切换；删除Python runtime/tests/setup/pyproject/requirements/PyInstaller；Cargo workspace是唯一version source和唯一`quick-share` binary；
- 独立切换边界：`c371fcf`保留Python tree并经干净archive成功构建v1.9.1 wheel；紧随的`chore!: remove Python runtime after Rust parity`提交只移除Python runtime/tests/packaging并更新切换证据；
- 补齐Web production bounded download/upload terminal progress和body-drop interruption；
- 用户追加Windows encoding：command boundary支持strict UTF-8、UTF-16LE和GBK→UTF-8，Windows真机2/2；不转码file/text payload bytes；
- 两阶段评审：`Batch-8-review.md`；评审中移除带RustSec中危advisory的Sigstore候选依赖，并修复Shell签名fail-open、PowerShell checksum变量冲突/staging cleanup、updater rollback、action pinning和uninstall ownership；Batch 8无未解决P0/P1；
- Rust workspace：163 tests listed（162默认通过、1个true-host mDNS ignored）；Clippy、fmt、Linux/Windows release、Windows GNU all-target check、RustSec、fuzz locked check、actionlint和`git diff --check`通过；
- fuzz：1,499,582 runs / 21 seconds，无crash、panic、OOM或sanitizer finding；
- GitHub repository已配置`RELEASE_SIGNING_KEY_PEM` secret；private key不在worktree/日志，公开root位于`security/release-signing-key.pem`并有一致性test。

## 当前闸门：Batch 9

Batch 8已关闭。未经用户明确批准，不开始T-024跨平台、性能、安全与发布总验收。

Batch 9硬门包括：

1. `snow 0.10.0`专家安全审查/审计证据或替代实现；
2. macOS Intel/Apple Silicon真机matrix；
3. 真实signed candidate release和Linux/macOS/Windows self-update/rollback；
4. 1 GiB/10 GiB/10,000 files、进程重启、磁盘不足、权限、冲突和性能/RSS基准；
5. 跨sender CLI invocation pending-transfer选择/恢复UX；
6. 最终`rust-rewrite-acceptance-report.md`和发布负责人明确批准v2.0.0 tag。
