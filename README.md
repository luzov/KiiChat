# KiiChat

一个轻量的、Cherry Studio 风格的桌面大模型聊天客户端，使用 [GPUI](https://gpui.rs)（Zed 的 GPU 加速 UI 框架）编写：添加任意 OpenAI 兼容的接口，一键拉取模型列表，然后直接开聊。

![浅色模式](docs/screenshot-light.png)

<details>
<summary>深色模式</summary>

![深色模式](docs/screenshot-dark.png)

</details>

## 功能

- **供应商管理**：填写 Base URL 与 API Key，点一次「获取模型」从 `{base_url}/models` 拉取模型列表；可保存多个供应商，竖向列表切换，随时「设为当前」。
- **会话管理**：左侧竖向会话列表，新建 / 切换 / 删除，标题自动取自第一条消息。
- **流式对话**：SSE 逐字输出，Markdown 渲染（含代码块高亮）；消息可复制、就地编辑、重新生成；失败的消息带「重试」按钮，失败原因直接显示在气泡里。
- **深浅色主题**：一键切换并持久化。
- **自绘标题栏**：无系统边框，窗口控件与主题配色一致。
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

1. 左下角点「设置」→ 供应商 → 「添加供应商」，填名称、Base URL、API Key。
   Base URL 填接口根地址即可，例如 `https://api.openai.com/v1`；只填 `https://api.deepseek.com` 会自动补上 `/v1`。
2. 点「获取模型」拉取该接口的模型列表，再点「保存」。
3. 回到对话，在输入框左下角选择模型，输入内容后回车发送。

## 兼容性

任何实现了 `GET /models` 和 `POST /chat/completions`（`stream: true`）的 OpenAI 兼容接口都可以直接使用，例如：

- OpenAI、DeepSeek、Moonshot / Kimi、智谱 GLM、SiliconFlow、OpenRouter
- 本地部署：Ollama（`http://localhost:11434/v1`）、vLLM、LM Studio、one-api / new-api 网关

## 已知限制

这是刻意做小的客户端，以下都不支持：

- 只有对话：没有工具调用、图片、文件附件、语音。
- 不显示 reasoning / 思考过程，只渲染最终回答。
- 没有系统提示词、temperature 等参数的自定义。
- 单窗口；会话历史整体存在一个 JSON 文件里，不会分页或归档。

## 开发

项目约定、架构说明与验证方法见 [AGENTS.md](AGENTS.md)。常用命令：

```sh
cargo build                  # 编译（debug）
cargo run                    # 启动窗口
cargo clippy --all-targets   # 静态检查
```

`scripts/` 下是开发期工具，用于在没有键盘/鼠标时验证界面：

- `scripts/mock_openai.py`：假的 OpenAI 兼容服务（`127.0.0.1:18080`），用于端到端验证流式对话与模型拉取。
- `scripts/uia.ps1`：打印窗口的无障碍树（控件类型、名称、位置），确认界面真的渲染出来了。
- `scripts/invoke.ps1`、`scripts/click.ps1`：通过 UI Automation 或合成点击驱动按钮。
- `scripts/capture.ps1`：截取窗口截图。

## 技术栈

| 部分 | 选型 |
| --- | --- |
| UI 框架 | GPUI（Zed 发布快照 `gpui-pre`）+ `gpui-pre-platform` |
| 控件与主题 | `gpui-component`（`Root`、`TitleBar`、Button、Input、主题令牌） |
| 对话界面 | `gpui-ai`（`Chat` 虚拟化transcript、`PromptBar` 输入框、流式 Markdown） |
| 网络 | `reqwest` + 独立线程上的 tokio 运行时，SSE 分片经 `async-channel` 回传 UI |
| 存储 | `serde` + `serde_json`，原子写入单个配置文件 |

## 许可

[MIT](LICENSE)