# Batch 3 两阶段评审：T-006 至 T-008

> 日期：2026-07-26
> 范围：mDNS 发现、本地 staging/恢复、剪贴板与文本降级
> 结论：**通过，允许进入下一人工闸门**

## 1. 评审范围

- `crates/quick-share-discovery/`
- `crates/quick-share-transfer/`
- `crates/quick-share-platform/`
- `crates/quick-share-core/src/paths.rs`
- workspace dependency/lockfile 变更

本批次仍不实现 Noise 会话、offer 状态机或端到端设备直传。

## 2. 阶段一：规范符合性评审

### T-006：mDNS 广播、扫描、去重和错误分类

结论：**通过**。

- 实现异步 `Discovery` trait、生产 `MdnsDiscovery`、RAII `MdnsRegistration` 和队列式 `FakeDiscovery`；
- TXT 固定为 `id/name/ver/port/fp/caps` 六个字段，拒绝未知、缺失、超长、非法版本、非法 fingerprint 和未知 capability；
- 设备名移除控制字符，并同时限制 Unicode scalar、TXT byte 和 DNS label 长度；
- 显式枚举 `if-addrs`，过滤 down、loopback、unspecified、multicast、link-local、P2P 和常见虚拟接口；
- 生产 `ServiceInfo` 使用显式 IP slice，回归测试确认 `is_addr_auto() == false`；
- 排除自身和 protocol major 不兼容服务；按 `DeviceId` 合并多地址，拒绝同 ID 不同 fingerprint；
- 所有 peer 均保持 `unverified = true`，等待 Batch 4 Noise/INFO 验证；
- `ScanResult.complete` 明确区分“扫描成功且零设备”与 partial/error，只有 `is_definitive_empty()` 可触发后续 Web fallback；
- backend/interface/channel 失败带 `--peer host:port` 操作建议；
- Linux 和 Windows 11 真机均完成生产 registration + scan 自发现测试，默认 3 秒测试窗口通过。

### T-007：staging、块存储、原子提交和恢复日志

结论：**通过**。

- staging 固定位于 `<output>/.quick-share-staging/<transfer-id>/`，与最终目录处于同一文件系统；
- 实现 `TransferStore`、entry-scoped `ChunkWriter` 和持久 `ResumeJournal`；
- manifest 持久绑定 `TransferId` 和 sender `DeviceId`，错误 sender 无法恢复；
- `.part` 使用稀疏预分配和平台 offset I/O，内存不随文件大小线性增长；
- chunk 在长度、offset、entry、transfer、BLAKE3 全部通过后写入；先 `sync_data`，再原子更新 journal；
- 重复相同 chunk 幂等，不同 digest 冲突失败；
- journal/file/chunk 总数和持久状态 byte size 均有硬限制，避免恶意状态导致无界分配；
- 完整 BLAKE3 通过前 final path 不可见；
- conflict `rename/error/skip/overwrite/ask` 复用安全路径策略，overwrite 使用跨平台 atomic replace；
- commit intent 在 rename 前持久化；rename 后崩溃可通过 final digest 对账；
- rename 目标发生竞态时会重新执行冲突决策，不会永久卡在旧 intent；
- storage-full、权限不足、data sync 后故障、journal replace 前后故障、commit intent 后故障和 final rename 后故障均有测试；
- 空文件、乱序块、4 GiB 以上稀疏文件、完整摘要错误和 Windows replace 均通过；
- staging 只提供查询和显式 `cleanup_if_complete`，不会自动删除可恢复数据。

### T-008：剪贴板与文本安全降级

结论：**通过**。

