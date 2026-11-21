# 构建

[← 返回 README](../README.zh-CN.md) · [English](building.md)

客户端**只支持 Windows**（macOS / Linux 的实现已删除）。客户端**不需要**编译时就知道中转服务器：装好之后在客户端里填就行，所以构建前不必先去部署服务器。想做成装机即用的安装包时，再把地址和指纹编译进去作为默认值（见[服务器地址和证书指纹](#服务器地址和证书指纹)）。

## 桌面客户端

### 依赖

| 需要 | 说明 |
|---|---|
| Windows 10 / 11 | WebView2 一般是系统自带的，没有时安装程序会自动下载安装 |
| Rust | 较新的 stable（edition 2024） |
| Node.js 20+ | 构建控制台界面 |
| CMake | 编译 libopus |

### 服务器地址和证书指纹

客户端**构建时不需要知道服务器**：这两项都不填直接构建，装好之后在客户端里填（右上角服务器状态 → **修改** → 地址 + 可选的指纹 → **保存并重启**）。

想做成装机即用的安装包，就用环境变量 `NIFE_REMOTE_SERVER` 和 `NIFE_REMOTE_SERVER_FP` 把默认值编译进去，或者直接改 `client/src-tauri/src/config.rs` 里的默认值：

- 不设置 `NIFE_REMOTE_SERVER` 时，在装好的客户端里设之前用 `127.0.0.1:4433`
- 指纹的格式不对（不是 64 位十六进制，允许用 `:` 或空格分隔）时构建会直接报错
- 它们只是默认值，装好之后 `config.toml` 里的 `server`、`server_fp` 优先（见[配置文件](usage.zh-CN.md#配置文件)）。那里的 `server_fp` 写错了会拒绝所有连接，而不是不校验

> [!NOTE]
> `NIFE_REMOTE_SERVER_FP` 留空时，客户端第一次连上服务器时记住它的证书（`config.toml` 里的 `[server_trust]`），之后证书变了就拒绝连接（首次连接时信任，控制台左上角显示“首次连接时信任”）。只有第一次连接防不了中间人，正式发布时请固定指纹。服务器有意换了证书时，删掉 `config.toml` 里的 `[server_trust]`。

### 构建

Windows（PowerShell）：

```powershell
cd client
npm install
npm run tauri build            # 只要 exe：npm run tauri build -- --no-bundle
```

服务器地址不用编译进去（装好后在客户端里填，见[服务器地址和证书指纹](#服务器地址和证书指纹)）。打包要几分钟的话，可以先把 `client/src-tauri/Cargo.toml` 里 `[profile.release]` 的 `incremental` 打开（现在已经打开），或者只在本地验证时用 `--debug`。

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
