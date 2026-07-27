# Quick Share 2 Rust 重写总验收报告

> 日期：2026-07-27
> 候选版本：`2.0.0-alpha.0`
> 候选提交：`c553e9b3e09e2cd4618b33d8552bc45e9aa5820f`
> 范围：T-024 / Batch 9
> 当前结论：**本地与Windows可用环境P0/P1通过；macOS真机/Apple签名按批准范围延期；发布负责人已于2026-07-27批准最终`v2.0.0` tag与发布**

## 1. 验收政策与明确延期

用户在进入Batch 9后明确批准以下范围调整：

1. 当前没有macOS Intel/Apple Silicon机器；
2. 当前没有Apple Developer账号；
3. Developer ID、codesign、notarization、stapling和Gatekeeper无警告安装不作为当前阻塞项；
4. macOS真实LAN、GUI clipboard和本地真机安装不得伪报通过；
5. 后续又指示“不用管gh了”，因此本报告不以创建/tag/publish GitHub candidate release作为当前动作，也不声称真实remote updater安装已完成。

跨平台Ed25519 `SHA256SUMS.sig`、SHA-256、fixed GitHub origin、updater验证和GitHub provenance是与Apple签名不同的供应链机制；实现、fixture和failure tests仍纳入本次安全验收。

状态定义：

- **PASS**：本候选代码或真实binary已执行并有证据；
- **PASS (automated)**：自动化测试覆盖，但当前没有额外true-host手工设备；
- **DEFERRED (approved)**：用户明确接受的外部设备/账号/发布动作延期；
- **BLOCKED**：未获豁免且阻止最终tag。本报告没有代码级P0/P1 blocker；最终tag批准门已于2026-07-27解除。

## 2. Red 基线与本批修复

Batch 9开始时没有默认勾选需求清单。真实验收首先暴露以下问题：

| 级别 | Red 发现 | 根因 | Green 修复 | 回归结果 |
|---|---|---|---|---|
| P1 | 10,000个1-byte文件约`11 files/s`，15分钟后仍可能因complete超过10秒失败 | TCP Nagle作用于串行小request/ACK；每块及每文件重写完整1.3 MiB state；逐文件commit checkpoint | TCP两端`TCP_NODELAY`；固定40-byte append-only chunk log；256个小块/最多16 MiB bounded sync batch；文件和目录batched intent/final checkpoint；20,000 entry hard bound容纳10,000 files及目录metadata | Linux 10,000 Unicode files+101 dirs+empty dir：9.816s（1,018.74 files/s）；Windows：34.324s；byte/content checks通过 |
| P1 | sender CLI进程退出后没有可选择的production resume UX | 每次send生成新transfer ID；wire不能区分fresh offer和explicit resume；receiver同进程会将重复ID当replay | 每次direct send显示UUID；新增`send --resume UUID`；`OfferCreate.resume`；same authenticated static identity + exact offer/manifest才可re-authorize；resume zero-peer禁止Web fallback | Linux receiver kill+restart 1 GiB resume 18.616s；Windows receiver kill+restart 512 MiB resume 10.294s；partial final始终不存在 |
| P1 | Windows release smoke把PowerShell 5.1 remoting中的null `Process.ExitCode`当失败 | `Start-Process`+redirect在该环境不可靠公开ExitCode | 参数less WaitForExit flush；检查稳定`^error:` contract、sender status及committed payload | Windows release smoke PASS |
| P2 | 10,000 files加root/dirs超过原10,000 manifest-entry limit；Windows命令行无法通过10,000独立path绕过 | “10,000 files”与“10,000 total entries”边界冲突 | QSP hard bound提高到20,000，8 MiB offer byte bound仍生效 | 单目录参数可在Linux/Windows发送10,000 files、目录和empty dir |
| P2 | discovery+transfer约2.022s，超出“通常2秒”体验预算边缘 | 默认scan本身为2,000ms | 默认scan改为1,800ms，仍可配置 | 同机production mDNS discovery+Noise transfer 1.820s |
| P2 | 10,000条offer/progress终端日志不可读 | 每entry和每chunk都render | offer最多显示前100项+omitted count；sender/receiver进度按约1%节流，initial/final必报 | 10,000-file真实transfer日志有界且完成 |
| P2 | 依赖许可证没有机器执行的policy gate | Release只有RustSec和SBOM | 新增`deny.toml`和commit-pinned cargo-deny action | 366 dependency graph licenses/sources PASS |
| Security Gate | 先前认为`snow 0.10.0`无正式第三方审计 | 上游README尚未合并2026年公开报告链接 | 定位Trail of Bits正式报告、1Password说明和fix review；验证8个fix commit都是v0.10.0 ancestor | 0 high；medium/low均已修复；两个informational完成适用性分析 |

