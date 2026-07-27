---
spike: SP-002
status: linux-windows-runtime-passed-conditional-go
updated_at: 2026-07-26T10:45:00+08:00
---

# SP-002 mDNS 与跨机器发现

## 原型

`spikes/discovery/`，使用 `mdns-sd 0.20.2` 和 `_quickshare._tcp.local.`。

```bash
# 机器 A
cargo run --manifest-path spikes/Cargo.toml -p qs-discovery-spike -- \
  advertise --name machine-a --port 53317 --duration 120

# 机器 B
cargo run --manifest-path spikes/Cargo.toml -p qs-discovery-spike -- \
  scan --timeout 10 --expect-id spike-machine-a
```

## 本机结果

同机 advertiser/scanner：

- 找到 1 个目标服务；
- 首次 resolved event 约 21ms；
- TXT `id/name/ver/caps` 正确；
- 两个输入清洗测试通过；
- Linux 与 Windows GNU target 均通过 `cargo check`。

## 重要发现

`ServiceInfo::enable_addr_auto()` 在当前含 Docker、bridge、veth、VPN/Meta 接口的机器上为一个服务返回了 **126 个地址**，包括大量带错误/无意义 scope 组合的 IPv6 link-local 地址和虚拟网段。

因此生产实现不能直接采用“自动发布全部地址”：

- 需要显式枚举和过滤接口；
- 应按可用 LAN 接口发布可达地址；
- 发现端需要对 link-local scope、虚拟接口和重复地址去重；
- 不能把 mDNS 返回地址直接视为可信或可达；
- 必须对候选地址执行受限连接验证。

## 待验证矩阵

| 组合 | 状态 |
|---|---|
| Linux → Linux 同机 | 通过 |
| Linux `192.168.31.25` ↔ Debian 13 `192.168.31.14` | 双向通过：约 408ms / 1069ms |
| Windows 11 x86_64 native | 原生 `.exe` 运行通过；同机 advertise/scan 约 69ms |
| Windows ↔ Linux 跨 `/24` | 未发现，符合 mDNS 不跨路由边界的预期 |
| macOS build/run | 待验证 |
| 防火墙静默丢包 | 待验证 |
| VPN/多网卡 | 当前机器发现地址污染已复现 |
| Wi-Fi 客户端隔离 | 待真实网络验证 |

## 双机结果

- Debian advertiser → 本机 scanner：408ms，正确解析 `192.168.31.14`；
- 本机 advertiser → Debian scanner：1069ms，正确解析 `192.168.31.25`；
- 两个方向均通过 expected device ID 检查；
- 无需在远端安装 Rust，glibc release 单文件可直接运行；
- 每个方向仍返回 11–14 个无意义 IPv6 link-local 候选，进一步确认接口过滤是 P0；
- Windows 自动地址模式返回 VMware `192.168.240.1`，反而遗漏主地址 `192.168.32.38`；
- 原型新增显式 `--ip` 后，Linux 只发布 `192.168.31.25`，Windows 只发布 `192.168.32.38`，两个平台均精确返回单一地址；
- Windows 与 Linux 位于不同 `/24` 时 mDNS 不跨路由发现，但 `--peer`/Noise 直连成功。

## 当前判定

Conditional Go。Linux 双机与 Windows 原生 mDNS 已证明库可运行；生产实现禁止 `enable_addr_auto()`，必须枚举接口并为每个获选 LAN 地址显式注册。mDNS 只承诺同一链路，跨子网必须使用 `--peer`。Windows↔Linux 同链路以及 macOS 运行验证仍保留为平台闸门。
