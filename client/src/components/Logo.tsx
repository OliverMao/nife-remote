// 应用图标：和 assets/icon.svg（Figma 导出）、托盘图标、安装包图标是同一张图。
// 图形里有 foreignObject 和 3D 变换，内联成 JSX 太脆，所以用渲染好的 PNG（256px，够 22~40px 用）。
export function Logo({ size = 28 }: { size?: number }) {
  return <img className="app-logo" src="/logo.png" width={size} height={size} alt="" aria-hidden />;
}
