# 开发

[← 返回 README](../README.zh-CN.md) · [English](development.md)

## 调试工具

```bash
# client/：调试构建（控制台启动就打开，被控端在同一个进程里）；NIFE_REMOTE_LOG 控制日志级别
NIFE_REMOTE_LOG=debug npm run tauri dev
# client/：音画同步实测（被控端播放闪白 + 短音，用法见脚本开头）
node scripts/measure-avsync.mjs <控制码> [配对码]

# client/src-tauri/：无界面被控端，用来在同一台机器上调试控制端
#（请在控制端静音，否则播放出的声音会被再次采集，形成回声）
cargo run --release --example headless_host
# client/src-tauri/：编码性能测试（抓真实屏幕，对比各编码器）
cargo run --release --example encode_bench
```

测画面流畅度或音画同步时请用 release 构建：调试构建的 CPU 缩放很慢（“文字清晰”或者显卡不支持时用，4K → 1080p 每帧约 0.5 秒）。比较 GPU 零拷贝和 CPU 两条路的耗时：`cargo run --release --example encode_bench`。

## 调试 Windows 服务

调试构建在托盘进程里运行被控端，`npm run tauri dev` 不需要安装。要调试服务本身，可以装上安装程序，或者在管理员终端里直接注册某个构建：

```powershell
# client/src-tauri/
target\release\nife-remote.exe --install       # 为这个 nife-remote.exe 注册并启动服务（在它旁边生成 config.toml）
target\release\nife-remote.exe --stop-service  # 重新构建之前先停止：exe 正在使用
target\release\nife-remote.exe --uninstall     # 删除服务和开机启动项
```

服务和被控端的日志用 `OutputDebugString` 输出：以管理员身份运行 Sysinternals 的 DebugView，勾选 Capture → Capture Global Win32。要改日志级别，在 `HKLM\SYSTEM\CurrentControlSet\Services\Nife Remote` 下新建多字符串值 `Environment`，写入 `NIFE_REMOTE_LOG=debug`，然后重启服务。服务装着的时候，从别处启动的调试构建仍然在自己的进程里运行被控端（有自己的 `config.toml` 和控制码）。

## 网页控制台

```bash
# 先构建界面，再在 client/web/ 里启动服务（默认连接 127.0.0.1:4433 的服务器，网页取自 client/dist（默认 ../dist）），浏览器打开 http://localhost:8080
(cd client && npm run build)
cd client/web && cargo run
# 修改界面时：在 client/ 里 npm run dev，浏览器打开 http://localhost:1420（/ws 会转给 8080 端口上的 nife-remote-web）
```

`crates/nife-remote-link/src/console.rs` 开头写了控制台和控制端服务之间的 WebSocket 协议（桌面版和浏览器一样）。

## 约定

### 协议版本

有两个版本号（`crates/nife-remote-proto/src/lib.rs`），规则见[工作原理 → 协议的演进](architecture.zh-CN.md#协议的演进)：

- `SIGNAL_VERSION` / `SIGNAL_MIN`：客户端 ⇄ 服务器（信令、会话头、数据报的会话 ID）。服务器接受这个区间里的客户端；没有交集时回复 `Incompatible`（带着服务器的区间），客户端提示用户该更新 Nife Remote 还是服务器，之后每分钟重试一次
- `SESSION_VERSION` / `SESSION_MIN`：被控端 ⇄ 控制端，在端到端握手里协商，服务器不看。不兼容时控制端显示 `peerIncompatible`

改协议时先想能不能兼容地改，能就不加版本号：

- 加消息：给枚举加一个**新编号**的变体，旧版本会跳过它；对方可能不认识的话，先在 `features` 里分一个能力位，对方报了才发
- 加字段：只能加 `Option`（或者带 `#[cbor(default)]`）的字段，用新的编号；必填字段要永远存在
- 枚举加值：只有编号的枚举用 `open_enum!`（`crate::cbor`）声明，不认识的值在哪里都解成 `Unknown(原值)`；收的一方要想好怎么对待它（不管、换成默认值、当成"对方的版本更新"）
- 带字段的枚举加变体：只有三个地方能跳过：整条消息、标了 `crate::cbor::opt` 的 `Option` 字段、标了 `crate::cbor::open_vec` 的列表元素。别的地方遇到不认识的变体故意是解码错误。不要把带字段的枚举直接放进派生的 `Option<…>`（也不要放进结构体、结构体再放进 `Option`）：minicbor 派生会把里面任何深度的不认识的变体当成 `None`，再从错的位置跳过（见 `cbor.rs` 的说明）
- 编号一旦发布就不能改、不能复用；只有改不下去（例如要删掉必填字段）才加版本号

版本握手里的版本区间（`ClientHello` / `ServerHello` 这两个 CBOR map 的 0、1 号键，`signal::Versions`）和 ALPN **永远不能改**，否则不同版本之间连“版本不一致”都无法告诉对方。

### 一份协议，两份实现

v9 发布之后，消息的编号和字段位置就固定了，不能随着实现重新生成：现有消息保持兼容，新增消息用新的编号。Rust 和控制台（TypeScript）用同一套格式：媒体包（`crates/nife-remote-proto/src/media.rs`）、输入事件、界面命令、连接选项（`nife_remote_link::controller::Command`、`ConnectOptions`）。界面和 Rust 之间的 JSON 全都是 camelCase：字段名、枚举值（`"h264"`、`"android"`、`"quickSettings"`），带字段的联合类型用一个字段标明是哪一种（命令 `cmd`、事件和输入 `type`、问题和提示 `kind`）。改 Rust 那边的格式时，同一次改动里把 TypeScript 的类型和解析一起改：`npm run typecheck` 是唯一的检查。

### 消息是类型，不是文字

凡是经过网络、或者从 Rust 端传给界面、可能显示给用户的内容，都是枚举（`ConnectError`、`Notice`、`ByeReason`、`Problem` 等），由界面按用户的语言变成文字：

- 控制台的文案在 `client/src/locales/zh.ts`（全部词条以它为准）和 `en.ts`
- 托盘的文案用 `tray.rs` 里的 `tr!`（`client/src-tauri/src/tray.rs`）
- 新增 Rust 的 `Problem` / `NoticeView` 时，两种语言的词条一起加（`zh.ts` 是全部词条的准）
- 日志和内部错误一律用英文

### 图标

源文件只有 `assets/icon.svg`。改完在仓库根目录跑 `node scripts/icon.mjs`：它用系统自带的 Edge/Chrome 无头渲染 SVG（里面有 foreignObject，普通光栅化库画不出来），再生成整套程序图标、托盘图标、界面 Logo 和 favicon。细节见 `assets/README.md`。
