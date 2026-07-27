---
spike: SP-005
status: linux-windows-passed-macos-pending
updated_at: 2026-07-26T10:45:00+08:00
---

# SP-005 剪贴板与单文件构建矩阵

## 原型

`spikes/clipboard/` 使用 `arboard 3.6.1`：

- 关闭默认 image-data feature；
- 开启 Wayland data-control；
- 只测试文本；
- 输出 JSON 能力结果；
- 错误清除终端控制字符并限制长度；
- 失败时明确建议 stdout 或 `--output`。

## 当前结果

Linux x86_64 headless：

```json
{
  "display": null,
  "wayland_display": null,
  "clipboard_available": false,
  "error": "X11 server connection timed out because it was unreachable",
  "fallback": "stdout or --output file"
}
```

结论：headless 失败可检测，不会阻止核心程序运行。相同 release 二进制已在 Debian 13 x86_64 第二台机器直接运行，得到同样的结构化降级结果。

构建：

- Linux x86_64：通过；
- Windows 11 x86_64：交叉编译的原生 `.exe` 无 Rust 运行时直接运行；
- Windows clipboard create/write/read round-trip 通过；
- Windows 中文 + emoji UTF-8 文本 round-trip 通过；
- `ldd` 只显示 libc/libgcc，没有 X11 动态库；X11 使用 x11rb；
- Wayland feature 引入额外纯 Rust/protocol 依赖，实际 Wayland 运行待验证；
- macOS build/run 待验证。

当前 debug 二进制较大，不代表 release 产物：初次 debug build 约 47 MB。最终需要 release + strip + feature 审计。

## 待验证

- X11 实际 read/write；
- Wayland 实际 read/write；
- 写入后进程退出时内容生命周期；
- macOS 构建和运行；
- musl build；
- release binary size。

当前判定：Linux headless 降级和 Windows 原生剪贴板 Go；Linux X11/Wayland 与 macOS 仍 Conditional。剪贴板失败不得阻塞文件传输。
