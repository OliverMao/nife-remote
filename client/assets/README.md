# 应用图标

`icon.svg` 是唯一的源文件（几何和 `src/components/Logo.tsx` 一样：紫色圆角方块 + 白色屏幕 + 遥控光标）。
程序图标、安装包图标、托盘图标、网页 favicon 都是它：

| 用途 | 文件 |
|---|---|
| 网页 favicon | `client/public/icon.svg` |
| exe / 安装包 / 任务栏 | `client/src-tauri/icons/icon.ico`、`icon.png`、`32x32.png`、`128x128.png`、`128x128@2x.png` |
| Microsoft Store 图标 | `client/src-tauri/icons/Square*Logo.png`、`StoreLogo.png` |
| 系统托盘 | `client/src-tauri/icons/tray.png`（64×64） |

改图标：改 `icon.svg`（记得同步 `Logo.tsx`），然后

```bash
# 1. SVG → 1024 的 PNG（sharp 的 SVG 渲染，随便找个临时目录装一下就行）
npm i --prefix /tmp/iconbuild sharp
node -e "const s=require('/tmp/iconbuild/node_modules/sharp');const fs=require('fs');\
const svg=fs.readFileSync('assets/icon.svg');\
(async()=>{await s(svg,{density:384}).resize(1024,1024).png().toFile('assets/icon.png');\
await s(svg,{density:384}).resize(64,64).png().toFile('assets/tray.png');})()"

# 2. 生成整套 ico / png / StoreLogo（会顺带生成 android、ios、icns，客户端只有 Windows，删掉）
cd client && npx tauri icon assets/icon.png
rm -rf src-tauri/icons/{android,ios,icon.icns}
cp assets/tray.png src-tauri/icons/tray.png
```

图标是编译进 exe 的（`tauri.conf.json` 的 `bundle.icon`），改完要重新 build 才会生效。
