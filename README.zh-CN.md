<p align="center"><img src="assets/icon.svg" width="96" alt="Nife Remote"></p>

<h1 align="center">Nife Remote</h1>

<p align="center"><b>高性能、低延迟的远程桌面</b><br>QUIC 传输 · 硬件编解码 · 最高 120 fps<br>Windows 客户端（被控 + 控制） · 浏览器控制台</p>

<p align="center"><a href="README.md">English</a> · <b>简体中文</b></p>

---

Nife Remote 是一个**高性能、低延迟**的远程桌面：画面用 GPU 硬件编码（AV1 / HEVC / H.264），最高 120 fps，经 QUIC 传输；网络变慢时直接跳到最新一帧，延迟不会越积越多；鼠标指针在本地绘制，移动感觉不到延迟。

同一个程序既是**被控端**（被远程控制的设备），也是**控制端**（你用来操作的设备）。两端通过你自己部署的**中转服务器**相连，服务器很小，用 Docker Compose 一条命令就能部署。输入对方的 6 位**控制码**就能连上。

> [!NOTE]
> **开发中（0.0.1）。** 从协议 v9 起，客户端和服务器、被控端和控制端之间各有一个版本区间，新功能用能力位协商、不认识的消息跳过，以后的版本可以混用；和 v9 之前的版本不兼容，会检测出来并提示（见[协议版本](docs/development.zh-CN.md#协议版本)）。

## 性能

从采集到显示，每一环都为低延迟设计：

- **硬件编解码**：被控端用 GPU 硬件编码（NVENC / Quick Sync / AMF），低延迟模式、无 B 帧；控制端用 GPU 解码（WebCodecs）。实测 4K 屏输出 1080p 时，硬件 AV1 编码每帧约 7 ms（RTX 显卡）
- **最好的格式**：按 AV1 > HEVC > H.264 自动选用双方都支持的、压缩率最高的格式；缩放和颜色转换用 SIMD，缓冲区全部复用
- **延迟不堆积**：被控端限制在途字节（≈ 码率 ×（RTT + 50 ms）），网络慢时跳到最新一帧；码率每次确认都按量到的排队时延和带宽调整，几百毫秒内降下来，恢复后逐步回升
- **直连**：两端在中转之外打洞直连（同一个局域网、或者 NAT 不是对称型时），画面、声音不再绕道服务器；自动选延迟低的那条路，直连断了马上回到中转，会话不断
- **QUIC + BBR**：BBR 拥塞控制在 Wi-Fi、跨境等丢包网络上比 Cubic 更稳；控制消息、声音、画面分开排队，鼠标和 Ping 不会排在大帧后面；中转服务器只转发字节，不解析画面
- **解码即显示**：解码器开启低延迟模式，解完立即画到 desynchronized Canvas，不排队等 vsync
- **本地指针**：远端光标的形状同步到本地，移动在本地完成
- **省带宽**：画面不变就不发帧，静止的桌面几乎不占带宽；不在屏幕上的会话自动降到 5 fps

详见[工作原理 → 画面](docs/architecture.zh-CN.md#画面)。

## 功能

- **Windows 系统服务**：锁屏、登录界面和 UAC 确认框也能看到、能操作，开机后登录之前就能被控制
- **固定的控制码**：重启、升级，甚至服务器重启之后都不变，直到你主动更换
- **控制台**：浏览器式标签页，同时控制多台电脑，多屏总览同时看所有画面。连接过的设备自动保存，下次点一下就连；对方离线时，等它上线自动连上
- **网页控制台**：控制端也能用 Docker 部署，浏览器打开就能用，操作的电脑上什么都不用装
- **声音**：被控端的系统声音同步到控制端（Opus + 自适应抖动缓冲），看视频时可以打开音画同步。反过来，控制端的麦克风可以放进 Windows 被控端的虚拟麦克风（Steam Streaming Microphone，用过一次 Steam 远程同乐就有），在那边开会、语音输入
- **实用工具**：剪贴板双向同步、以键盘输入方式发送剪贴板、系统快捷键（Win、Alt+Tab、Ctrl+Shift+Esc……）、多显示器、全屏时锁定键盘、画质预设
- **默认安全**：会话端到端加密，中转服务器看不到画面和键盘输入；第一次连接要输入对方托盘里的配对码（不经过服务器），之后认得出对方的身份；TLS 1.3 + 证书指纹固定，控制码防暴力猜测；被控端随时能看到谁连着，一键断开
- **不留痕迹**：要保存的东西都在一个 `config.toml` 里，其余运行时文件退出时删除；[卸载](docs/usage.zh-CN.md#卸载)后什么都不剩
- **中英双语**：跟随系统语言；对方电脑和服务器发来的提示也按**你**的语言显示

## 支持的平台

| 平台 | 被控端 | 控制端 | 说明 |
|---|:-:|:-:|---|
| Windows 10 / 11 | ✅ | ✅ | 需要安装，被控端作为系统服务运行 |
| 浏览器（网页控制台） | — | ✅ | Chrome / Edge、Safari 16.4+、Firefox 130+ |

**客户端只支持 Windows。** 中转服务器（Docker）和网页控制台跑在 Linux 上。已知问题见[已知限制](docs/limitations.zh-CN.md)。

## 工作方式

```mermaid
flowchart LR
    host["被控端<br>Windows"]
    relay["中转服务器<br>nife-remote-server（Docker）"]
    ctrl["控制端<br>桌面控制台 / 网页控制台"]
    host <-->|"QUIC · UDP 4433"| relay
    relay <-->|"QUIC · UDP 4433"| ctrl
    host -. "直连（打得通时）" .- ctrl
```

被控端启动后连上中转服务器，领到一个 6 位控制码，控制端用控制码找到它。服务器只负责登记控制码、配对会话和转发字节；会话是端到端加密的，服务器看不到内容。被控端和控制端都是主动连接服务器，不需要开放端口，也不需要公网 IP。会话开始后两端还会试着打洞直连，打通了画面就不再经过服务器（打不通就一直走中转）。详见[工作原理](docs/architecture.zh-CN.md)。

## 快速开始

需要准备：

- 一台两端都能访问到的机器，装好 Docker，开放 **UDP 4433**（中转）和 **TCP 443**（Caddy）
- 构建客户端用的 Windows 机器，以及 Rust、Node.js 20+、CMake（见[构建](docs/building.zh-CN.md#依赖)）

### 1. 部署服务端

一台装好 Docker 的机器，开放 **UDP 4433**（中转）和 **TCP 443**（Caddy，账号服务）。

**1）取两个文件**（镜像里只有程序，端口 / 卷 / 环境变量在 compose 里；不用 clone 仓库）

```bash
mkdir -p ~/nife-remote && cd ~/nife-remote
base=https://raw.githubusercontent.com/OliverMao/nife-remote/main   # 国内不通可换 https://cdn.jsdelivr.net/gh/OliverMao/nife-remote@main
curl -fsSLO $base/docker-compose.yml
curl -fsSL  $base/.env.example -o .env
```

**2）填 `.env`**

```ini
NIFE_REMOTE_IMAGE=mirror.naig.us/nife-remote-server:0.0.1
NIFE_REMOTE_AUTH_IMAGE=mirror.naig.us/nife-remote-auth:0.0.1
NIFE_REMOTE_AUTH_RELAY_HOST=relay.example.com:4433   # 客户端直连的中转地址（原样交给客户端）
NIFE_REMOTE_AUTH_RELAY_FP=                           # 留空 = 客户端第一次连接时记下指纹
```

**3）Caddy 反代账号服务**（客户端和网页控制台都用 `https://auth.example.com`）

```caddyfile
auth.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

写进 `/etc/caddy/Caddyfile`（和已有站点并列），然后 `systemctl reload caddy`。域名要解析到这台机器，
`80` / `443` 要放行，Caddy 会自动申请证书。

**4）启动**

```bash
docker compose pull && docker compose up -d
curl -s https://auth.example.com/health        # {"ok":true,...,"relay":"relay.example.com:4433"}
curl -s https://auth.example.com/relay         # {"relay":{"address":"...","fingerprint":""}}
```

首次启动自动建账号 `niferemote` / `niferemote`，登录后立刻改密码。

- 中转服务器**不要**放进 Caddy：它是 UDP 4433 的 QUIC，代理不了，客户端直连
- 数据在 `data/`（证书、控制码数据库、`auth/accounts.json`），**保留这个目录**
- 升级：改 `.env` 里的 tag → `docker compose pull && docker compose up -d`
- 账号服务也可以和中转服务器分在两台机器上：它只把地址和指纹交给客户端

### 2. 构建客户端（Windows）

```powershell
cd client
npm install
npm run tauri build            # 出安装包：bundle/nsis/Nife Remote_<版本>_x64-setup.exe
```

客户端连哪个账号服务**只由构建决定**，装好之后界面和配置文件都改不了、也看不到。构建时按
环境变量 `NIFE_REMOTE_AUTH` → `client/.env` → 默认值（正式服务 `https://nife-remote-auth.naig.us`）取，
所以 `npm run tauri build` 直接就是正式服务；自己另部署账号服务就设 `NIFE_REMOTE_AUTH`（或写进
`client/.env`）。中转服务器的地址和指纹由账号服务分配，构建时不用管。

### 3. 连接

1. 两台电脑都装上 Nife Remote（运行安装程序，需要管理员权限）
2. 在**被控**的电脑上右键托盘图标，看到 6 位**控制码**（控制台首页上也有）
3. 在**你自己**的电脑上打开 Nife Remote（控制台启动就会打开；关掉了就从托盘菜单选**打开控制台**），输入控制码。第一次连接这台电脑时还要输入它托盘菜单里的 6 位**配对码**，之后就不用了

完整的使用说明见[使用指南](docs/usage.zh-CN.md)。

### 不装客户端：网页控制台

在任意一台装了 Docker 的机器上启动网页控制台，用浏览器操作：

```bash
mkdir -p client/web && cd client/web
base=https://raw.githubusercontent.com/OliverMao/nife-remote/main
curl -fsSLO $base/client/web/docker-compose.yml
curl -fsSL  $base/client/web/.env.example -o .env
```

`.env` 填三项：

```ini
NIFE_REMOTE_WEB_PORT=3000                            # 8080 被账号服务占了
NIFE_REMOTE_SERVER=relay.example.com:4433            # 中转地址
NIFE_REMOTE_SERVER_FP=<中转服务器的证书指纹>
```

```bash
docker compose pull && docker compose up -d
```

本机打开 `http://localhost:3000`，其它电脑 `https://<这台机器的地址>:3000`（自签证书，浏览器提示“不安全”，
继续即可）。要正规证书就在 Caddyfile 里加 `web.example.com { reverse_proxy 127.0.0.1:3000 }`；访问密码
`NIFE_REMOTE_WEB_PASSWORD=...`。

## 文档

| 文档 | 内容 |
|---|---|
| [使用指南](docs/usage.zh-CN.md) | 被控端、Windows 安装版、配置文件、卸载、控制台、画质、界面语言 |
| [构建](docs/building.zh-CN.md) | Windows 依赖和安装包、签名、APK |
| [工作原理](docs/architecture.zh-CN.md) | 架构、代码目录、Windows 服务、画面和声音、音画同步、磁盘上的文件 |
| [安全](docs/security.zh-CN.md) | 加密和证书固定、防暴力猜码、控制码和重连凭证 |
| [已知限制](docs/limitations.zh-CN.md) | 已知问题 |
| [开发](docs/development.zh-CN.md) | 测试、调试工具、项目约定 |

## 安全须知

- 全程 TLS 1.3（QUIC），客户端用证书指纹校验服务器。构建时不设置指纹，客户端就在第一次连上服务器时记住它的证书，之后证书变了就拒绝连接，只有第一次连接防不了中间人；正式发布时请固定指纹
- **端到端加密**：画面、声音、键盘鼠标、剪贴板在被控端和控制端之间加密，中转服务器看不到、也改不了；它只知道谁在什么时候连了谁、流量多大。网页控制台是端到端加密的终点，它所在的机器看得到画面
- **控制码 + 配对码**：服务器知道所有控制码，所以第一次连接还要输入被控端托盘里的配对码（不经过服务器，输错 5 次就暂停），配对之后被控端记住这个控制端。Windows 被控端以 SYSTEM 身份运行，能连上的人就能以管理员权限操作这台电脑；被控端的托盘里随时能看到谁连着，可以断开他们、更换控制码或配对码

详见[安全](docs/security.zh-CN.md)。

## 参与开发

| 目录 | 内容 |
|---|---|
| `crates/` | 协议（`nife-remote-proto`）和客户端到服务器的链路（`nife-remote-link`） |
| `server/core/` | 中转服务器（Rust + QUIC） |
| `server/auth/` | 账号服务（Python + FastAPI，账号与"本账号设备"） |
| `client/web/` | 网页控制台（和桌面版同一份界面、同一套控制台协议，只是换了个外壳：浏览器里用） |
| `client/` | 桌面客户端：Tauri（Rust）+ React 界面，界面也给网页控制台用 |

怎么跑测试、调试工具和项目约定（协议版本、界面文案等）见[开发](docs/development.zh-CN.md)。

## 许可证

[MIT](LICENSE)
