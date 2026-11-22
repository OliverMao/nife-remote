# 构建

[← 返回 README](../README.zh-CN.md) · [English](building.md)

客户端**只支持 Windows**（macOS / Linux 的实现已删除）。客户端连哪个账号服务**由构建决定**（见[账号服务地址](#账号服务地址)），所以发布版要先有账号服务地址；中转服务器的地址和指纹由账号服务分配，构建时不用管。

## 桌面客户端

### 依赖

| 需要 | 说明 |
|---|---|
| Windows 10 / 11 | WebView2 一般是系统自带的，没有时安装程序会自动下载安装 |
| Rust | 较新的 stable（edition 2024） |
| Node.js 20+ | 构建控制台界面 |
| CMake | 编译 libopus |

### 账号服务地址

客户端连哪里**只由构建决定**，装好之后界面和配置文件都改不了、也看不到。地址在编译时由 `client/src-tauri/build.rs`
读出来：环境变量 `NIFE_REMOTE_AUTH` → `client/.env` → 默认的正式服务 `https://nife-remote-auth.naig.us`。
什么都不设直接构建就是正式服务：

```powershell
npm run tauri build
```

自己另外部署了账号服务（见[部署](../deploy/deploy.zh-CN.md)）时，构建前设环境变量，或者写进 `client/.env`
（从 `client/.env.example` 复制）：

```powershell
$env:NIFE_REMOTE_AUTH="https://auth.example.com"     # 反代域名，不写端口
npm run tauri build
```

中转服务器的地址和指纹不用编译：账号服务的 `GET /relay` 分配，客户端自己跟上（见[配置文件](usage.zh-CN.md#配置文件)）。

### 构建

Windows（PowerShell）：

```powershell
cd client
npm install
npm run tauri build            # 只要 exe：npm run tauri build -- --no-bundle
```

中转服务器不用编译进去（由账号服务分配，见[账号服务地址](#账号服务地址)）。打包要几分钟的话，可以先把 `client/src-tauri/Cargo.toml` 里 `[profile.release]` 的 `incremental` 打开（现在已经打开），或者只在本地验证时用 `--debug`。

生成的文件都在 `client/src-tauri/target/release/` 下：

| 文件 | 说明 |
|---|---|
| `bundle/nsis/Nife Remote_<版本>_x64-setup.exe` | 安装程序 |
| `nife-remote.exe` | `--no-bundle` 时只有这个（见下面的 Windows 说明） |

### Windows 说明

- 安装程序为所有用户安装并注册系统服务（见 [Windows 安装版](usage.zh-CN.md#windows-安装版)），附加步骤在 `client/src-tauri/windows/hooks.nsh`
- `npm run tauri build -- --no-bundle` 只生成 `nife-remote.exe`（不打包 NSIS），但发布版没有安装时不能运行。手动代替安装包（管理员）：

  ```powershell
  mkdir "C:\Program Files\Nife Remote"
  copy .\client\src-tauri\target\release\nife-remote.exe "C:\Program Files\Nife Remote\"
  & "C:\Program Files\Nife Remote\nife-remote.exe" --install     # 注册并启动服务；卸载用 --uninstall
  ```

  装好之后 exe 不能再移动（服务里记的是这个路径）
- 打包速度：`[profile.release]` 用 `lto = false` + `codegen-units = 256`（多核并行）。原来 `lto = "fat"` + `codegen-units = 1` 最后一步只能单核，在这台机器上要几分钟；现在改一行代码重新打包约 45 秒，代价是 exe 大 3 MB 左右
- C 运行时是静态链接的，不需要装 VC++ 运行库
- Media Foundation 是延迟加载的，Windows N 版（没装媒体功能包）也能运行，会改用软件编码
