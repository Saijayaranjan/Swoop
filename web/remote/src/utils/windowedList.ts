// Pure range maths for a small windowed/virtualised list of fixed-height rows.
// The component only needs to render `[startIndex, endIndex)` and pad with `topPadding` /
// `bottomPadding` spacers to keep the scrollbar's total height correct.

export interface WindowedRange {
  startIndex: number;
  endIndex: number;
  topPadding: number;
  bottomPadding: number;
  totalHeight: number;
}

export interface WindowedListInput {
  scrollTop: number;
  viewportHeight: number;
  itemHeight: number;
  itemCount: number;
  /** Extra rows rendered above/below the viewport to reduce blank flashes while scrolling. */
  overscan?: number;
}

/** Compute the visible index range and padding for a fixed-row-height virtualised list. */
export function computeWindowedRange(input: WindowedListInput): WindowedRange {
  const { scrollTop, viewportHeight, itemHeight, itemCount } = input;
  const overscan = input.overscan ?? 3;
  const totalHeight = Math.max(0, itemCount * itemHeight);

  if (itemCount <= 0 || itemHeight <= 0) {
    return { startIndex: 0, endIndex: 0, topPadding: 0, bottomPadding: 0, totalHeight: 0 };
  }

  const safeScrollTop = Math.max(0, scrollTop);
  const safeViewport = Math.max(0, viewportHeight);

  const firstVisible = Math.floor(safeScrollTop / itemHeight);
  const visibleCount = Math.ceil(safeViewport / itemHeight) + 1;

  const startIndex = Math.max(0, firstVisible - overscan);
  const endIndex = Math.min(itemCount, firstVisible + visibleCount + overscan);

  const topPadding = startIndex * itemHeight;
  const bottomPadding = Math.max(0, totalHeight - endIndex * itemHeight);

  return { startIndex, endIndex, topPadding, bottomPadding, totalHeight };
}
