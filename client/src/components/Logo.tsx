// 应用图标（和 assets/icon.svg、托盘图标、安装包图标同一套）：平涂紫色圆角方块 + 白色屏幕和遥控光标。
// 不用渐变、不用阴影，缩到 16px 也认识。
export function Logo({ size = 28 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 1024 1024" aria-hidden>
      <rect x="32" y="32" width="960" height="960" rx="224" fill="#6D4AE0" />
      {/* 屏幕 */}
      <rect x="248" y="284" width="528" height="352" rx="56" fill="none" stroke="#fff" strokeWidth="56" />
      {/* 底座 */}
      <path d="M 378 740 L 646 740" fill="none" stroke="#fff" strokeWidth="56" strokeLinecap="round" />
      <path d="M 512 636 L 512 740" fill="none" stroke="#fff" strokeWidth="56" strokeLinecap="round" />
      {/* 遥控光标 */}
      <path
        fill="#fff"
        stroke="#fff"
        strokeWidth="40"
        strokeLinejoin="round"
        d="M 452 392 L 452 616 L 508 568 L 542 644 L 604 618 L 570 542 L 646 542 Z"
      />
    </svg>
  );
}
