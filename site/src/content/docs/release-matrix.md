---
title: Release Matrix
description: Release artifact targets and download URL conventions.
---

# DbGraph Release Matrix

DbGraph publishes one prebuilt archive per supported Rust target triple.
All artifacts use an unambiguous versioned name:

```text
dbgraph-v{version}-{target}.{tar.gz|zip}
dbgraph-v{version}-{target}.{tar.gz|zip}.sha256
```

`latest` may be resolved through the GitHub release redirect or API, but pinned
downloads should use a `vX.Y.Z` tag.

| OS | Arch | Target | Archive | GitHub Actions runner |
|---|---|---|---|---|
| macOS | x64 | `x86_64-apple-darwin` | `tar.gz` | `macos-15-intel` |
| macOS | arm64 | `aarch64-apple-darwin` | `tar.gz` | `macos-latest` |
| Linux | x64 | `x86_64-unknown-linux-gnu` | `tar.gz` | `ubuntu-latest` |
| Linux | arm64 | `aarch64-unknown-linux-gnu` | `tar.gz` | `ubuntu-latest` (cross-build) |
| Windows | x64 | `x86_64-pc-windows-msvc` | `zip` | `windows-latest` |
| Windows | arm64 | `aarch64-pc-windows-msvc` | `zip` | `windows-latest` (cross-build) |

Download URL convention:

```text
https://github.com/zaenzhang/dbgraph/releases/download/v{version}/dbgraph-v{version}-{target}.{ext}
https://github.com/zaenzhang/dbgraph/releases/download/v{version}/dbgraph-v{version}-{target}.{ext}.sha256
```

Installers and npm wrappers must derive URLs from this matrix rather than
maintaining separate platform-specific naming rules.

The tag-triggered release workflow builds all six targets. The macOS x64 job
uses an Intel runner explicitly; `macos-latest` is the ARM64 build. A successful
build establishes archive availability, while installation and runtime checks
on each platform are separate verification steps. See the asset list on the
specific [GitHub release](https://github.com/zaenzhang/dbgraph/releases).
