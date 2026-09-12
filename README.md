# KiiChat

[![ci](https://github.com/luzov/KiiChat/actions/workflows/ci.yml/badge.svg)](https://github.com/luzov/KiiChat/actions/workflows/ci.yml)

一个轻量的、Cherry Studio 风格的桌面大模型聊天客户端，使用 [GPUI](https://gpui.rs)（Zed 的 GPU 加速 UI 框架）编写：添加任意 OpenAI 兼容的接口，一键拉取模型列表，然后直接开聊。

![浅色模式](docs/screenshot-light.png)

<details>
<summary>深色模式</summary>

![深色模式](docs/screenshot-dark.png)

</details>

<details>
<summary>设置 · 模型</summary>

![设置页](docs/screenshot-models.png)

</details>

## 功能

- **模型供应商**：填写 Base URL 与 API Key，选择接口格式（OpenAI Chat Completions / OpenAI Responses / Anthropic Messages），可配置最大输出 tokens（Anthropic 必填），点一次「获取模型」拉取模型列表，再勾选要保留的模型；竖向列表维护多个供应商，随时切换「当前」。输入框下方实时预览将要请求的完整地址。API Key 在配置文件中以本机密钥混淆存储。
- **模型选择**：输入框上方有模型选择器（左对齐浮层），支持搜索过滤；当前会话的模型单独记忆。
- **会话管理**：左侧竖向会话列表，新建 / 切换 / 删除，标题自动取自第一条消息。
- **流式对话**：SSE 逐字输出，Markdown 渲染（含代码块高亮）；支持 DeepSeek/GLM `reasoning_content` 与 Anthropic `thinking`，思考过程可折叠查看。消息下方的方形图标按钮提供「复制 / 分支 / 重试」，用户消息另外可以「编辑」并重发。回复失败时重试按钮会展开成红色的「重试」，失败原因直接显示在气泡里。
- **折叠侧边栏**：一键收起会话列表，专注当前对话，状态会记住。
- **分支会话**：以任意一条消息为起点分叉出一个新会话，原会话保持不变。
- **代理设置**：跟随系统（读系统与环境变量）、不使用代理、自定义代理（例如 `http://127.0.0.1:7890`）三选一。
- **深浅色主题**：一键切换并持久化，冷蓝工具配色（见 `DESIGN.md`）。
- **自绘标题栏**：无系统边框，窗口控件与主题配色一致；窗口标题只在标题栏出现，侧边栏不再重复。
- **纯本地**：全部状态存于一个 JSON 文件，没有服务端、没有账号、没有遥测。
- **单文件二进制**，启动即用。

## 快速开始

需要 Rust 1.85 以上（edition 2024）。

```sh
git clone https://github.com/luzov/KiiChat.git
cd KiiChat
cargo run --release
```

配置文件位置（删掉即可重置全部状态）：

| 系统 | 路径 |
| --- | --- |
| Windows | `%APPDATA%\KiiChat\config.json` |
| Linux | `~/.config/KiiChat/config.json` |
| macOS | `~/Library/Application Support/KiiChat/config.json` |

## 使用

1. 点右上角工具栏的齿轮图标进入 **设置**，在 **模型** 分项里点「添加供应商」，填名称、Base URL、API Key。
   Base URL 填接口根地址即可，例如 `https://api.openai.com/v1`；只填 `https://api.deepseek.com` 会自动补上 `/v1`。
2. 点「获取模型」拉取该接口的模型列表，再点「保存」。
3. 回到对话，在输入框左下角选择模型，输入内容后回车发送。
4. 网络需要代理时，到 **网络** 分项选择「自定义代理」并填地址（如 `http://127.0.0.1:7890`），点「应用」。

## 平台支持

Windows 是开发和验证平台（Windows 11 / 200% 缩放）。macOS 与 Linux 由 CI 构建，未逐项手工验证——`gpui-pre-platform` 支持这两个平台，但界面细节（尤其是自绘标题栏）可能需要各自微调。

## 兼容性

支持三种接口格式：

| 格式 | 对话接口 | 模型列表 | 鉴权 |
| --- | --- | --- | --- |
| OpenAI Chat Completions | `POST /chat/completions` | `GET /models` | `Authorization: Bearer` |
| OpenAI Responses | `POST /responses` | `GET /models` | `Authorization: Bearer` |
| Anthropic Messages（Claude） | `POST /messages` | `GET /models` | `x-api-key` + `anthropic-version` |

任何实现其中一种的接口都可以直接使用，例如：

- OpenAI、DeepSeek、Moonshot / Kimi、智谱 GLM、SiliconFlow、OpenRouter
- 本地部署：Ollama（`http://localhost:11434/v1`）、vLLM、LM Studio、one-api / new-api 网关

## 数据与隐私

所有状态都在一个 JSON 文件里，路径：

| 系统 | 路径 |
| --- | --- |
| Windows | `%APPDATA%\KiiChat\config.json`（即 `C:\Users\<你>\AppData\Roaming\KiiChat\config.json`） |
| Linux | `~/.config/KiiChat/config.json` |
| macOS | `~/Library/Application Support/KiiChat/config.json` |

里面有：供应商（**API Key 经本机密钥混淆，但不等于系统级加密**）、会话与消息（含思考过程）、主题、代理设置、侧边栏折叠状态。删掉这个文件即可完全重置；**它不会被提交到仓库**，也请不要把它分享出去。同目录下的 `install.key` 是混淆用的本机密钥，请一并保密。

除了你主动发起的接口请求（`{base_url}/models`、`{base_url}/chat/completions`），程序不访问任何其他网络地址，没有遥测、没有账号。

## 构建与分发

本地构建（首次约 6 分钟，之后增量约 1~2 分钟）：

```sh
cargo build --release     # 产物：target/release/kiichat.exe（约 25 MB）
```

想省掉本地编译，可以推一个 tag，让 GitHub Actions 直接出包：

```sh
git tag v0.1.0 && git push origin v0.1.0
```

`release.yml` 会在 Windows / macOS / Linux 三个平台上构建并把可执行文件挂到 Release 页面；`ci.yml` 在每次推送时跑 `cargo build` + `clippy -D warnings`。

发布形式：**当前是绿色单文件（portable）**，双击即用，不需要安装包——它只依赖系统自带的图形栈（Windows 10 1809+ / 带 GPU 的 macOS / Linux 桌面），不写注册表、不装服务。如果你要分发给非技术用户，再考虑下面的打包方式：

| 方式 | 适用 | 需要 |
| --- | --- | --- |
| 便携 exe（现在） | 自己用、给同行 | 无 |
| MSI | Windows 安装/卸载、开始菜单项 | WiX（`cargo-wix`），可选代码签名 |
| NSIS `setup.exe` | 更小、可自定义安装向导 | NSIS 脚本 |
| `.app` + `dmg` | macOS 分发 | `cargo-bundle`，需 Apple 签名/公证 |
| AppImage / `.deb` | Linux 分发 | `linuxdeploy` / `cargo-deb` |

注意：未签名的 exe 在别的机器上首次运行会被 SmartScreen 拦一下（“更多信息 → 仍要运行”）；要消除需要购买代码签名证书。

## 已知限制

这是刻意做小的客户端，以下都不支持：

- 只有对话：没有工具调用、图片、文件附件、语音。
- 思考过程默认折叠，只保证最终回答始终可见。
- 没有系统提示词、temperature 等参数的自定义；不发多轮以外的上下文。
- 分支只是复制已有消息到新会话，不共享后续对话。
- 单窗口；会话历史整体存在一个 JSON 文件里，不会分页或归档。
- API Key 为本机混淆，不是系统钥匙串/DPAPI 级加密。

## 开发

项目约定、架构说明与验证方法见 [AGENTS.md](AGENTS.md)。常用命令：

```sh
cargo build                  # 编译（debug）
cargo run                    # 启动窗口
cargo clippy --all-targets   # 静态检查
```

`scripts/` 下是开发期工具，用于在没有键盘、看不到画面的情况下验证界面：

- `scripts/mock_openai.py`：假的 OpenAI 兼容服务（`127.0.0.1:18080`），用于端到端验证流式对话与模型拉取。
- `scripts/uia.ps1` / `scripts/buttons.ps1`：打印窗口的无障碍树 / 按钮清单（名称、位置），确认界面真的渲染出来了。
- `scripts/invoke.ps1`、`scripts/click.ps1`：通过 UI Automation 调用按钮，或做 DPI 感知的合成点击。
- `scripts/capture.ps1`：按窗口实际像素截图（配 `PIL` 可做颜色验证）。

## 技术栈

| 部分 | 选型 |
| --- | --- |
| UI 框架 | GPUI（Zed 发布快照 `gpui-pre`）+ `gpui-pre-platform` |
| 控件与主题 | `gpui-component`（`Root`、`TitleBar`、Button、Input、主题令牌、`TextView`） |
| 对话界面 | `gpui-ai` 的 `PromptBar`（输入框）与 `StreamingText`（流式 Markdown）；气泡、消息动作、会话列表由本项目自己实现 |
| 网络 | `reqwest` + 独立线程上的 tokio 运行时，SSE 分片经 `async-channel` 回传 UI |
| 存储 | `serde` + `serde_json`，原子写入单个配置文件 |

## 许可

[MIT](LICENSE)