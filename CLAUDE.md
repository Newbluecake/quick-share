# Quick Share - Development Notes

## Version Bump Checklist

发布新版本时，以下文件必须同步更新版本号：

| 文件 | 位置 | 说明 |
|------|------|------|
| `src/__init__.py` | `__version__ = "x.x.x"` | 运行时版本，`--version` 命令读取此处 |
| `setup.py` | `version="x.x.x"` | pip 安装包版本 |
| `CHANGELOG.md` | 新增版本节 | 记录变更内容 |

> **注意**：`src/__init__.py` 和 `setup.py` 必须保持一致，否则 pip 安装的版本与 `--version` 显示的版本会不同。
