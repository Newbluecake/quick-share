# Batch 8 两阶段评审：T-021 至 T-023

> 日期：2026-07-27
> 范围：安全自更新、三平台安装与 Release CI、Python→Rust 最终切换、Windows GBK/UTF 编码边界
> 结论：**通过，允许进入 Batch 9 人工闸门；Batch 8 范围内无未解决 P0/P1**

## 1. 评审范围

- `crates/quick-share-update/src/{lib,github,checksum,signature,replace}.rs`
- `crates/quick-share-update/tests/update.rs`
- `crates/quick-share-cli/src/{app,lib,terminal}.rs`
- `crates/quick-share-cli/tests/release_contract.rs`
- `crates/quick-share-platform/src/encoding.rs`
- `crates/quick-share-platform/src/network.rs`
- `install.sh`
- `install.ps1`
- `tests/install/test_install_sh.bats`
- `tests/install/test_install_ps1.Tests.ps1`
- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`
- `scripts/release/{smoke-unix,smoke-windows,check-signing-key}.sh/.ps1`
- `security/release-signing-key.pem`
- `README.md`、`CHANGELOG.md`、`CONTRIBUTING.md`、`SECURITY.md`、`docs/migration-v2.md`
- Python runtime、packaging、tests 和 PyInstaller build 的删除

本批次把 `quick-share update` 从占位命令接入固定 GitHub 仓库、签名清单和跨平台自替换，并把发布/安装链切换为 Rust 单文件。发布私钥没有进入工作区：新的 Ed25519 私钥只写入 GitHub Actions Secret `RELEASE_SIGNING_KEY_PEM`；仓库仅保留公开验证根。

## 2. 阶段一：规范符合性评审

### 2.1 T-021：签名验证、原子替换和失败回滚自更新

结论：**通过**。

- `quick-share update --check`、`--yes` 和 `--version` 已接入生产 CLI；非交互安装没有 `--yes` 时拒绝，check-only 不下载资产；
- semver 使用 `semver::Version`，latest 不允许隐式 downgrade，显式旧版本同样拒绝，requested tag 与 API 返回版本必须完全一致；
- GitHub API origin、owner、repository 和 release asset path 固定为 `Newbluecake/quick-share`；初始 URL 必须 HTTPS、无 userinfo/query/fragment，tag 必须匹配；
- redirect 最多 5 次，只允许 HTTPS 的 `github.com`、`api.github.com` 和 `release-assets.githubusercontent.com`，HTTP downgrade 和 foreign host fail closed；
- metadata 1 MiB、checksum manifest 1 MiB、signature 4 KiB、binary 512 MiB，全部流式/checked 计数；asset 实际字节数必须与 GitHub metadata 精确一致；
- release 必须同时包含当前 target raw binary、`SHA256SUMS` 和 `SHA256SUMS.sig`，重复或缺失资产拒绝；
- `SHA256SUMS.sig` 是对 manifest 原始字节的 Ed25519 signature；updater 使用内嵌完整公钥验证后才解析目标 SHA-256，再验证 binary；
- checksum parser 要求唯一 exact basename、64 hex digest，拒绝 path-shaped name、duplicate 和 malformed manifest；
- candidate 在当前 executable 同目录使用随机 create-new 名称，download/cancel/error 后 Drop 清理；Linux 权限只继承普通 0o777 bits，明确剥离 setuid/setgid/sticky；
- replacement 前创建 durable runnable backup并执行 candidate `--version`；replacement 后再次启动 installed path并要求 exact `quick-share <expected-version>`；
- Unix 和 Windows 使用 `self-replace 1.5.0` 平台策略。Windows running-exe lock 由 relocated helper 处理；外围 backup/内容比较/rollback 处理 immediate failure 和 postflight failure；
- checksum、signature、download interruption、stale plan、replacement error 和 startup error 都不会把未验证 candidate 暴露为成功安装；
- UpdateError human/Debug 不显示 reqwest URL，`ReleaseAsset::Debug` 隐藏 signed redirect URL，避免 query signature/token 进入日志；
- Ctrl+C cancellation 停止 bounded download并清理 partial candidate。

### 2.2 T-022：安装脚本和跨平台 Release CI

结论：**通过**。

#### Shell installer

- Linux x86_64 固定选择 musl asset；macOS 显式区分 x86_64/Apple Silicon；unsupported OS/arch fail closed；
- 只接受固定 GitHub release URL，curl 强制 HTTPS/TLS 1.2，redirect 最终 host 必须是 GitHub/release-assets；
- 下载 raw executable、`SHA256SUMS` 和 `.sig`；OpenSSL 支持 Ed25519 时使用脚本内 pinned DER key 验签，否则在可用时用 `gh attestation verify`，最后才明确提示仅 HTTPS+SHA-256；
- install-dir 内用 `mktemp` 创建不可预测 regular staging，避免可预测 PID 文件名/symlink clobber；candidate preflight、move、postflight 任一步失败恢复已有 executable；
- `sc`/`rc` 的 PATH 或目标路径一旦存在就只警告、不覆盖；Unix 创建相对 symlink到同一 binary。

#### PowerShell installer

- Windows AMD64 显式映射 MSVC asset；版本和 fixed-origin URL 严格验证；
- SHA256SUMS exact/duplicate check、可选 OpenSSL Ed25519/`gh` provenance、GUID staging、pre/postflight 和 rollback 已实现；
- Windows 自带 `sc.exe` 被视为真实冲突，不会被 Quick Share 覆盖；`rc.exe` 只在 free 时复制；managed marker 使 uninstall 不删除用户预先存在的同名文件；
- firewall 只有显式 `-AddPrivateFirewallRule` 才调用 `New-NetFirewallRule`，且限定 installed program、Inbound、TCP、Private profile；同名 rule 已存在时拒绝修改；uninstall 只移除 program 匹配的自身 rule；
- 默认安装到 `%LOCALAPPDATA%\QuickShare\bin` 并更新 User PATH，不要求管理员或 Python。

#### Release CI

- matrix 构建 Linux GNU、Linux musl、macOS Intel、macOS Apple Silicon、Windows MSVC；
- raw executable 是 updater/installer 资产，同时产生 versioned tar.gz/zip；
- 每个平台对最终 staged executable 运行 `--version`、`--help` 和真实 loopback Noise file transfer；
- tag version 必须与 Cargo binary `--version` exact match；
- 每个 raw binary 生成 SPDX JSON SBOM 和 GitHub build provenance；
- aggregate job deterministic 生成 SHA256SUMS，用 protected Ed25519 secret生成 raw 64-byte signature，再 attest checksum manifest并创建 GitHub Release；
- release 前重新执行 fmt、Clippy、workspace tests、RustSec 和 pinned public-key consistency；
- 所有 GitHub Actions（含 third-party action）使用 40-hex commit pin，不使用 mutable `@master`/major tag；`actionlint 1.7.12` 通过；
- CI 的 Linux/Windows installer jobs执行 Bats/Pester；platform matrix由 check 提升为三平台 all-target/all-feature tests。

### 2.3 T-023：Python 功能迁移、切换和清理

结论：**通过（最终 v2.0.0 发布仍受 T-024 闸门约束）**。

- 建立 `docs/migration-v2.md`，把 v1.9.1 的 339 tests 按 CLI/config、Web、upload、ZIP、security、network、progress、updater、installer 和 release 映射到 Rust/Bats/Pester/E2E；
- Web 传统模式补齐 bounded download/upload terminal progress：1 MiB 节流、start/progress/complete/interrupted，filename 终端转义，body drop停止 producer；
- README 全量切换到 Rust `send/receive/serve/devices/config/update`、Noise/SAS、严格 Web fallback、HTTPS、安装、构建与平台说明；所有示例由 parser/release contract验证；
- `CHANGELOG.md` 增加 Unreleased Rust 2 rewrite、breaking migration、security 和 removal；
- 新增 `CONTRIBUTING.md`、`SECURITY.md`、MIT `LICENSE`、v1→v2 config/rollback指引；
- `build.sh` 只运行 pinned Rust fmt/Clippy/test/release build，不包含 pip/pytest/PyInstaller；
- 删除 `src/*.py`、全部 Python tests、`setup.py`、`pyproject.toml`、requirements、coverage/PyInstaller build 和 Python CI；
- `Cargo.toml [workspace.package].version` 是唯一 product/package version source；cargo metadata只列出一个 binary：`quick-share`；
- release workflow禁止 `setup-python`、pip、PyInstaller和 Python entrypoint；
- v1 shared secret不迁移为 v2 trust；旧 config只保留安全 output directory，migration文档提供 tag v1.9.1 rollback步骤；
- 切换历史按任务要求拆分：前置提交 `c371fcf` 已包含完整Rust实现但仍保留全部Python源码/packaging；从该提交导出的干净源码已成功构建 `quick_share-1.9.1-py3-none-any.whl`。紧随其后的独立 `chore!: remove Python runtime after Rust parity`提交只执行Python runtime/tests/packaging切除和切换证据更新。

### 2.4 Windows GBK→UTF-8 补充需求

结论：**通过**。

- Rust内部文本保持 UTF-8/Unicode；文件名由 Windows wide API/OsString进入，不按 GBK 猜测；
- `quick-share-platform::encoding` 只位于 subprocess OS boundary，优先 strict UTF-8，其次 BOM/heuristic UTF-16LE，最后用 `encoding_rs::GBK` 转为 UTF-8；三者都失败时 fail closed；
- Windows network PowerShell命令显式设置 UTF-8 output，并仍用 decoder兼容旧中文 code page；
- `install.ps1` 和 Windows smoke脚本设置 `$OutputEncoding`/Console UTF-8；产品 warning使用自有 ASCII `[WARN]` 标签，避免本地化 PowerShell prefix经 SSH/GBK变乱码；
- Linux和Windows真机均通过 UTF-8、UTF-16LE、GBK中文 round-trip tests；Windows Pester日志的产品 warning不再乱码；
- 该转换不用于 file body、chunk、text payload 或 redirected stdout data，传输字节不被静默转码。

## 3. 阶段二：代码质量与安全评审

结论：**通过；Batch 8 范围内无未解决 P0/P1。**

### 3.1 供应链与密钥边界

- 初版曾评估 GitHub Sigstore verification crate，但它引入无修复的 `RUSTSEC-2023-0071` (`rsa`) 中危 advisory 和超过 200 个额外依赖，因此在进入产品前完整移除；
- 最终方案只新增 `ed25519-dalek`、bounded reqwest和 `self-replace`，最终 RustSec 366 dependencies无 advisory；
- release private key由本机临时生成后直接写入 GitHub Actions Secret并删除，工作区、日志、fixtures和项目 memory均无 private key；
- public PEM、updater raw key hex、Shell/PowerShell DER base64由 `check-signing-key.sh`逐次对照，并验证固定 signed fixture；
- GitHub Secret `RELEASE_SIGNING_KEY_PEM` 已存在；只在 signing step注入，不传给 build、SBOM、third-party action或 artifacts；
- key rotation采用 overlap release：旧 key签名的版本先同时信任新 key，之后才切换 secret，不能同一首发直接替换 trust root。

### 3.2 更新事务与 Windows 文件锁

- service边界把 release metadata、download、signature verifier和 executable replacer抽象为独立 trait，fault tests不访问真实网络/当前 test binary；
- candidate/manifest/signature都有同目录随机临时生命周期；signature→checksum→binary→preflight→backup→replace→postflight顺序不可绕过；
- `self-replace` Windows策略会 relocate running image并在 parent退出后清理；Quick Share外围另有独立 durable backup；
- replacement API失败时比较 destination与backup：未变则确认旧版本仍在，变化/缺失则把candidate移开并restore；postflight失败走同一 rollback；
- cleanup failure不把已经成功启动的更新伪报为内容验证失败；残留随机 hidden backup可由系统/用户处理，不自动宽泛清理目录。

### 3.3 Installer fail-closed 与 no-clobber

- Shell review发现 `die`仅 return时，在函数被 `if`调用导致 Bash `errexit`抑制，后续可能继续并把签名失败当成功；所有 security guard改为显式 `return 1`，tampered signature test由错误通过变为稳定失败；
- PowerShell review发现 `$matches` 与自动变量 `$Matches`大小写不敏感冲突，导致合法 checksum解析异常；改为独立 `$foundChecksums`；
- PowerShell candidate invocation异常最初绕过 staging cleanup；preflight改为 try/catch + remove，Pester验证无 `.quick-share.new.*`；
- Shell staging由可预测 `.$$` 改为 install-dir `mktemp` regular file，阻止 precreated symlink clobber；
- PowerShell uninstall最初会删除 install-dir内未由安装器创建的 `sc.exe`/`rc.exe`；managed marker修复 ownership，新增 no-delete test；
- firewall tests使用 mock并在真机运行前后不触发系统修改；实际 executable/alias smoke未使用 firewall开关。

### 3.4 资源、日志与可维护性

- updater所有网络body、redirect、asset count/name和signature/checksum都边界明确，不对 release binary做内存聚合；
- candidate `--version`有 10 秒 deadline和 4 KiB stdout bound；stderr丢弃，避免 untrusted preflight输出进入终端；
- Web progress用容量 64 channel、1 MiB节流和容量 4 body channel；client drop传播 interruption并停止读取；
- Windows encoding fallback只处理 command output，错误不包含原始可能敏感bytes；
- product crates继续 `#![forbid(unsafe_code)]`；Windows executable lock的 unsafe OS细节封装于锁定的 `self-replace` dependency，不复制到产品代码。

### 3.5 评审中发现并已修复

| 级别 | 发现 | 修复与回归 |
|---|---|---|
| P1 | Sigstore候选库引入无修复 `rsa` timing advisory和巨大依赖面 | 完整移除，改为 pinned Ed25519 signed manifest；RustSec恢复零 advisory；固定fixture/脚本/updater验证通过 |
| P1 | Bash `die`在 conditional context下可能因 `errexit`被抑制后继续，tampered signature测试实际返回成功 | 所有安全guard显式 return；Bats和手工mutation test确认失败状态非零 |
| P1 | `self-replace` immediate error的不同Windows阶段不能只依赖“destination是否存在”判断旧版本完整 | durable backup + byte compare + move-aside + restore统一rollback；fake replacement与cross-platform build回归 |
| P1 | T-023 parity审查发现Web production未消费download/upload progress events | 新增 bounded Web progress event和CLI renderer，真实2 MiB download输出start/1 MiB progress/complete |
| P1 | PowerShell checksum变量与自动 `$Matches`冲突；bad candidate异常留下staging | 重命名变量并用try/catch cleanup；Windows Pester从8/10提升到11/11 |
| P2 | updater HTTP error Debug可能包含GitHub signed redirect query | UpdateError自定义Debug/固定Display，ReleaseAsset URL脱敏；token marker test通过 |
| P2 | download只检查最大值，未检查GitHub asset metadata exact size | EOF后要求received == metadata size，clean truncation归类 interrupted |
| P2 | updater权限可能继承setuid/setgid bits | 仅继承0o777并确保owner read/execute，special bits剥离 |
| P2 | GitHub Actions使用mutable major/master refs | checkout、toolchain、cache、artifact、attestation、SBOM、release和audit actions全部pin 40-hex commit；contract/actionlint通过 |
| P2 | Windows installer uninstall可能删除未管理alias | `.quick-share-managed-{sc,rc}` ownership marker；unmarked alias保留test通过 |
| P2 | Windows旧PowerShell/SSH环境出现GBK warning prefix乱码 | 加入UTF-8/UTF-16LE/GBK boundary decoder，PowerShell UTF-8设置和ASCII warning标签；Windows真机通过 |

## 4. 验证结果

```text
cargo fmt --all -- --check                                           PASS
cargo clippy --workspace --all-targets --all-features -- -D warnings PASS
cargo test --workspace --all-targets --all-features                  PASS
  163 tests listed; 162 passed + 1 true-host mDNS ignored
cargo check --workspace --all-targets --all-features \
  --target x86_64-pc-windows-gnu                                    PASS
cargo build -p quick-share-cli --release --locked \
  --target x86_64-pc-windows-gnu                                    PASS
cargo build --workspace --release --locked                           PASS
cargo audit                                                          PASS
  366 dependencies; no advisory
cargo check --manifest-path fuzz/Cargo.toml --locked                 PASS
actionlint 1.7.12                                                    PASS
scripts/release/check-signing-key.sh                                 PASS
Bats installer contracts                                             PASS (8/8)
Pester 5.7.1 installer contracts on Windows 11                       PASS (11/11)
git diff --check                                                     PASS
```

其他验证：

- `./build.sh`：fmt、Clippy、163 tests、release build、`--version`/`--help` 全链通过；
- Linux release artifact loopback smoke：isolated sender/receiver HOME、Noise direct、payload compare、receiver once shutdown通过；
- Linux Shell installer：真实release executable preflight/install/postflight、relative `sc`/`rc`、existing alias no-clobber通过；
- Windows PowerShell installer：parser、target mapping、checksum duplicate、rollback cleanup、no-clobber、uninstall ownership、firewall parameter tests 11/11；
- Windows真实cross-built executable install：old replacement、`--version`、built-in `sc.exe` conflict skip、`rc.exe` create通过；
- Windows GBK/UTF test executable：2/2；UTF-8、UTF-16LE、GBK中文转换一致；
- Windows GNU all-target/all-feature check和release binary build通过；
- Cargo metadata：唯一binary `quick-share`，全部product crates version `2.0.0-alpha.0`；
- Python predecessor (`c371fcf`)：`python3 setup.py bdist_wheel`成功生成`quick_share-1.9.1-py3-none-any.whl`；
- Python runtime scan：当前切换树无 `.py`、`src/`、setup/pyproject/requirements/PyInstaller spec；
- release signing private-key scan：无 private PEM、GitHub token或secret value；GitHub Actions secret仅能列出名称/更新时间；
- Web production 2 MiB download：start、1 MiB progress、2 MiB progress、completed日志与payload compare通过；
- `noise_records` fuzz：1,499,582 runs / 21 seconds，无 crash、panic、OOM或sanitizer finding。

## 5. 已知限制与 Batch 9 硬门

| 级别 | 项目 | 后续处理 |
|---|---|---|
| Release Blocker | `snow 0.10.0` 无正式第三方安全审计 | T-024取得专家审查/审计证据，或迁移实现；不得仅凭RustSec绿色发布 |
| Release Blocker | 新release workflow尚未在本未提交worktree的真实tag上执行 | T-024/发布候选提交后观察五平台matrix、attestation、signature、SBOM和Release assets全部绿色 |
| 平台门 | macOS Intel/Apple Silicon仍无本地真机证据 | T-024在两种macOS硬件执行安装、mDNS、Noise、Web TLS、clipboard、symlink、update/rollback |
| 更新真产物门 | 当前GitHub latest仍为v1.9.1，不含v2 raw/signed assets，因此alpha binary不能完成真实remote self-update | 候选release产物生成后在Linux/macOS/Windows安装旧candidate→更新新candidate→故障rollback |
| 恢复UX | 跨sender CLI invocation的pending transfer选择/恢复界面仍未补齐 | T-024前实现或明确降级边界 |
| Windows alias UX | PowerShell installer按任务允许copy `rc.exe`；更新main binary不会原子刷新正在使用/旧copy alias | T-024决定改为managed shim/hardlink策略或让installer rerun刷新，并记录行为 |
| 平台专项 | Linux X11/Wayland真实desktop clipboard、X11 ownership、Windows无Developer Mode symlink notice待验证 | T-024平台矩阵完成 |

## 6. 最终结论

T-021、T-022、T-023 已满足安全自更新实现、单文件安装/发布链和Python→Rust代码切换目标。发布链使用固定仓库、pinned Ed25519根、SHA-256、原子staging/rollback、commit-pinned Actions、SBOM和GitHub provenance；安装器不覆盖已有快捷命令，不静默修改firewall；Cargo是唯一版本源且release只构建一个Rust binary。用户追加的Windows GBK→UTF兼容已在明确OS command boundary实现并通过Windows真机。

Batch 8 可以关闭并停在 Batch 9 人工闸门。此结论不是v2.0.0发布批准：T-024必须完成macOS双架构真机、真实signed release/update rollback、`snow`安全门、性能/大文件/安装总矩阵和发布负责人批准。
