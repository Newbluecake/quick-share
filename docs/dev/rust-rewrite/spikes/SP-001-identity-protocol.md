---
spike: SP-001
status: linux-windows-noise-passed-selection-proposed
updated_at: 2026-07-26T10:34:25+08:00
---

# SP-001 身份协议安全原型

## 目标

比较自签名 mTLS、TLS + 应用签名、Noise XX，验证首次握手、SAS、设备公钥固定和中间人检测。

## 当前环境

- Linux x86_64
- Rust 1.92.0
- `snow 0.10.0`，MSRV 1.85，MIT OR Apache-2.0
- Noise pattern：`Noise_XX_25519_ChaChaPoly_BLAKE2s`

## 已完成证据

原型：`spikes/identity/`

```bash
cargo test --manifest-path spikes/Cargo.toml -p qs-identity-spike
cargo run --manifest-path spikes/Cargo.toml -p qs-identity-spike -- --check-mitm
```

结果：

- Noise XX 双方握手成功；
- 双方 handshake hash 派生的 6 位 SAS 一致；
- 双方均取得对方长期 static public key；
- responder identity 被替换时，已有 pin 不匹配；
- 终止式 MITM 建立两条独立 Noise 会话时，真实两端 SAS 不一致；
- 5 个自动化测试通过（含 bounded TCP frame）；
- Linux x86_64 双机 Noise XX 握手和加密 ping/pong 通过；
- 双机持久化 key 重启后 public key pin 保持不变；
- 双机配置错误 pin 时客户端失败关闭，服务端观察到连接中止；
- Windows 11 x86_64 原生二进制本机 Noise/MITM 测试通过；
- Windows 11 → Debian 13 跨子网 Noise 握手、SAS、加密 ping 和重启 pinning 通过。

一次实测：

```text
initiator SAS = responder SAS = 895100
pinning_succeeded = true
MITM endpoint SAS = 551155 / 035588
```

六位 SAS 理论上存在 1/1,000,000 碰撞概率，因此它是人工比较码，不是长期 identity。长期信任绑定完整 static public key。

## 候选对比（当前）

| 方案 | 优点 | 当前阻碍 |
|---|---|---|
| 自签名 mTLS | HTTP/2 和成熟 TLS 流控 | Axum/axum-server 的 peer cert 提取需要自定义 acceptor；未知证书受限接入需要高风险 verifier |
| TLS + 应用签名 | 服务端 HTTPS 普通；API 调试方便 | canonical request、nonce、时钟窗口和重放防护需要自行正确组合 |
| Noise XX | 原生适合双方首次未知 static key；SAS/pinning 直接 | 需要自定义有界 framing；尚未找到可引用的独立安全审计；不能直接复用浏览器/HTTP2 |

参考：

- https://docs.rs/snow/
- https://github.com/mcginty/snow
- https://docs.rs/axum-server-mtls

## 双机结果

环境：

- 发起端：Linux x86_64，`192.168.31.25`；
- 响应端：Debian 13 x86_64，`192.168.31.14`；
- Noise TCP 端口：54441–54443。

首次握手：

```text
server local static = 659cced7...24fa3a
client local static = a1f40741...b2c82f
server SAS = client SAS = 946806
encrypted ping/pong = OK
```

持久 key 重启后：双方 static key 不变、SAS 新鲜变化、正确 pin 通过。错误 pin 使用全零 32-byte 值时客户端返回 `remote identity pin mismatch`，没有发送应用数据。

## Windows ↔ Linux 结果

- Windows 11 Enterprise x86_64：原生 `.exe` 无 Rust 运行时直接运行；
- Windows client → Debian server：SAS `514377`，加密 ping/pong 通过；
- 使用两端持久 key 重启：SAS `517628`，public key pin 保持不变；
- Windows 剪贴板和 Noise 测试均不需要管理员权限；
- 两台机器位于不同 `/24`，直接 TCP 路由可达，不影响 Noise。

## 建议结论

建议 **Conditional Go：选择 Noise XX 作为设备直传安全握手与传输层**。原因是它最符合双方首次未知 static key 的配对语义，Linux/Windows 双机证据已通过，且比“未知自签名 mTLS verifier”或“自定义 canonical request 签名”暴露更少的自定义认证组合。

必须保留的条件：

1. `snow` README 明确说明尚未接受正式安全审计，因此发布前必须进行独立安全评审；
2. 生产代码只使用官方 Noise pattern，不发明密码原语；
3. 继续补 framing 截断、重放、超时、rekey 和 fuzz 测试；
4. static private key 必须使用安全原子存储和权限；
5. 生产协议通过 Noise 有界 frame 承载，不直接复制 Spike；
6. `snow 0.10.0` 锁定的 `aes-gcm` 为 0.10.3，持续执行 RustSec 审计；
7. 传统 Web 仍单独使用 rustls HTTPS。

维护调研：snow 2026-03 仍有依赖和功能提交，crate `unsafe_code = "forbid"`，但“活跃维护”和“无 unsafe”不能替代正式密码学审计。
