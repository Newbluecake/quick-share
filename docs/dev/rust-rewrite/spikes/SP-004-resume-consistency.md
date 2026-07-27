---
spike: SP-004
status: linux-windows-kill-resume-passed
updated_at: 2026-07-26T10:45:00+08:00
---

# SP-004 分块恢复与崩溃一致性

## 原型

`spikes/resume/` 实现单文件最小模型：

- 4 MiB 默认块；
- BLAKE3 块摘要；
- offset 写入；
- 数据 `sync_data` 后才记录完成；
- journal 使用同目录 tempfile + sync + rename；
- final 完整摘要通过后才 rename；
- 损坏 journal 失败关闭。

## 自动化测试

7 个测试通过：

1. 完成前 final path 不可见；
2. 数据同步后、journal 更新前崩溃可通过重写恢复；
3. journal rename 前崩溃保留旧有效状态；
4. 乱序块、重启、补传和 final digest；
5. 重复相同块幂等、冲突块拒绝；
6. 截断 journal 不暴露 final；
7. 分块生成内容与连续 payload 的最终摘要一致。

## 32 MiB 实验

```text
Injected failure: AfterDataSync
Missing after reopen: [0,1,2,3,4,5,6,7]
PASS: resumed and verified 33554432 bytes
```

## 真实进程终止结果

Linux x86_64：

- 256 MiB、64 块任务在提交 5 块后 `kill -9`，退出状态 137；
- final path 不存在，journal 正确记录 5/64；
- 重启只补传 59 块；
- 最终文件 268,435,456 bytes，BLAKE3 通过。

Windows 11 x86_64：

- 使用 `Stop-Process -Force` 在 5/64 块后终止；
- 重启补传并完成 256 MiB，exit code 0；
- Windows tempfile persist、journal replace 和 final rename 可用；
- 包含空格和中文的目标目录重复执行同样 kill/resume 测试通过。

## 当前结论

文件级方案 Conditional Go。后续正式实现仍必须补充：

- Windows 正在被其他程序打开时的 rename/file lock；
- 目录多文件提交；
- 磁盘满与 fsync 失败；
- journal 与 `.part` 不一致时的 reconciliation；
- 1 GiB/10 GiB 和大量小文件；
- 真正断电无法由进程测试完全模拟，仍需依赖持久化顺序和恢复校验。

明确边界：可保证单个未完成文件不会伪装成 final；不能保证整个目录任务在所有平台整体原子出现。
