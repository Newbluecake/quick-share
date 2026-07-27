# Batch 2 两阶段评审：T-002 至 T-005

> 日期：2026-07-26
> 范围：QSP/1 数据契约、配置与身份、安全路径/manifest、CLI 契约
> 结论：**通过，允许进入下一人工闸门**

## 1. 评审范围

本批次只固定协议、持久状态、文件系统模型和命令契约，不连接真实发现、Noise 会话或文件传输。评审覆盖：

- `crates/quick-share-protocol/`
- `crates/quick-share-platform/`
- `crates/quick-share-core/`
- `crates/quick-share-cli/`
- `Cargo.toml`、`Cargo.lock`
- `docs/dev/rust-rewrite/rust-rewrite-{design,tasks,context}.md`

## 2. 阶段一：规范符合性评审

### T-002：QSP/1 协议

结论：**通过**。

- 已提供 `DeviceInfo`、`Capabilities`、`TransferOffer`、`ManifestEntry`、`OfferDecision`、`TransferStatus`、`ChunkDescriptor`、`ProtocolError`；
- major 不同会拒绝，minor 使用较低版本，能力取交集；
- offer 编码上限 8 MiB、manifest 上限 10,000 entries、路径和 chunk 均有硬限制；
- `decode_offer` 在 JSON 解析前检查 body 长度，安全敏感结构拒绝未知字段；
- fixture 位于 `tests/fixtures/protocol/v1/offer.json`；
- 稳定错误码、decision/status/chunk wire form 和任意 JSON 不 panic 均有测试。

### T-003：配置、目录、身份和信任

结论：**通过**。

- 配置合并顺序为默认值 → TOML → 环境变量 → CLI override；
- `config.toml` 和 `trusted-devices.toml` 使用 sibling temp + sync + atomic replace；
- 原子替换失败不会破坏旧目标；
- 身份使用持久 X25519 static key，首次并发启动使用 no-clobber 创建并收敛到同一身份；
- identity 文件损坏、版本错误、公私钥不匹配时 fail closed，不静默重建；
- Unix 私钥权限实测为 `0600`；
- Windows 私钥 DACL 实测为 protected，只有当前用户、SYSTEM、BUILTIN\Administrators，均为非继承 FullControl；
- trusted-device pin 同时校验完整 static public key 与其派生 `DeviceId`，支持 list/check/trust/rename/remove；
- Debug 输出对私钥显示 `[REDACTED]`；
- 旧 JSON 只迁移 `last_dir`，不迁移旧 peer address/shared secret。

### T-004：安全路径、manifest 和符号链接

结论：**通过，保留后续提交层硬门**。

- `RelativePath` 拒绝绝对路径、drive/UNC、`..`、NUL、Windows 非法字符和保留设备名；
- destination resolution 会拒绝已有 symlink/non-directory ancestor；property test 验证所有接受路径只能词法解析到 root 内；
- manifest 支持多文件、目录、空目录、空文件、Unicode、重复/大小写冲突顶层名称；
- 名称冲突使用 NFC + Unicode lowercase collision key，并生成跨 Windows 可用的 rename；
- 默认保存 symlink 元数据，`follow_links` 才跟随；损坏链接和目录循环明确失败；
- `SourceSnapshot` 支持传输前重新检查常见 TOCTOU 变化；
- 生产 10,000-entry 边界有精确测试；
- unsafe/escaping link 需要确认，不支持 native link 时保存带原因和原 target 的 notice 文件；
- Windows 11 真机 native symlink 创建通过。

提交层仍必须在 T-011/T-012 使用 no-follow/capability-based 打开和原子提交；当前 ancestor 检查不能单独消除检查后替换竞态，禁止把它当作最终接收写入安全边界。

### T-005：CLI 契约

结论：**通过**。

- 已定义 `send`、`receive`、`serve`、`devices`、`config`、`update`；
- `sc`/`sc.exe` 根据 argv[0] 注入 `send`，`rc`/`rc.exe` 注入 `receive`；
- `--peer`/`--web`、路径/`--text`/`--clipboard`、HTTP/自定义证书冲突均由 clap 拒绝；
- `config set` 只接受白名单非敏感 key，不能设置 identity/private-key/token；
- parser 输出不依赖执行 adapter 的 `CommandIntent`；
- 非交互确认必须显式 `--yes`；
- 退出码固定为 0、2、3、4、5、6、7、8、9，与设计一致；
- Windows 真机验证 `sc.exe --text hello` 为 0、`rc.exe --output received` 为 0、冲突参数为 2。

## 3. 阶段二：代码质量评审

结论：**通过**。

### 结构与信息隐藏

- 协议 crate 只含 wire 数据和限制；
- core 负责配置、身份、路径和 manifest；
- platform 负责目录、权限和原子文件；
- CLI parser 与执行 intent 分离；
- core 不依赖 CLI/Web，未形成 crate 循环；
- 产品仍只有一个发布 binary：`quick-share`。

### 安全性

- 产品 crate 均使用 `#![forbid(unsafe_code)]`；Windows ACL 通过经过封装的依赖调用；
- 私钥不实现明文 Debug/Display，普通 config 命令不包含身份数据；
- trust 文件拒绝 key/ID 不一致和重复 pin；
- Web 明文必须显式 `--allow-http`，且不能和 TLS 证书参数组合；
- 未发现 panic/unwrap/todo/unimplemented 出现在生产路径。

### 质量验证

```text
cargo fmt --all -- --check                                      PASS
cargo clippy --workspace --all-targets --all-features -- -D warnings  PASS
cargo test --workspace                                         PASS (46 tests)
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu PASS
git diff --check                                               PASS
pytest -q                                                      PASS (339 tests)
```

Windows 11 真机：

- platform storage tests：4/4；
- config/identity tests：9/9（后续新增的纯校验也已通过 Windows cross-check）；
- manifest/path applicable tests：7/7；
- native symlink：Created；
- CLI shortcut/冲突退出码：通过；
- 测试二进制、ACL probe 文件和 CLI probe 目录均已清理。

## 4. 已知限制与后续硬门

| 级别 | 项目 | 处理 |
|---|---|---|
| P1（后续硬门） | 实际 receiver commit 的 TOCTOU/no-follow 安全 | T-011/T-012 必须使用 capability-based/no-follow handle，不得只复用词法检查 |
| P1（条件验证） | Windows 非管理员且关闭 Developer Mode 的 symlink fallback 真机 | 当前纯逻辑 fallback 已测；发布前能力矩阵必须补真机 |
| P1（条件验证） | macOS 真机构建、目录、symlink 语义 | CI cross-platform job 保留，发布前补 Intel/Apple Silicon 真机 |
| P2 | proptest Windows 独立复制运行时无法定位 source persistence 路径 | 测试本身通过；CI 在仓库目录运行不受影响 |

这些项目不阻塞 Batch 2，因为本批次尚未暴露网络接收写入；它们在对应 transfer/platform 任务中继续作为验收硬门。

## 5. 最终结论

T-002、T-003、T-004、T-005 均满足本批次目标并通过两阶段评审。Batch 2 可以关闭，流程应停在人工闸门，等待用户批准后再进入 Batch 3。
