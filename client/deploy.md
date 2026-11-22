# 客户端：连接域名、调试运行、发布安装包

## 1. 连接域名配在哪

**只有一个地方：构建时的账号服务地址**，优先级 环境变量 → `client/.env` → 默认值（正式服务
`https://nife-remote-auth.naig.us`）。什么都不设，直接构建就是正式服务：

```powershell
cd client
npm run tauri build
```

自己另外部署了账号服务，二选一：

```powershell
$env:NIFE_REMOTE_AUTH="https://auth.example.com"   # 环境变量，反代域名、不写端口
npm run tauri build
```

```ini
# client/.env（从 client/.env.example 复制）
NIFE_REMOTE_AUTH=https://auth.example.com
```

装好之后没有输入框、`config.toml` 里也没有这一项，界面既不显示也改不了——要换账号服务只能重新打包、重新安装。

中转服务器的地址和指纹不用配：账号服务的 `GET /relay` 分配，客户端自己跟上（存在 `config.toml` 的 `[relay]` 里，
改了没用，下次连上就换回来）。

## 2. debug 运行

```powershell
cd client
npm run tauri dev                                   # 编译 + 运行，前端热重载
```

只要 exe、不启动开发服务器：

```powershell
cd client
npm run tauri build -- --debug --no-bundle
.\client\src-tauri\target\debug\nife-remote.exe
```

`npm run tauri dev` 卡在端口 1420 被占用时，先结束旧的 vite：

```powershell
Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
  Where-Object { $_.CommandLine -match 'vite' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
```

用 WebView2 调试端口看界面（可选）：

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333"
npm run tauri dev
```

## 3. 打安装包

版本号三处保持一致：

- `client/package.json`
- `client/src-tauri/tauri.conf.json`
- `client/src-tauri/Cargo.toml`

```powershell
cd client
npm ci
npm run tauri build
```

产物：

| 文件 | 说明 |
| --- | --- |
| `client\src-tauri\target\release\nife-remote.exe` | 程序本体 |
| `client\src-tauri\target\release\bundle\nsis\Nife Remote_<版本>_x64-setup.exe` | 安装程序，发给用户的就是这个 |

只出 exe、不打安装包：`npm run tauri build -- --no-bundle`。这种 exe 没经过安装不能直接运行；手动放的话（管理员）：

```powershell
mkdir "C:\Program Files\Nife Remote"
copy .\client\src-tauri\target\release\nife-remote.exe "C:\Program Files\Nife Remote\"
copy .\client\src-tauri\target\debug\config.toml      "C:\Program Files\Nife Remote\config.toml"   # 有的话
```

## 4. 发布

打 tag、建 release、传安装包：

```powershell
cd D:\a_new_project\airhand
git tag v0.0.1
git push --tags

winget install --id GitHub.cli -e                   # 没装 gh 时
gh release create v0.0.1 "client\src-tauri\target\release\bundle\nsis\Nife Remote_0.0.1_x64-setup.exe" --title "Nife Remote 0.0.1" --notes "首个版本"
```

不装 `gh`：浏览器打开 `https://github.com/OliverMao/nife-remote/releases/new`，把 exe 拖进去发布。

发布前自检：

- 安装包名里的版本号和 `.env` / 镜像 tag 一致
- 全新的 Windows 账户上装一遍：能启动、能连账号服务、托盘图标正常
- 关窗口是隐藏到托盘，托盘菜单两项，右键有控制码和配对码