## 3. 需求第3节验收矩阵

### 3.1 核心流程

| Case | 状态 | 证据 |
|---|---|---|
| Linux、Windows单一binary | PASS | Linux release及Windows GNU cross binary true-host运行；Cargo metadata只有`quick-share` |
| macOS单一binary | DEFERRED (approved) | release matrix包含Intel/ARM；无本地机器，不声称true-host PASS |
| `sc`/`rc`无冲突可用且不覆盖 | PASS | Bats 8/8、Pester 11/11、Windows built-in `sc.exe` conflict、Linux real install smoke |
| `rc`纯终端receiver | PASS | Linux/Windows production listener、Noise loopback、graceful stop |
| `sc`发现并选择receiver | PASS | production mDNS+Noise 1.820s；双机Linux/Windows历史矩阵 |
| definitive zero自动Web | PASS | `routing-linux.sh` production HTTPS fallback |
| 拒绝/超时/失败不Web fallback | PASS | production rejection no-Web；orchestration matrix |
| `--peer`/`--web`强制模式 | PASS | production explicit cases和parser/route tests |

### 3.2 内容类型

| Case | 状态 | 证据 |
|---|---|---|
| 单/多/空文件 | PASS | production loopback、manifest/store/direct tests |
| 大于4 GiB | PASS | 10 GiB production Noise transfer + sparse >4 GiB u64 test |
| 目录结构和空目录 | PASS | Linux/Windows 10,000 Unicode file directory，empty-dir存在，diff/sample verify |
| 文本和clipboard | PASS | Linux headless、Windows native Unicode clipboard和production text direct历史证据 |
| 文本不执行 | PASS | shell payload test和safe output tests |
| 默认不follow symlink | PASS | manifest/direct/store tests及Linux/Windows symlink matrix |
| `--follow-links` cycle/escape | PASS (automated) | cycle、broken、outside、Windows portability tests |

### 3.3 安全

| Case | 状态 | 证据 |
|---|---|---|
| 陌生accept/reject | PASS | production non-TTY reject、Linux/Windows SAS logs、offer tests |
| accept once / accept+trust | PASS | SAS-gated state machine和Windows/Linux真机身份证据；`--yes`只能once |
| trust绑定static identity而非IP | PASS | full-key pinning、IP-independent device ID、wrong-key fail tests |
| encryption/auth/integrity | PASS | fixed Noise XX、MITM/replay/tamper/wrong pin、BLAKE3 final digest |
| trusted auto/confirm | PASS | OfferPolicy tests及config contract |
| devices list/rename/remove | PASS | production command tests，无完整key输出 |
| log/config/update secret redaction | PASS | Debug/log tests、repository secret scan、signed URL redaction |
| Windows Public profile诊断且不改firewall | PASS | Windows true-host diagnostic，firewall hash unchanged；installer仅explicit Private rule |
| `snow`第三方审计门 | PASS | `snow-0.10-security-evidence.md`；Trail of Bits 4 engineer-weeks；v0.10 ancestry验证 |

### 3.4 性能与可靠性

测试主机：Ubuntu 5.15 x86_64，31 GiB RAM，测试磁盘剩余约76 GiB；release build；同机TCP loopback+Noise+真实staging/fsync/hash/commit。该环境不是宣称千兆线速的专用裸机，只用于可复现相对资源和程序瓶颈检查。

