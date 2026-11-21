# 应用图标（唯一源文件）

**`icon.svg`** 是唯一的图标源文件（Figma 导出的紫色 N + 浅灰光带 + 白色圆角块）。换图标就替换这一个文件，
然后跑：

```bash
node scripts/icon.mjs
```

脚本会用系统自带的 Edge（没有就用 Chrome）把 SVG 渲染成三张 PNG，再调用 `tauri icon` 生成整套程序图标，
最后把托盘图标、界面 Logo、favicon 放到该在的位置。**不要直接改生成出来的 png / ico**，下次生成会覆盖。

为什么用浏览器渲染：这个 SVG 里有 `foreignObject` 和 3D 变换（Figma 的 3D 图层），只有浏览器引擎能画；
`sharp` / `resvg` / `librsvg` 这类光栅化库会把这些图层整个丢掉，只剩白块。

## 生成出来什么

| 用途 | 文件 |
|---|---|
| 控制台里的 Logo（左栏标题、设置弹层） | `client/public/logo.png`（256×256，`components/Logo.tsx` 引用） |
| 网页 favicon | `client/public/logo.png`（`client/index.html`） |
| exe / 安装包 / 任务栏 | `client/src-tauri/icons/icon.ico`、`icon.png`、`32x32.png`、`128x128.png`、`128x128@2x.png` |
| Microsoft Store 图标 | `client/src-tauri/icons/Square*Logo.png`、`StoreLogo.png` |
| 系统托盘 | `client/src-tauri/icons/tray.png`（64×64，离线时运行时去色） |
| 渲染用的 1024 源图 | `client/assets/icon.png` |

图标是编译进 exe 的（`tauri.conf.json` 的 `bundle.icon` 和 `windows.nsis.installerIcon`），
所以改完图标要重新 `npm run tauri build` 才会生效。

## 想换渲染浏览器

```bash
ICON_BROWSER="C:/Program Files/Google/Chrome/Application/chrome.exe" node scripts/icon.mjs
```
