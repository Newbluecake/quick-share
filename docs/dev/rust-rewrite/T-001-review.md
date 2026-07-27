---
feature: rust-rewrite
task: T-001
status: passed
review_depth: complex
reviewed_at: 2026-07-26T11:03:41+08:00
---

# T-001 两阶段评审

## 实施范围

- 根 Cargo workspace、lockfile 和 Rust 1.92 toolchain；
- 7 个规划 crate 边界；
- 唯一发布 binary `quick-share`；
- 最小 `--help`、`--version` 和缺参行为；
- CLI/Workspace contract tests；
- Linux/macOS/Windows CI check、Clippy、format、test 和 RustSec audit；
- 保留 Python baseline/release，尚未执行切换。

## TDD 证据

Red：初始空 `main()` 时 4 个 contract tests 中 3 个失败：

- `--help` 无输出；
- `--version` 无输出；
- 缺参错误码错误。

Green：使用 clap 实现最小 CLI 后 4/4 通过。

Refactor：删除各 library crate 暴露的无业务意义 `CRATE_ROLE` placeholder，只保留明确 crate 文档和依赖边界。

## 阶段 1：规范符合性评审

### 判定：通过

| T-001 要求 | 结果 | 证据 |
|---|---|---|
| Cargo workspace | 通过 | 根 `Cargo.toml`、resolver 3 |
| 7 个设计 crate | 通过 | `cargo metadata` 显示 7 packages/members |
| 单一发布 binary | 通过 | 唯一 bin target 为 `quick-share` |
| Edition/MSRV | 通过 | edition 2024、Rust 1.92.0 |
| 最小 help/version | 通过 | Linux/Windows 原生运行 |
| 格式/Clippy/test | 通过 | CI 和本地命令均配置 |
| 三平台 CI 骨架 | 通过 | ubuntu/macos/windows matrix |
| 依赖安全检查 | 通过 | `rustsec/audit-check@v2` |
| Python 暂不切换 | 通过 | release workflow 和 Python 源未修改 |

未实施后续任务的 send/receive/protocol 功能，符合 T-001 边界，无越界实现。

## 阶段 2：代码质量评审

### 判定：通过

- workspace 依赖方向无循环；
- protocol/platform 位于底层，CLI 位于组合根；
- `unsafe_code = "forbid"` 同时在 workspace 和 crate 根设置；
- Cargo.lock 已生成并用于 CI `--locked`；
- release profile 使用 thin LTO、单 codegen unit 和 strip；
- CLI tests 使用真实编译产物，不 mock parser；
- 错误路径验证真实退出码 2；
- 没有 `todo!`、`unimplemented!`、`dbg!`；
- CI permissions 为只读，未硬编码 secret；
- YAML 由本地 parser 验证；
- Windows x86_64 release 二进制约 628 KiB，可在真实 Windows 11 无 Rust 环境运行。

### 已知非阻塞项

1. 本机 PATH 中存在过时的独立 rustfmt wrapper；验证时显式使用 Rust 1.92 toolchain 的 cargo-fmt。GitHub CI 不受该本机环境问题影响。
2. Python 全量基线曾出现一次现有并发日志测试时序抖动；目标测试独立 5/5 通过，重新全量运行 339/339 通过。没有修改 Python 测试或生产代码。
3. workspace 使用 `2.0.0-alpha.0` 表示未发布 Rust 重写；它不是 release bump。T-023/T-024 发布切换时必须按项目版本清单统一唯一版本源和 CHANGELOG。
4. macOS 当前仅由 CI matrix 骨架覆盖，真实平台验证仍是 Batch 0 条件和 T-024 发布门。

## 验证记录

```text
cargo fmt --all -- --check                         PASS
cargo clippy --workspace --all-targets -D warnings PASS
cargo test --workspace --locked                    4 tests PASS
cargo build --release --locked                     PASS
cargo check --target x86_64-pc-windows-gnu         PASS
Windows 11 --version/--help/no args                 PASS
Python pytest                                      339 PASS
YAML parse                                         PASS
git diff --check                                   PASS
```

## 最终结论

T-001 完成并通过规范符合性和代码质量两阶段评审。进入 T-002 至 T-005 前需通过 Batch 1 用户闸门。