| Case | 状态 | 结果 |
|---|---|---|
| discovery通常2秒内 | PASS | mDNS discovery+完整小文件transfer 1.820s |
| 1 GiB streaming | PASS | 17.569s，58.28 MiB/s；sender 65,876 KiB RSS，receiver 14,940 KiB |
| 10 GiB streaming | PASS | 183.478s，55.81 MiB/s；sender 68,176 KiB RSS，receiver 15,136 KiB |
| RSS不随文件线性增长 | PASS | payload 10x，sender RSS 1.035x，receiver RSS 1.013x |
| 无显式程序限速 | PASS | 无rate limiter；默认durable 4 MiB chunks达到约446–466 Mbit/s；16 MiB option 1 GiB为61.09 MiB/s但RSS提高到138 MiB，保留4 MiB安全默认 |
| 10,000 files pipeline | PASS | Linux 9.816s/1,018.74 files/s/22.7 MiB sender RSS；Windows 34.324s |
| sender/receiver重启resume | PASS | Linux 1 GiB和Windows 512 MiBproduction kill/restart，same UUID exact-manifest resume |
| finite retry | PASS | 最多3次+jitter tests；nonretry identity/integrity/cancel不循环 |
| no partial final | PASS | kill/reject/storage/permission/integrity tests；只留hidden staging |
| storage/permission/conflict | PASS | RLIMIT_FSIZE deterministic storage exhaustion、chmod read-only、rename conflict production matrix |
| port conflict | PASS | real occupied listener显示Address already in use |
| extreme power-loss durability | PASS (fail-closed design) | large chunks ACK前sync；small batches最多256 records/16 MiB；最终BLAKE3禁止暴露损坏final；真实断电未执行 |

可复现命令：

```bash
scripts/acceptance/large-transfer-linux.sh target/release/quick-share 1G
scripts/acceptance/large-transfer-linux.sh target/release/quick-share 10G
scripts/acceptance/small-files-linux.sh target/release/quick-share 10000
scripts/acceptance/resume-restart-linux.sh target/release/quick-share 1G
scripts/acceptance/faults-linux.sh target/release/quick-share
scripts/acceptance/routing-linux.sh target/release/quick-share
```

### 3.5 Web模式

| Case | 状态 | 证据 |
|---|---|---|
| catalog browse/download | PASS | production HTTPS及catalog tests |
| URL/QR/curl/wget | PASS | endpoint tests和production output |
| upload/quota/timeout | PASS | streaming multipart、wrong password rate、quota race、timeout shutdown |
| path traversal/symlink/TOCTOU | PASS | canonical reopen、post-catalog swap、public DTO tests |
| desktop/mobile UI | PASS | Batch 7 Chrome desktop/iPhone 14、console 0、WCAG AA 0 issue；UI本批未修改 |
| HTTP显式opt-in | PASS | production explicit HTTP；TLS失败无downgrade |
| Web streaming progress | PASS | 2 MiB production start/1 MiB/completed，body drop interruption |

### 3.6 配置与发布

| Case | 状态 | 证据 |
|---|---|---|
| CLI > env > TOML > default | PASS | config integration tests |
| Downloads default/override | PASS | AppDirs/receive tests |
| Shell/PowerShell no-clobber | PASS | Bats 8/8、Windows Pester 11/11 |
| signed updater/check/rollback | PASS (automated) | fixed-origin provider、strict checksum、Ed25519 fixture、candidate size/version、fake replacement rollback tests |
| real signed remote updater | DEFERRED (approved) | 用户指示当前不再处理gh；现有latest仍是v1资产，不伪报remote install |
| actual GitHub three-platform release assets | DEFERRED (approved) | workflow和contract已建立；当前报告不创建tag/release |
| macOS signing/notarization | DEFERRED (approved) | 无Developer账号；README明确unsigned/Gatekeeper warning |
| SPDX SBOM/provenance | PASS (pipeline contract) | release jobs对actual raw binaries生成SPDX和GitHub attestation；真实candidate artifact随tag延期 |

## 4. 安全与供应链

### 4.1 `snow 0.10.0`

