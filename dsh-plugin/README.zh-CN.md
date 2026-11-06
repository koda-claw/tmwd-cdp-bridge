# tmwd-cdp-bridge DSH 插件

可选的 [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness)
插件，把本机的 `tmwd-cdp-bridge` 浏览器桥接能力暴露为原生模型工具。插件只是
一个带鉴权的本地 HTTP 客户端；浏览器操作仍由 bridge、Chrome/Edge 扩展和浏览器
完成。

## 工具

| 工具 | 参数 | 用途 |
|---|---|---|
| `tmwd_browser_status` | 无 | 检查 `/health`、扩展连接状态和 token 文件是否存在；只读且安全。 |
| `tmwd_list_tabs` | 无 | 列出已知标签页的 `id`、URL、标题和激活状态。 |
| `tmwd_find_tab` | `url_contains?`、`title_contains?` | 按 URL 或标题子串查找第一个匹配的标签页。 |
| `tmwd_execute_js` | `session_id?`、`code`、`timeout?`、`fallback?` | 在标签页中执行 JavaScript，返回可 JSON 序列化的结果。 |
| `tmwd_cdp` | `session_id?`、`method`、`params?`、`timeout?` | 发送原始 Chrome DevTools Protocol 命令，用于仅 CDP 可用的能力或 CSP 失败场景。 |

每次 RPC 调用都会重新读取 token 文件。工具不会返回 token，也不会把它写进日志。

## 前置条件

- DeepSeek Harness `0.2.2-rc.1` 或更新版本，并提供 `tools` 服务。
- 本机已安装并启动 `tmwd-cdp-bridge`。
- 已加载匹配的 unpacked 扩展，并在 Chrome 或 Edge 中连接成功。

安装插件前先确认 bridge 状态：

```sh
tmwd-cdp-bridge status --json
tmwd-cdp-bridge start
curl -s http://127.0.0.1:18766/health
```

`/health` 应包含 `"extension_connected": true`，扩展 id 为
`eghifjkffmcmffejmaaeicejpfopplem`。

## 安装

从 npm 安装：

```sh
dsh plugin --profile web add @koda-claw/dsh-plugin-tmwd-cdp-bridge
dsh web
```

从源码 checkout 安装，本地路径等价：

```sh
dsh plugin --profile web add ./dsh-plugin
dsh web
```

CLI 会记录依赖、链接 `dsh-plugin/cordis.patch.yml` 中的 bundle patch，并在
profile 中启用该 bundle。如果 profile 正在运行且启用了 HMR，新工具会自动出现；
否则重启 profile。

