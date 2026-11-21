#!/usr/bin/env node
// 从 assets/icon.svg 生成所有图标：程序图标、安装包图标、托盘图标、界面里的 Logo、网页 favicon。
//
//   node scripts/icon.mjs
//
// 图标源文件只有一个：**assets/icon.svg**（Figma 导出的那个，带 foreignObject / 3D 变换）。
// 换图标就替换这个 SVG 再跑一次本脚本，别直接改生成出来的 png / ico。
//
// 为什么用浏览器渲染：SVG 里的 foreignObject（Figma 的 3D 图层）只有浏览器引擎能画，
// sharp / resvg 这类光栅化库会把这些图层整个丢掉。这里用系统自带的 Edge（没有就用 Chrome）
// 无头渲染三张 PNG，再交给 `tauri icon` 出整套 ico / StoreLogo。
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const svgPath = join(root, "assets/icon.svg");
const client = join(root, "client");

/** 找一个能无头渲染的浏览器 */
function browser() {
  const candidates = [
    process.env.ICON_BROWSER,
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
  ].filter(Boolean);
  const found = candidates.find((p) => existsSync(p));
  if (!found) {
    console.error(
      "找不到 Edge / Chrome。装一个，或者用 ICON_BROWSER=<浏览器 exe 的路径> 指定。",
    );
    process.exit(1);
  }
  return found;
}

/** 把 SVG 放进一个正好 size×size、背景透明的页面里，无头渲染成 PNG */
function render(exe, svg, size, out) {
  const dir = mkdtempSync(join(tmpdir(), "nife-icon-"));
  const html = join(dir, "icon.html");
  writeFileSync(
    html,
    `<!doctype html><html><head><meta charset="utf-8"><style>
       html,body{margin:0;padding:0;background:transparent;overflow:hidden}
       #box{width:${size}px;height:${size}px;display:grid;place-items:center;background:transparent}
       #box svg{width:${size}px;height:${size}px}
     </style></head><body><div id="box">${svg}</div></body></html>`,
    "utf8",
  );
  execFileSync(
    exe,
    [
      "--headless=new",
      "--disable-gpu",
      "--hide-scrollbars",
      "--force-device-scale-factor=1",
      "--default-background-color=00000000",
      `--window-size=${size},${size}`,
      `--screenshot=${out}`,
      pathToFileURL(html).href,
    ],
    { stdio: "ignore" },
  );
  rmSync(dir, { recursive: true, force: true });
  if (!existsSync(out)) throw new Error(`渲染失败：${out}`);
}

const exe = browser();
const svg = readFileSync(svgPath, "utf8");
console.log(`用 ${exe} 渲染 ${svgPath}`);

// 1) 三张 PNG：tauri icon 的 1024 源图、界面 Logo（256）、托盘（64）
render(exe, svg, 1024, join(client, "assets/icon.png"));
render(exe, svg, 256, join(client, "public/logo.png"));
render(exe, svg, 64, join(client, "assets/tray.png"));

// 2) 整套程序图标（ico / png / StoreLogo）
execFileSync("npx", ["tauri", "icon", "assets/icon.png"], {
  cwd: client,
  stdio: "inherit",
  shell: true,
});

// 3) 客户端只有 Windows：删掉顺手生成的 android / ios / icns；托盘图标单独拷
const icons = join(client, "src-tauri/icons");
for (const junk of ["android", "ios", "icon.icns"]) {
  rmSync(join(icons, junk), { recursive: true, force: true });
}
copyFileSync(join(client, "assets/tray.png"), join(icons, "tray.png"));

console.log("完成：程序图标、托盘图标、界面 Logo、favicon 都更新了（重新 build 才会进 exe）。");