- Trail of Bits report SHA-256：`d2ee52a3ffed85a382201a353f844df21ee7ebeafe5f6539f215469baa8bd433`；
- 审计目标commit：`009411d2035a77ece57b0a6873169de7693a0aa0`；
- `v0.10.0` commit：`4bb43f50370bdb3e8b1b57814ac662864db2704f`；
- 8个resolved finding fix commits均为v0.10 ancestor；
- TOB-SNOW-2只涉及PSK+pre-message/custom/fallback，不适用固定XX；
- TOB-SNOW-8 ephemeral memory non-zeroization为informational，本机root/physical access残余风险；不赋予LAN attacker解密/伪造能力；
- Quick Share继续禁止PSK、fallback、custom suite和runtime negotiation。

### 4.2 自动扫描

```text
cargo audit: 366 dependencies, no advisory
cargo deny check licenses sources: PASS
repository private-key/token scan: PASS
release public key consistency: PASS
GitHub Actions action refs: 40-hex commit pinned
product crates: forbid(unsafe_code)
```

### 4.3 Fuzz

```text
noise_records
2,306,514 runs / 31 seconds
no crash, panic, OOM, timeout finding, or sanitizer finding
```

## 5. 自动化和平台结果

- Rust workspace：167 tests listed；166 passed；1 true-host multicast-only mDNS test ignored by default；
- fmt：PASS；
- Clippy all-target/all-feature `-D warnings`：PASS；
- Linux release：PASS；
- Windows GNU all-target/all-feature cross check和release：PASS；
- Windows 11 Enterprise AMD64：release smoke、10,000 files、512 MiB receiver restart resume、Unicode/GBK和Pester 11/11：PASS；
- Bats：8/8；
- actionlint：PASS；
- cargo-deny licenses/sources：PASS；
- RustSec：PASS；
- `git diff --check`：PASS。

GitHub-hosted matrix曾由draft PR启动，但用户随后明确要求不再处理gh。本报告不轮询、不把未读取的runner结果计入通过，也不以它作为本轮结论依据。

## 6. 两阶段评审结论

### 阶段1：规范符合性

- 需求第3节全部转换为上表自动化/true-host/manual/deferred case；
- 发现的P1小文件性能/complete timeout和跨进程resume UX已修复并在Linux/Windows重验；
- 自动Web边界、SAS/trust、完整性、路径、Web、updater和installer约束没有为通过性能测试而放宽；
- macOS和真实GitHub release只按用户明确指示延期，没有伪造证据。

判定：**当前批准范围通过**。

### 阶段2：代码质量与安全

- append log固定40 bytes，40 MiB hard cap，entry/index/digest replay严格验证，partial record重开时截断；
- small checkpoint批次固定256/16 MiB，不是无界buffer；
- batch commit先持久化全部intent，再做capability-bound hardlink/rename，parent sync后一次final checkpoint；中途失败由Committing reconciliation恢复；
- resume必须显式flag，fresh replay仍拒绝；同manager只对Accepted+same owner full key+byte-identical offer签发新token；receiver restart则重新走normal confirmation；
- resume definitive-zero不会Web fallback；source snapshot/manifest变化失败；
- TCP_NODELAY只移除request/ACK delayed-packet latency，不修改加密、auth、ACK或resource bounds；
- dependency license policy、Snow audit适用性和residual risk都有版本化证据。

判定：**无未处理代码级P0/P1**。

## 7. 发布决策

### 当前可以批准的内容

- Linux和Windows候选核心CLI、安全直传、Web、installer、性能和故障恢复；
- Rust-only切换；
- fixed-origin signed update/release pipeline实现；
- 无代码级P0/P1、无已知高危依赖advisory。

### 当前不能声称的内容

- macOS Intel/Apple Silicon true-host功能已通过；
- macOS Developer ID签名/公证已完成；
- public GitHub `v2.0.0` signed assets和real remote self-update已通过（必须等发布工作流和产物验证后才能声明）。

### 最终状态

**T-024在用户调整后的本轮执行范围内为Conditional PASS；该范围和最终`v2.0.0`发布已由发布负责人于2026-07-27明确批准。**

发布后仍需验证实际GitHub资产、签名、SBOM、provenance和release smoke；这不会改变macOS true-host/Apple signing的DEFERRED状态。未来补充DEFERRED项时，不需要重做独立且代码未变的Linux/Windows、安全和性能证据。
