# AI Commit for OpenIDEA

> Generates a Conventional-style Chinese commit message from the files you
> checked in the Commit tool window, using DeepSeek (or any OpenAI-compatible
> endpoint). It is a pure Wasm plugin — no IDE source changes. Requires
> OpenIDEA **0.1.39+** (plugin API 1.3, commit extension point) and a trusted
> project.

在 Commit 面板勾选文件 → 点提交消息上方的 **AI Commit** 按钮 → 模型生成
「标题 + 空行 + 分条详情」的中文提交信息 → 自动填入提交消息框 → 你检查后点 IDE 自带的 Commit。
插件**不自动提交**。

使用官方插件 API 1.3 的 `commit.pending-commit` / `set-commit-message` 与
`commit.message` 菜单扩展点（上游 PR #1941）。

## 安装 Install

1. 从 [Releases](https://github.com/yokiweb/openidea-ai-commit/releases) 下载
   `yoki.ai-commit-0.1.0.oiplugin`。
2. Settings › Plugins › **Install Plugin from Package File…**，选择该文件。
3. 确认权限页后启用；在 Settings › Plugins 里填 **DeepSeek API Key**。

也可以 clone 本仓库，用 **Install Plugin from Disk…** 选择仓库目录
（目录内含 `plugin.toml` + `dist/plugin.wasm`，适合本地调试）。

包由 publisher `yoki` 的 Ed25519 密钥签名，公钥：

```
B6us5d/AvCxkbIxX0sgmAnuj1JGsbYfwWYmtJJYXZV8=
```

```sh
sha256sum yoki.ai-commit-0.1.0.oiplugin
# 02265dfb36773c51de3d1083024b0f69b2c183e23735ee5f043b70e7e4207b20
openidea-plugin check yoki.ai-commit-0.1.0.oiplugin
```

私钥不在此仓库；任何改动包内文件都会使签名失效，IDE 会拒绝安装。

## 配置 Settings

| 项 | 默认 | 说明 |
|---|---|---|
| DeepSeek API Key | 无 | 存系统 keychain，插件读不到明文 |
| `endpoint` | `https://api.deepseek.com/chat/completions` | 任何 OpenAI-compatible 端点 |
| `model` | `deepseek-chat` | 模型名 |
| `language` | `zh` | 可切 `en` |
| `maxDiffChars` | `48000` | 送模型的 diff 上限（字符） |
| `systemPrompt` | 空 | 空 = 内置中文规范化提示词 |

换用自建/其他服务时，改 `endpoint` 并把对应的 `net:<host>` 加入授权，否则出网请求会被 IDE 拒绝。

## 权限与理由 Permissions and why

| Capability | 级别 | 为什么需要 |
|---|---|---|
| `vcs.read` | required | `pending-commit()` 返回本次提交**已勾选**文件与改动块的 diff（IDE 侧 1 MiB 上限）；未勾选文件不会进入请求。 |
| `vcs.write` | required | 仅用于 `set-commit-message()` 把生成的文字写回消息框，**不执行 commit**；带 revision 保护，你在此期间改过草稿就不会覆盖。 |
| `secrets` | required | API Key 存在系统 keychain，由 IDE 在出网时以 `{{secret:api-key}}` 替换 Bearer token；插件代码读不到明文。 |
| `net:api.deepseek.com` | optional | 唯一出网目标：`POST /chat/completions`。不授权则功能不可用；错误响应体不回显（可能含代码/密钥）。 |

All requests are plain JSON over HTTPS to the configured endpoint; no
telemetry, no file upload, no process execution, no storage access.

## 数据流与安全

- diff 只含已勾选改动块；二进制文件只记路径。
- 长请求通过 `host::begin_task` 提到 60 秒，可在后台任务里取消；取消后不写回。
- `set_commit_message(revision, text)` 带 Stale 保护，不会覆盖你手写的文字。
- 未配置 key、无仓库、未勾文件、只有二进制改动时不发请求，直接提示。

## 构建 Build

SDK 随仓库附带（`vendor/openidea-plugin/`，来源见 `NOTICE.md`），无需联网取上游仓库。

```sh
rustup target add wasm32-wasip2
cargo test                                  # 13 个纯函数 + MockHost 单测
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/openidea_plugin_ai_commit.wasm dist/plugin.wasm
openidea-plugin check .
openidea-plugin pack . --key <your.key> --publisher <your-publisher-id>
```

`openidea-plugin` 来自 OpenIDEA 的 release 附件，或
`cargo install --locked --git https://github.com/l0ng-ai/openidea ide-host --bin openidea-plugin`。

## 已知限制

- 需要 OpenIDEA 0.1.39+（插件 API 1.3）；旧版 IDE 无 commit 扩展点。
- 只对 Commit 工具窗口 / Commit 对话框的「已勾选」改动生效。
- 不自动提交；生成结果始终由你检查后提交。
- 真机交互（IDE 内点击按钮）尚未做端到端实测；单元测试与
  `openidea-plugin check`（包级、组件导入/导出校验）已通过。

## 许可

Apache-2.0，见 `LICENSE`。随附 SDK 的来源见 `NOTICE.md`。
