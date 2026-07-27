# Quick Share - Development Notes

## Version Bump Checklist

Quick Share 2 is a Rust-only workspace. `Cargo.toml` under `[workspace.package]` is the single version source; every product crate inherits it with `version.workspace = true`.

When preparing a release:

| File | Location | Purpose |
|---|---|---|
| `Cargo.toml` | `[workspace.package].version` | The only runtime/package version |
| `Cargo.lock` | resolved workspace packages | Regenerated and committed after the version change |
| `CHANGELOG.md` | new release section | User-visible changes and migration notes |

Do not recreate Python version files or duplicate the version in scripts. Release workflows derive the asset version from the immutable Git tag and verify that the binary reports the Cargo workspace version.

A final `v2.0.0` tag requires the T-024 release acceptance approval. Do not bypass the `snow 0.10.0` security-review and macOS true-host gates.