包已发布为
[`@koda-claw/dsh-plugin-tmwd-cdp-bridge@0.1.0`](https://www.npmjs.com/package/@koda-claw/dsh-plugin-tmwd-cdp-bridge)，
`dsh-plugin-` 前缀沿用社区 DSH bundle 的命名约定。推荐从 npm 安装；从本仓库或
包含 `dsh-plugin/` 的 release archive 安装也可以。

## 配置

bundle 插入的条目 id 是 `tmwd-cdp-bridge`。在 profile 自己的
`cordis.patch.yml`（例如 `~/.dsh/profiles/web/cordis.patch.yml`）中覆盖配置：

```yaml
- id: tmwd-cdp-bridge
  config:
    httpPort: 18766
    requestTimeoutMs: 120000
    # tokenFile: /absolute/path/to/token
    # appDir: /absolute/path/to/tmwd-cdp-bridge
```

| 字段 | 默认值 | 含义 |
|---|---|---|
| `httpPort` | `18766` | bridge HTTP RPC 端口；主机固定为 `127.0.0.1`。 |
| `requestTimeoutMs` | `120000` | 单次 HTTP 请求的上限。`tmwd_execute_js` 和 `tmwd_cdp` 在 `timeout` 加开销更大时会自动放宽。 |
| `tokenFile` | 平台默认 | 显式指定 token 路径。 |
| `appDir` | 平台默认 | bridge 应用目录；插件读取 `<appDir>/token`。 |

token 默认路径与 bridge 一致：

- macOS：`~/Library/Application Support/tmwd-cdp-bridge/token`
- Linux：`${XDG_DATA_HOME:-$HOME/.local/share}/tmwd-cdp-bridge/token`
- Windows：`%LOCALAPPDATA%\tmwd-cdp-bridge\token`

未设置 `appDir` 时会读取 `CDP_BRIDGE_APP_DIR`。

## 典型流程

1. 调用 `tmwd_browser_status`，先解决不可达或扩展未连接的问题。
2. 用 `tmwd_list_tabs` 或 `tmwd_find_tab` 选择标签页。
3. 先执行只读的 `tmwd_execute_js`，例如 `document.title` 或
   `document.body.innerText.slice(0, 4000)`。
4. 遇到 CSP 或 isolated world 限制时，用 `fallback: "cdp"` 重试，或调用
   `tmwd_cdp` 执行 `Runtime.evaluate`。
5. 对页面做破坏性操作前先让用户确认。

## 安全与隐私

- 插件只访问 `http://127.0.0.1:<httpPort>`，不能配置任意 bridge URL。
- token 每次调用时从磁盘读取，只发送给本机 loopback bridge；错误信息不包含 token。
- `tmwd_execute_js` 和 `tmwd_cdp` 运行在用户已登录的浏览器上下文中，可以读取
  页面数据、改变页面状态。工具描述要求模型在执行破坏性操作前确认，但插件本身
  无法强制用户意图。
- 插件不会启动、停止、安装或关闭 bridge，也不会对外暴露 bridge 端口。
- bridge 和扩展持有较强的浏览器权限（扩展声明 `cookies`、`debugger`、
  `<all_urls>`）。请把 token 文件当作浏览器控制权保护，只从可信来源安装 bridge。

## 限制

- bridge 必须已经运行且扩展已连接；插件不会自动启动。
- 一个 bridge 进程只支持一个浏览器连接。
- `execute_js` 作用于顶层 frame；CSP 和 isolated world 场景用 CDP 模式处理。
- 本版本有意不暴露新建/关闭标签页，以及 bridge 的 `batch` 和 `shutdown` 命令。
- 已在 macOS 上针对 DeepSeek Harness `0.2.2-rc.1`、bridge `0.1.3`、扩展 `2.3`
  验证。Windows 和 Linux 使用 bridge 文档中的 token 路径，已有单元测试覆盖，
  但尚未纳入真实浏览器发布矩阵。

## 为什么用原生插件而不是 MCP server

原生 Cordis bundle 直接注册工具，复用 profile 的配置与 token 处理，不需要额外
常驻进程。对 DSH 之外的 Agent，MCP 包装仍然有价值，但本仓库已经提供通用的
`skills/tmwd-cdp-bridge` 流程和 RPC 契约。

## 发布

包已发布为 `@koda-claw/dsh-plugin-tmwd-cdp-bridge@0.1.0`，并声明
`publishConfig.access: "public"`。发布新版本时，更新 `package.json` 中的版本号，
然后在干净的 checkout 中执行：

```sh
cd dsh-plugin
npm publish --access public
```

发布账号必须是 `@koda-claw` 作用域的 owner。bundle patch 使用 scoped 包名，
因此 npm 安装与本地路径安装行为一致。

## 开发

```sh
cd dsh-plugin
node --test
```

单元测试覆盖 token 与路径解析、HTTP 错误映射、超时以及每个工具的参数载荷。此外
已用真实 DeepSeek Harness `0.2.2-rc.1` 运行时验证五个工具通过 `defineTool`
注册，并用真实 bridge `0.1.3` 和已连接扩展验证状态、标签页列表、标签页查找和
只读 `document.title` 执行。