- 实现可注入 `Clipboard` trait 和 `NativeClipboard` arboard adapter；
- arboard 关闭默认 image feature，仅启用 Wayland data-control；
- Linux 无 `DISPLAY`/`WAYLAND_DISPLAY` 时在启动 backend 前返回结构化、可操作的 `Unavailable`；
- backend error 移除控制字符并限制长度；
- `TextPayload` 限制为 1 MiB，Debug 永远输出 `[REDACTED]`，summary 只含来源、长度和短 digest；
- clipboard write 失败时，优先写入显式 `--output` 语义文件，否则原样写 stdout；
- output 使用 no-clobber 原子创建，不静默覆盖；
- 测试证明 shell-like 文本只作为字节输出，不执行、不打开；
- Windows 11 Unicode clipboard round-trip 真机通过；Linux headless 降级通过。

## 3. 阶段二：代码质量评审

结论：**通过**。

### 架构与边界

- discovery crate 只产生不可信发现 hint，不依赖 transfer；
- platform crate 只封装接口、目录、权限、原子文件和 clipboard；
- transfer crate 负责本地 staging、文本 payload 和接收降级，不将网络逻辑混入存储；
- core 继续保持 CLI/Web 无关；
- 所有正式 crate 保持 `#![forbid(unsafe_code)]`。

### 安全性

- 禁止 `enable_addr_auto()` 的行为由结构测试固定；
- TXT、journal、manifest、chunk 和 text 均有显式资源上限；
- final commit 前执行 block digest 和 full digest 两层校验；
- resume 同时绑定 transfer 和 sender identity；
- journal 状态损坏、final digest 不匹配或 commit 对账不一致时 fail closed；
- text 和 clipboard backend 错误不会进入敏感 Debug；
- RustSec 对 189 个 lockfile dependencies 扫描通过，无已知 advisory。

## 4. 验证结果

```text
cargo fmt --all -- --check                                      PASS
cargo clippy --workspace --all-targets --all-features -- -D warnings  PASS
cargo test --workspace                                         PASS
  69 tests listed; 68 default-pass + 1 true-host mDNS ignored
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu PASS
cargo audit                                                    PASS
pytest -q                                                      PASS (339 tests)
git diff --check                                               PASS
```

额外真机：

- Linux production mDNS registration + scan：PASS；
- Windows production mDNS registration + scan：PASS；
- Windows staging/offset write/crash recovery/atomic overwrite：9/9；
- Windows fake text delivery：4/4；
- Windows native Unicode clipboard：PASS；
- Linux headless clipboard actionable fallback：PASS；
- Linux release build：PASS；当前 `ldd` 仅显示 libc/libgcc，无图形系统动态库；T-016 真正接线后必须重复该检查；
- Windows 临时测试目录均已清理。

## 5. 已知限制与后续硬门

| 级别 | 项目 | 后续处理 |
|---|---|---|
| P1（条件验证） | Linux X11 与 Wayland 真桌面 clipboard | T-008 adapter 已完成；发布前及 T-016 集成时分别补真机 |
| P1（条件验证） | macOS Intel/Apple Silicon mDNS、staging、clipboard 真机 | CI 平台 job 保留，发布前补真机矩阵 |
| P1（条件验证） | Windows↔Linux 同一链路 production mDNS | 当前可用机器处于不同子网；已有各平台本机和 SP 双机证据，T-010/T-024 补同链路 |
| P1（后续硬门） | 实际 receiver 多 entry、directory、symlink commit 的 no-follow handle | T-011 连接 offer/manifest 时完成，不能只依赖路径预检查 |
| P2 | Windows firewall 静默丢弃 multicast 无法仅靠 mDNS socket 区分零设备 | T-014 增加 network profile/firewall 诊断；显式 backend 错误已不会伪装为零结果 |
| P2 | X11 clipboard ownership 依赖进程生命周期/clipboard manager | T-016 receiver 必须让 adapter 生命周期覆盖交付提示，并验证进程退出行为 |

## 6. 最终结论

T-006、T-007、T-008 满足 Batch 3 的本地能力目标，并通过两阶段评审。流程应停在人工闸门；用户批准后才能进入 Batch 4（T-009 Noise/SAS/pinning、T-010 offer/确认/可信状态机）。
