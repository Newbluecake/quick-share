---
spike: SP-003
status: passed-policy-d
updated_at: 2026-07-26T10:45:00+08:00
---

# SP-003 Web TLS 与扫码体验

## 原型

`spikes/web-tls/`：

- rcgen 生成临时自签名证书；
- SAN 包含实际 advertise IP；
- axum-server/rustls 提供 HTTPS；
- 输出证书 SHA-256 指纹和终端二维码；
- 自动定时关闭；
- 不安装本地 CA。

## 本机验证

```bash
spikes/target/debug/qs-web-tls-spike \
  --bind 127.0.0.1:54432 \
  --advertise-ip 127.0.0.1 \
  --duration 4
```

结果：

- 普通 `curl https://...` 退出码 60：`self-signed certificate`；
- `curl --insecure` 能打开实验页面；
- 每次运行证书指纹不同；
- IP SAN 和证书指纹测试通过。

这证明 TLS 加密本身可实现，也证明零配置客户端不会信任临时证书。

## 双机网络结果

Linux `192.168.31.25` 与 Debian 13 `192.168.31.14` 两个方向均验证：

- 普通 curl 退出码 60，自签名证书不受信；
- `curl --insecure` 成功加载页面；
- SAN 包含 advertise IP，失败原因是信任链而不是名称不匹配；
- 局域网监听和防火墙路径可达。

曾在 `192.168.31.14:54435` 启动临时浏览器实验；Windows 人工验证完成后已停止服务。

## Windows 浏览器实测

用户从 Windows 浏览器访问 `https://192.168.31.14:54435/`：

- 浏览器明确显示 HTTPS/证书警告；
- 用户继续后成功看到 `Quick Share HTTPS spike reached`；
- 测试完成后已主动停止远端服务，端口不可达；
- 用户已接受策略 D；浏览器具体名称和二维码移动端仍可后补。

仍待验证：

- Android/iOS 扫码后是否允许继续；
- `curl --insecure` 是否与项目的安全定位冲突。

## 决策记录

| 策略 | 安全 | 体验 |
|---|---|---|
| 默认自签名 HTTPS | 加密 | 浏览器警告 |
| 默认 HTTP + 128-bit token | 可控制访问但可被监听 | 最顺滑 |
| 安装本地 CA | 加密且可消除警告 | 安装/权限复杂 |
| 用户提供证书 | 最适合管理环境 | 普通用户不可用 |

## Windows 原生服务端与防火墙

Windows 11 x86_64 原生 `qs-web-tls-spike.exe`：

- 本机 `https://127.0.0.1` 可正常访问，证明 rustls/axum Windows server 可用；
- Linux 访问 Windows `192.168.32.38:54446` 超时；
- Windows 当前网络类别为 `Public`、防火墙启用、没有 Quick Share inbound rule；
- 同一 Windows 机器上的 Noise server 对 Linux inbound 也超时，说明问题位于 Windows 入站策略而非 HTTPS；
- 产品必须检测网络类别/入站不可达，提供明确诊断；不得静默关闭防火墙；
- 安装器只能在用户明确同意并具备权限时添加最小范围规则，优先限制 Private profile 和程序路径。

用户选择策略 D，SP-003 判定 Go：

- 默认自签名 HTTPS；
- 支持配置用户自己的受信任证书；
- HTTP 仍必须显式 `--allow-http`；
- 接受默认首次浏览器访问出现证书警告的体验成本；
- 不安装实验 CA。
