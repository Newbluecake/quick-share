# `snow 0.10.0` 安全审计适用性证据

> 评估日期：2026-07-27
> Quick Share 使用方式：`Noise_XX_25519_ChaChaPoly_BLAKE2s`，`snow = 0.10.0` 默认纯 Rust resolver
> 判定：**满足 T-024 第三方审计硬门；无未处理高危问题，保留一个已知 informational 内存残留风险**

## 1. 独立审计来源

1Password/AgileBits 委托 Trail of Bits 对 `snow` 做了四个 engineer-weeks 的独立安全评估：

- 审计报告：<https://github.com/trailofbits/publications/blob/master/reviews/2024-03-agilebits-snow-securityreview.pdf>
- Trail of Bits publications repository：<https://github.com/trailofbits/publications>
- 1Password 发布说明：<https://1password.com/blog/strengthening-snow-for-the-open-source-community>
- 上游把报告加入 README 的 PR：<https://github.com/mcginty/snow/pull/211>
- 下载报告 SHA-256：`d2ee52a3ffed85a382201a353f844df21ee7ebeafe5f6539f215469baa8bd433`

审计目标 commit：`009411d2035a77ece57b0a6873169de7693a0aa0`。审计覆盖 Noise specification compliance、cryptographic primitive use、API safety、state transitions、DoS、input validation、error states及平台风险，使用人工评审、静态和动态测试。

审计共报告 10 项：0 high、1 medium、1 low、8 informational。2024-03 fix review 确认 8/10 resolved。

## 2. `snow 0.10.0` 修复继承验证

Quick Share 锁定的 `v0.10.0` tag commit 为：

```text
4bb43f50370bdb3e8b1b57814ac662864db2704f
```

以下 Trail of Bits 确认的修复 commit 均为 `v0.10.0` 的 ancestor（用 `git merge-base --is-ancestor <fix> v0.10.0`逐项验证）：

| Finding | Severity | 修复 commit | v0.10.0包含 |
|---|---:|---|---|
| TOB-SNOW-1 deprecated sodiumoxide | Informational | `87b7f608e4c7f60b397bcfb88df73f2579a0bc4f` | 是 |
| TOB-SNOW-3 oversized received message | Informational | `c6657a7cadaa6e67cd636d6ed2c04dd354157d94` | 是 |
| TOB-SNOW-4 protocol name trailing data | Informational | `8161a5fe2fcd8c006874217359074e5e0bb181d4` | 是 |
| TOB-SNOW-5 invalid PSK index panic | Low | `270c9fd388a7cd8c9d12e114b13e04832c9bf858` | 是 |
| TOB-SNOW-6 builder overwrite | Informational | `23bc38f054542903889d3de06a849843daadc593` | 是 |
| TOB-SNOW-7 nonce advance after failed authentication | Medium | `5b451e9507b6a70d762600cda40bfd9a7428e093` | 是 |
| TOB-SNOW-9 repeated PSK modifier | Informational | `671e0b9699e140c55d6f33d3d507338b65892c2` | 是 |
| TOB-SNOW-10 inconsistent key lengths | Informational | `357fbac3f3894ebe1bbbf66d7e2271b418c58b6d` | 是 |

公开 GitHub advisory `GHSA-7g9j-g5jg-3vv3` 对应 TOB-SNOW-7，影响 `< 0.9.5`，`0.9.5` 已修复；Quick Share 使用 `0.10.0`。`cargo audit` 也不报告该版本。

## 3. 两个 unresolved informational finding

### TOB-SNOW-2：PSK + pre-message ephemeral

不适用于 Quick Share：

- finding只影响包含 PSK和pre-message ephemeral key的非标准/fallback pattern；
- 报告明确说明当时Snow不支持会触发该行为的pattern；
- Quick Share固定为标准 `XX`，不使用PSK、fallback modifier、custom pattern或运行时suite negotiation；
- protocol name是编译期固定常量，不能由peer输入。

### TOB-SNOW-8：ephemeral keys未主动zeroize

适用但评级为 informational，作为残余风险接受：

- 获得root/physical access并读取进程旧内存的攻击者，理论上可能恢复未zeroize的ephemeral material，削弱历史会话forward secrecy；
- 这不允许仅位于LAN的攻击者窃听、伪造Noise identity、绕过SAS/pinning或修改ciphertext；
- Quick Share不能从外部可靠补丁Snow内部handshake allocations，且自行fork cryptographic implementation会增加更高风险；
- 产品已避免把keys写日志，identity file使用private ACL/mode，session错误立即关闭，record/frame严格有界；
- 后续升级Snow时继续跟踪upstream zeroization；若威胁模型扩展到本机root/physical-memory adversary，应重新设为发布硬门。

## 4. Quick Share 补充验证

Quick Share不是只依赖库审计。产品测试另覆盖：

- fixed XX static-key authentication和full-key pinning；
- transcript-bound 6-digit SAS及MITM endpoints不同SAS；
- wrong static pin、ciphertext tamper/truncation/replay和handshake replay fail closed；
- operation wall-clock deadline、oversized record和8 MiB logical frame bound；
- coordinated rekey及uncoordinated key failure；
- malformed arbitrary records fuzz，无panic/OOM；
- authorization bearer绑定peer、transfer、manifest、permission和expiry；
- Noise/framing错误后session永久关闭，不在同一state继续处理。

## 5. 发布判定

Trail of Bits正式审计和fix review解决了原先“`snow`没有第三方审计证据”的阻塞事实。`snow 0.10.0`包含全部medium/low修复；剩余两个均为informational，其中TOB-SNOW-2不适用固定XX，TOB-SNOW-8是明确记录的本机高权限内存残余风险。

因此，T-024可在以下条件下通过此安全门：

1. 保持exact `snow 0.10.0` lock和RustSec无advisory；
2. 不增加PSK、fallback/custom pattern或suite negotiation；
3. 保留现有adversarial Noise tests和fuzz；
4. 跟踪upstream TOB-SNOW-8 zeroization进展。
