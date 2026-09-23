// Small windowed/virtualised list: renders only the rows near the viewport, using
// `computeWindowedRange` (unit-tested in src/utils/windowedList.test.ts) for the maths.

import { useRef, useState } from "preact/hooks";
import type { JSX } from "preact";
import { computeWindowedRange } from "../utils/windowedList.ts";

export interface WindowedListProps<T> {
  items: readonly T[];
  itemHeight: number;
  height: number;
  renderItem: (item: T, index: number) => JSX.Element;
  getKey: (item: T) => string;
  overscan?: number;
  emptyState?: JSX.Element;
}

export function WindowedList<T>({ items, itemHeight, height, renderItem, getKey, overscan, emptyState }: WindowedListProps<T>) {
  const [scrollTop, setScrollTop] = useState(0);
  const containerRef = useRef<HTMLDivElement | null>(null);

  if (items.length === 0 && emptyState) {
    return (
      <div class="windowed-list-empty" style={{ height: `${height}px` }}>
        {emptyState}
      </div>
    );
  }

  const range = computeWindowedRange({
    scrollTop,
    viewportHeight: height,
    itemHeight,
    itemCount: items.length,
    overscan,
  });

  const visible = items.slice(range.startIndex, range.endIndex);

  return (
    <div
      ref={containerRef}
      class="windowed-list"
      style={{ height: `${height}px`, overflowY: "auto" }}
      onScroll={(e) => setScrollTop((e.currentTarget as HTMLDivElement).scrollTop)}
    >
      <div style={{ height: `${range.topPadding}px` }} aria-hidden="true" />
      {visible.map((item, i) => (
        <div key={getKey(item)} style={{ height: `${itemHeight}px` }}>
          {renderItem(item, range.startIndex + i)}
        </div>
      ))}
      <div style={{ height: `${range.bottomPadding}px` }} aria-hidden="true" />
    </div>
  );
}
