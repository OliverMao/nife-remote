// Nife Remote 的系列图标：和 NifeCode 同一套 —— 紫色圆角方块 + 白色图形（平涂，不用渐变）。
//
// 侧栏、卡片标题这些"功能入口"用它；界面里的小按钮（关闭、刷新……）仍用 Icons.tsx 的线性图标。
export type TileGlyph = "host" | "devices" | "connect" | "settings" | "assist";

const GLYPHS: Record<TileGlyph, React.ReactNode> = {
  // 本机被控：一块屏幕，右上角有向外发的信号
  host: (
    <>
      <rect x="6.5" y="8" width="17" height="12" rx="2.5" />
      <path d="M11 23h8M15 20v3" />
      <path d="M19.5 5.5a5 5 0 0 1 3 3" strokeOpacity="0.65" />
    </>
  ),
  // 我的设备：两台电脑
  devices: (
    <>
      <rect x="4.5" y="7" width="13" height="10" rx="2.2" />
      <path d="M8 20h6M11 17v3" />
      <rect x="16" y="11" width="7.5" height="6" rx="1.8" strokeOpacity="0.75" />
    </>
  ),
  // 远程协助 / 新建连接：箭头进入屏幕
  connect: (
    <>
      <rect x="5" y="6.5" width="18" height="13" rx="2.5" />
      <path d="M11 23h6M14 19.5v3.5" />
      <path d="M9.5 13h8M14.5 9.8 17.8 13l-3.3 3.2" />
    </>
  ),
  // 设置：两条滑杆
  settings: (
    <>
      <path d="M7 6.5v11M7 21v-1.5M17 6.5V8M17 12v9" />
      <circle cx="7" cy="19" r="2.2" />
      <circle cx="17" cy="10" r="2.2" />
    </>
  ),
  // 远程协助（UU 那个"开始协助"）：两只手/往回箭头
  assist: (
    <>
      <path d="M6 10h6.5a2.5 2.5 0 0 1 0 5H9" />
      <path d="M9 4.5 5 8l4 3.5" />
      <path d="M18 14h-6.5a2.5 2.5 0 0 1 0-5H15" />
      <path d="M15 19.5l4-3.5-4-3.5" />
    </>
  ),
};

/** `size`：整块图标的边长（方块），图形按 24×24 画 */
export function TileIcon({ glyph, size = 24 }: { glyph: TileGlyph; size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden className="tile-icon">
      <rect width="24" height="24" rx="6.5" fill="var(--tile, #6d4ae0)" />
      <g
        fill="none"
        stroke="#fff"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
        transform="translate(0 0)"
      >
        {GLYPHS[glyph]}
      </g>
    </svg>
  );
}
