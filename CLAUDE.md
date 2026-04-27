# Quick Share - Development Notes

## Version Bump Checklist

发布新版本时，以下文件必须同步更新版本号：

| 文件 | 位置 | 说明 |
|------|------|------|
| `src/__init__.py` | `__version__ = "x.x.x"` | 运行时版本，`--version` 命令读取此处 |
| `setup.py` | `version="x.x.x"` | pip 安装包版本 |
| `pyproject.toml` | `version = "x.x.x"` | 现代构建配置版本 |
| `CHANGELOG.md` | 新增版本节 | 记录变更内容 |

> **注意**：`src/__init__.py`、`setup.py` 和 `pyproject.toml` 中的版本号必须保持一致。
