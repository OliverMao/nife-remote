# 部署：四个端

[← 返回 README](../README.zh-CN.md) · [English](deploy.md)

这一篇讲怎么把 Airhand 放到真实环境里：每一端装在哪、开放什么端口、配置什么。只想先跑起来的话，看 README 的[快速开始](../README.zh-CN.md#快速开始)；这里是把每一端拆开讲清楚。

## 有哪几个端

| 端 | 是什么 | 装在哪 | 端口 |
|---|---|---|---|
| **中转服务器** | 按控制码把两端配对，只转发加密后的字节 | 一台有公网 IP 的机器，Docker | 入站 **UDP 4433** |
| **被控端** | 采集画面 / 声音、编码、注入键鼠 | 要被控制的每台电脑 | 只需**出站** UDP |
| **控制端（桌面）** | 解码、显示、采集输入 | 你自己的电脑 | 只需**出站** UDP |
| **网页控制台** | 控制端跑在服务器上，浏览器当界面 | 任意装了 Docker 的机器 | 入站 **TCP 8080** |

被控端和控制端是**同一个安装包**：装好之后在托盘菜单里切换角色，不需要装两份。网页控制台是控制端的另一种形态，被控的电脑仍然要装客户端。

```
                    ┌──────────────────┐
   被控端  ──UDP──▶ │   中转服务器      │ ◀──UDP── 控制端（桌面）
  （画面源头）       │  UDP 4433        │
                    └──────────────────┘ ◀──UDP── 网页控制台（容器）
                             ▲
                             └── 浏览器 ──TCP 8080──▶ 网页控制台
```

两端打得通直连时，画面和声音不经过服务器（见[直连](#直连与端口)），但服务器始终负责配对和兜底。

## 中转服务器

服务器监听 **UDP 4433**。首次启动时会在 `docker-compose.yml` 同目录的 `data/` 下生成自签名证书，**请保留这个目录**：客户端靠证书指纹认出服务器，被控端固定的控制码也记在这里的 SQLite 数据库里（`airhand.db`，服务器运行时旁边还有 `-wal`、`-shm` 两个文件）。保存控制码的被控端（有 `config.toml` 的桌面客户端）不管离线多久，都能从这里取回原来的控制码。

### 启动

```bash
git clone <仓库地址> airhand
cd airhand
docker compose up -d --build
docker compose logs airhand | grep fingerprint
```

日志里会打印证书的 SHA-256 指纹（64 位十六进制），客户端要用。确认在跑：

```bash
docker compose ps
ss -lunp | grep 4433        # 应该是 0.0.0.0:4433
```

镜像基于 distroless，约 28 MB，没有 shell，**只能看日志**，`docker exec` 进不去。

### 环境变量（见 `docker-compose.yml`）

| 变量 | 默认值 | 说明 |
|---|---|---|
| `AIRHAND_LISTEN` | `0.0.0.0:4433` | 监听地址（UDP） |
| `AIRHAND_CERT` / `AIRHAND_KEY` | 服务端程序旁边的 `data/cert.pem` / `data/key.pem`（Docker 镜像里是 `/data/cert.pem` / `/data/key.pem`） | 证书路径，不存在时自动生成；`data/` 文件夹不存在时自动创建 |
| `AIRHAND_CERT_NAMES` | `airhand,localhost` | 自动生成证书时写入的 SAN |
| `AIRHAND_DB` | 证书旁边的 `airhand.db`（默认就是服务端程序旁边的 `data/airhand.db`） | SQLite 数据库，存固定的控制码（控制码、被控端密钥的哈希、最后在线时间）。丢了它，离线的被控端的控制码就可能分给别人（没人占用的话仍然能取回）。打不开时服务器把控制码记在内存里，并在日志里报错 |
| `AIRHAND_MAX_SESSIONS` | `4` | 每台被控端同时允许的控制端数量 |
| `AIRHAND_MAX_CONNS_PER_IP` | `128` | 同一个 IPv4 地址 / IPv6 的 /64 同时最多几条连接（网页控制台每个标签页一条；很多被控端共用一个出口 IP 时调大） |
| `AIRHAND_NEW_CODES_PER_IP` | `30` | 同一个 IPv4 地址 / IPv6 的 /48 每 10 分钟最多分配几个新控制码（取回原来的控制码不算） |
| `AIRHAND_CONNECTS_PER_MINUTE` | `6` | 同一个 IPv4 地址 / IPv6 的 /64 一分钟内最多向同一台被控端发起几次连接（不低于 `AIRHAND_MAX_SESSIONS`；很多控制端共用一个出口 IP 连同一台被控端时调大） |
| `RUST_LOG` | `info` | 日志级别 |

镜像以 root 运行：挂载进来的 `data/` 不管属于谁都写得进去（目录不存在时 Docker 会替你建一个属于 root 的）。Linux 宿主机上用 rootful Docker 时，服务在里面生成的文件属于 root，读取、备份、删除要用 sudo。

### UDP 缓冲区

服务器把 socket 的收发缓冲区设成 8 MiB（系统默认只有约 208 KB，几十 Mbps 的画面一个突发就能把它填满、整批丢包，表现为卡顿和延迟尖峰）。Linux 内核会把超过 `net.core.rmem_max` / `wmem_max` 的部分悄悄截掉（容器里改不了，要在宿主机上改），被截掉时服务器启动时会在日志里警告：

```bash
sudo sysctl -w net.core.rmem_max=8388608 net.core.wmem_max=8388608
# 重启后仍然有效：
echo -e 'net.core.rmem_max=8388608\nnet.core.wmem_max=8388608' | sudo tee /etc/sysctl.d/90-airhand.conf
```

### 监控

没有 HTTP 健康检查接口，运维看三样：

```bash
docker compose ps                 # 容器在不在
ss -lunp | grep 4433             # UDP 端口在不在
docker compose logs --tail=100 airhand
```

## 被控端

### 安装

| 系统 | 怎么装 |
|---|---|
| Windows 10 / 11 | 运行 `Airhand_<版本>_x64-setup.exe`，需要管理员权限，装到 `C:\Program Files\Airhand` |
| macOS 13+ | 打开 dmg，把 `Airhand.app` 拖进“应用程序”，第一次打开时授予**屏幕与系统录音**和**辅助功能**权限 |
| Linux | `sudo apt install ./Airhand_<版本>_amd64.deb`（只支持 X11 桌面） |

Windows 上被控端作为系统服务 **Airhand** 运行（SYSTEM），所以锁屏、登录界面、UAC 弹窗都能控制，切换用户也会跟过去。托盘图标只是普通程序，显示状态、操作服务。没有安装的 `airhand.exe` 不能直接运行：发布版会提示运行安装程序。

macOS 上被控端平时只在菜单栏里，打开控制台时才出现在程序坞。授予“屏幕与系统录音”后需要重新打开 Airhand。

Linux 的托盘图标走 AppIndicator，原版 GNOME 要装 “AppIndicator and KStatusNotifierItem Support” 扩展；没有 AppIndicator 库时程序照常作为被控端运行，控制码打印在 stderr 里。

### 指向你的服务器

两种方式，选一种：

1. **构建时写死**（批量部署推荐）：构建客户端时设置 `AIRHAND_SERVER` 和 `AIRHAND_SERVER_FP`，装好就是对的，见[构建](building.zh-CN.md#服务器地址和证书指纹)。
2. **装好后改 `config.toml`**（少量机器最省事）：

| 系统 | 位置 |
|---|---|
| Windows | `C:\Program Files\Airhand\config.toml`（要用管理员身份改） |
| macOS | `~/Library/Application Support/app.airhand.client/config.toml` |
| Linux | `~/.config/airhand/config.toml` |

```toml
server = "relay.example.com:4433"
server_fp = "<64 位十六进制指纹>"
```

`server_fp` 留空也能用：第一次连上时记住服务器证书（TOFU），之后证书变了就拒绝；但第一次挡不住中间人，正式环境建议填死。改完要重启 Airhand（Windows 上重启服务 / 托盘退出再打开）。

### 安全与无人值守

- 默认 `access = "pairing"`：没配对过的控制端第一次连接要输入托盘里的 6 位配对码（不经过服务器）。只有在自己完全信任的服务器上，才考虑 `access = "open"`，那等于信任中转服务器
- 控制码是**固定的**，保存在 `config.toml` 里，重启、升级、服务器重启都不变，直到主动“更换控制码”。被控端离线 180 天才释放
- Windows 默认开机自动启动（开机即启动服务，登录前就能被控制）；macOS 在菜单里切换“登录时自动启动”
- 被控端**不需要开放任何入站端口**，只要出站 UDP 能到服务器

## 控制端（桌面）

装的是同一个安装包，角色在托盘里切换：

- Windows：托盘 → **打开控制台**（控制台打开期间，服务里的被控端照常运行）
- macOS / Linux：托盘 → **切换为控制端**

`config.toml` 里的 `server` / `server_fp` 和被控端一样；控制台自己的身份在 `[controller]`，连接过的电脑在 `[[device]]`。Windows 上这些按账户加密（DPAPI），多用户机器上每个人一份。

控制端同样只需要**出站 UDP**。多开标签、多屏总览、画质、声音、剪贴板这些是使用问题，见[使用指南](usage.zh-CN.md#控制端)。

## 网页控制台（Docker）

控制端也可以跑在容器里：用 Docker Compose 启动，浏览器打开一个端口就是控制台，操作的电脑上不用安装或构建任何东西。界面和桌面版的控制台完全一样（多会话、多屏总览、画质、声音、剪贴板……）；被控的电脑仍然运行 Airhand 客户端。

```bash
cd web
AIRHAND_SERVER=relay.example.com:4433 AIRHAND_SERVER_FP=<服务器证书指纹> docker compose up -d
# 也可以 cp .env.example .env，改好后直接 docker compose up -d
```

然后用浏览器打开：

- 运行容器的这台电脑：`http://localhost:8080`
- 其它电脑：`https://<这台电脑的地址>:8080`

同一个端口同时支持 HTTP 和 HTTPS。浏览器只在安全的页面（HTTPS 或 localhost）上提供 WebCodecs 视频解码，所以从其它电脑访问要用 https://（用 http:// 打开时页面会给出 https 的链接）。证书是第一次启动时生成的自签名证书，保存在 `web/data/` 里（绑定挂载，保留这个目录，浏览器就不会再次提示），浏览器第一次会提示不安全，确认继续即可。需要 Chrome / Edge、Safari 16.4+ 或 Firefox 130+。

环境变量（见 `web/docker-compose.yml` 和 `web/.env.example`）：

| 变量 | 默认值 | 说明 |
|---|---|---|
| `AIRHAND_SERVER` | （必填） | 中转服务器地址 `host:port` |
| `AIRHAND_SERVER_FP` | 空 | 中转服务器的证书指纹。留空则第一次连上时记住服务器的证书（`data/server-trust.txt`），之后证书变了就拒绝连接；服务器确实换了证书时删掉这个文件 |
| `AIRHAND_NAME` | `Airhand Web` | 被控端托盘里“谁在控制”显示的名称 |
| `AIRHAND_WEB_IDENTITY` | `AIRHAND_WEB_CERT` 所在目录里的 `controller-key.txt`（镜像里是 `/data/controller-key.txt`） | 网页控制台的端到端加密身份（所有标签页共用的一把私钥，文件权限 600）。和网页控制台配对过的电脑靠它认出网页控制台；没有地方保存时只在内存里，每次重启都要重新配对。端到端加密的终点是网页控制台，它所在的机器看得到画面（见[安全](security.zh-CN.md#端到端加密)） |
| `AIRHAND_WEB_PASSWORD` | 空 | 访问密码：浏览器弹出登录框，用户名随意。留空则任何能访问这个端口的人都能使用控制台（连接每台电脑仍然需要它的控制码） |
| `AIRHAND_WEB_PORT` | `8080` | 映射到宿主机的端口 |
| `AIRHAND_WEB_CERT_NAMES` | `localhost` | 自动生成 HTTPS 证书时写入的域名 / IP（逗号分隔）。想把证书导入系统信任时，填上实际访问用的地址 |
| `AIRHAND_WEB_CERT` / `AIRHAND_WEB_KEY` | `/data/cert.pem` / `/data/key.pem` | HTTPS 证书路径。用自己的证书时放进 `web/data/`（或者改这两个路径） |
| `RUST_LOG` | `info` | 日志级别 |

- 每个浏览器标签页是一个独立的控制台，有自己到中转服务器的连接；关闭或刷新标签页会断开它的会话（有会话时浏览器会先确认）。和网页服务的连接断了会自动重连，会话随后自动恢复
- 画面要等浏览器收到后才确认，被控端的在途控制因此把“服务 → 浏览器”这段网络也算进去，这段网络慢时画面不会越积越多。工具栏显示的延迟 = 服务到被控端的往返 + 浏览器到服务的往返，所以服务最好部署在离浏览器近的地方（例如同一个局域网）
- 剪贴板用的是浏览器的剪贴板：对方复制的文本写进本机剪贴板（页面在前台时）；本机复制的文本在回到这个页面时发给对方，需要浏览器允许读取剪贴板（Chrome / Edge 第一次会询问）。Firefox 不允许网页自动读取剪贴板，只能从对方同步过来
- 不全屏时，Ctrl+W、Ctrl+T 等快捷键会被浏览器拦截；全屏时 Chrome / Edge 会锁定键盘，Win、Alt+Tab 等也发给远端
- 放在反向代理后面（例如用自己的域名和证书）：代理到容器的 `http://…:8080`，需要转发 WebSocket（`/ws`）并保留 `Host` 头（或设置 `X-Forwarded-Host`），否则会被当成跨站请求拒绝。可以放在子路径下
- 镜像基于 distroless，约 30 MB，容器里以 root 运行（和中转服务器的一样：挂载进来的 `web/data/` 不管属于谁都写得进去；用 rootful Docker 时里面的文件在宿主机上属于 root）。只需要出站的 UDP（到中转服务器）和入站的这一个 TCP 端口

## 直连与端口

| 方向 | 端口 | 谁开 |
|---|---|---|
| 被控端 → 中转服务器 | 出站 UDP 4433 | 被控端网络 |
| 控制端 → 中转服务器 | 出站 UDP 4433 | 控制端网络 |
| 服务器 | 入站 UDP 4433 | 服务器安全组 + 防火墙 |
| 浏览器 → 网页控制台 | 入站 TCP 8080 | 网页控制台那台机器 |
| 被控端 ⇄ 控制端（直连） | 复用连中转的**同一个 UDP socket** | 不用额外开 |

会话建立后两端会尝试 UDP 打洞直连，成功时画面和声音不再经过服务器（省流量、降延迟），失败就继续走中转，会话不断。打洞用的是连中转的那个本地端口，不需要在防火墙上为直连单独放行。对称 NAT 或防火墙挡 UDP 时直连打不通，属正常情况。

## 升级、备份与迁移

| 事 | 怎么做 |
|---|---|
| 升级服务器 | `git pull && docker compose up -d --build`，**不要动 `data/`** |
| 备份 | 停容器后备份整个 `data/`（`cert.pem`、`key.pem`、`airhand.db`；运行中直接拷要先 `sqlite3 airhand.db ".backup ..."` 或用文件系统快照） |
| 迁移服务器 | 把旧 `data/` 一起拷到新机器，指纹不变，客户端不用改；不拷就是新证书，所有客户端要改 `server_fp` |
| 升级客户端 | Windows 直接覆盖安装（保留 `config.toml`，控制码和“我的设备”都在）；macOS 替换 `.app`；Linux 装新 deb |
| 忘记网页控制台密码 | `AIRHAND_WEB_PASSWORD` 是启动参数，改环境变量后 `docker compose up -d` 重启即可 |
| 换服务器证书 | 删客户端 `config.toml` 里的 `[server_trust]`，或改 `server_fp` |

## 排错

| 现象 | 先查 |
|---|---|
| 被控端托盘一直是灰的 | 服务器地址 / 端口对不对；出站 UDP 4433 通不通；`docker compose logs airhand` 有没有收到连接 |
| 能连上但画面卡、延迟尖峰 | 宿主机 `rmem_max` / `wmem_max` 调了没有；日志里有没有缓冲区被截掉的警告；服务器出口带宽 |
| 客户端提示证书变了 | 服务器 `data/` 是不是被删了或换了机器；确实换了就更新 `server_fp` |
| 控制端连不上某台被控端 | 控制码对不对；被控端默认要配对码，第一次连要输入托盘里的 6 位；输错 5 次配对会暂停，换一个配对码 |
| 网页控制台打不开 | TCP 8080 放行没有；从别的电脑要用 `https://`；反代要转发 `/ws` 并保留 `Host` |
| 网页控制台能开但连不上电脑 | 容器到中转服务器的**出站 UDP** 通不通（容器网络常见只放行 TCP） |
