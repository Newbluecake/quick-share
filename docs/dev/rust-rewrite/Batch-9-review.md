# Batch 9 两阶段评审：T-024

> 日期：2026-07-27
> 候选：`2.0.0-alpha.0` / `c553e9b`
> 完整证据：`rust-rewrite-acceptance-report.md`
> 结论：**用户调整范围内Conditional PASS；无未处理代码级P0/P1；final v2.0.0 tag等待单独批准**

## 阶段1：规范符合性

评审按需求文档第3节逐项读取实际代码、测试和true-host输出，没有基于实施摘要直接判定。

### 完整性

- 核心route、内容类型、Noise/SAS/trust、Web、配置、installer和updater均映射到自动化或production case；
- 1 GiB、10 GiB、10,000 files、sender/receiver restart、storage、permission、conflict、port和RSS/throughput有可复现脚本；
- `snow 0.10.0`不再依赖“无audit”假设：Trail of Bits report/fix ancestry/剩余finding适用性已记录；
- 依赖漏洞、许可证、来源、fuzz、SBOM/provenance pipeline均覆盖；
- macOS true-host/Apple signing和当前GitHub candidate release严格标记DEFERRED，不标记PASS，符合用户明确范围调整。

### 准确性

- resume不是使用新ID伪装重传，而是explicit wire resume、same transfer ID、same static owner、byte-identical offer和fresh authorization；
- definitive zero resume不会进入Web；fresh replay仍拒绝；
- 10,000-file case是单目录参数下真实10,000 Unicode files+目录+empty dir，不以10,000空mock替代；
- 10 GiB case传输、接收、hash和commit真实执行，RSS来自release process；
- storage/permission cases没有通过修改firewall或宽松路径规则实现。

### Red→Green修复循环

1. 10,000 files第一次约11 files/s并在complete timeout失败：判定P1，不接受“metadata workload慢”解释；
2. 修复TCP_NODELAY后消除request/ACK delayed-packet瓶颈；append log/batch commit消除O(n²) journal rewrite；
3. Windows目录metadata仍使complete超过10秒：再次判定P1；增加directory metadata batch后真机34.324s通过；
4. Windows PowerShell release smoke错误使用nullable ExitCode：修复后读取stable error contract+payload；
5. sender CLI restart无production选择：增加explicit UUID resume并在Linux/Windows kill/restart重验。

判定：**阶段1通过**。

## 阶段2：代码质量与安全

阶段1通过后才执行本阶段。

### 安全

- `OfferCreate.resume`默认false，legacy fresh decode稳定；resume only接受Accepted、same full public key-derived identity、same owner和exact offer digest；
- authorization token每次随机重发，仍只在Noise内，receiver只存hash；
- chunk log固定40-byte records、40 MiB cap，zero entry/out-of-range/conflicting digest fail closed，partial tail有界截断；
- small-file checkpoint最多256 records/16 MiB，最终BLAKE3和hidden staging不变；
- batch commit先写all intents，capability-bound hardlink/rename后sync parent并写final checkpoint；中断由Committing状态reconcile；
- manifest entry cap从10,000调整为20,000但offer仍有8 MiB byte cap、path/depth/chunk/resource bounds；
- TCP_NODELAY不改变Noise record、authorization或ACK语义；
- Snow audit剩余TOB-SNOW-8是本机root/physical memory informational residual，未被错误描述成LAN-safe zero risk。

### 可维护性与测试

- wire、offer manager、orchestration、store、receiver和CLI各自保留模块边界；
- fault-sensitive API有unit tests，完整用户链有release binary scripts；
- acceptance scripts使用isolated HOME/state/output、固定port、trap cleanup、hash/diff和非零失败；
- GitHub Actions和cargo-deny action均commit-pinned；
- README、CHANGELOG、migration、design和security evidence同步。

### 独立验证

```text
Rust: 167 listed; 166 passed; 1 true-host mDNS ignored
Clippy -D warnings: PASS
Linux/Windows release builds: PASS
Windows production/Pester: PASS
RustSec: 366 dependencies, no advisory
cargo-deny licenses/sources: PASS
fuzz: 2,306,514 runs / 31s, no finding
actionlint / signing-key consistency / diff check: PASS
```

判定：**阶段2通过；无未处理代码级P0/P1**。

## 发布边界

此评审不是final tag授权。以下陈述仍禁止：

- “macOS Intel/Apple Silicon true-host已通过”；
- “macOS binary已Developer ID签名/公证”；
- “真实GitHub signed candidate self-update已通过”；
- “v2.0.0 tag已获批准”。

最终tag必须由用户/发布负责人在阅读`rust-rewrite-acceptance-report.md`后单独明确批准。
