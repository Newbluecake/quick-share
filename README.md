# Quick Share

快速启动HTTP服务分享单个文件，自动生成适合 curl/wget 的下载链接。

## 功能特性

- 🚀 **一键启动**: 单个命令即可启动HTTP服务并分享文件
- 🌐 **自动检测IP**: 自动检测局域网IP地址，无需手动查找
- 🔌 **智能端口**: 默认8000端口，冲突时自动递增
- 🔒 **安全限制**: 仅允许下载指定文件，防止路径遍历攻击
- 📋 **便捷复制**: 自动生成 curl/wget 下载命令
- ⏱️ **自动停止**: 支持下载次数和运行时长限制
- 📊 **实时日志**: 显示下载进度和访问记录

## 快速开始

```bash
# 分享文件（默认配置：10次下载或5分钟后自动停止）
quick-share /path/to/file.zip

# 自定义配置
quick-share file.zip -n 3 -t 10m -p 9000
```

## 使用场景

- 服务器间快速传输文件
- 临时分享日志文件给同事
- 快速分享构建产物
- 局域网内设备间文件共享

## 开发状态

当前项目处于需求澄清阶段，已完成：

- ✅ 需求简报 (Brief)
- ✅ 完整需求文档 (Requirements)

## 下一步

查看完整需求文档：
- [Quick Share - 需求简报](docs/dev/quick-share/quick-share-brief.md)
- [Quick Share - 需求文档](docs/dev/quick-share/quick-share-requirements.md)

开始实施开发：
```bash
/dev:spec-dev quick-share --skip-requirements
```

## License

MIT
