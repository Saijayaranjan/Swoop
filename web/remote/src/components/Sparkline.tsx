// Tiny live sparkline drawn on <canvas> for the dashboard's download/upload speed history.
// No charting library: two filled paths on a fixed-height canvas, redrawn on each data push.

import { useEffect, useRef } from "preact/hooks";
import type { SpeedPoint } from "../state/tasks.ts";

export interface SparklineProps {
  points: readonly SpeedPoint[];
  height?: number;
  downloadColor?: string;
  uploadColor?: string;
}

export function Sparkline({ points, height = 48, downloadColor = "#3b82f6", uploadColor = "#f59e0b" }: SparklineProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
    const width = canvas.clientWidth || 300;
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    if (points.length < 2) {
      return;
    }

    const maxSpeed = Math.max(1, ...points.map((p) => Math.max(p.download, p.upload)));
    const stepX = width / Math.max(1, points.length - 1);

    const drawLine = (values: number[], color: string): void => {
      ctx.beginPath();
      values.forEach((value, i) => {
        const x = i * stepX;
        const y = height - (value / maxSpeed) * (height - 4) - 2;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.strokeStyle = color;
      ctx.lineWidth = 1.5;
      ctx.stroke();
    };

    drawLine(points.map((p) => p.download), downloadColor);
    drawLine(points.map((p) => p.upload), uploadColor);
  }, [points, height, downloadColor, uploadColor]);

  return (
    <canvas
      ref={canvasRef}
      class="sparkline"
      style={{ height: `${height}px`, width: "100%" }}
      role="img"
      aria-label="Recent download and upload speed"
    />
  );
}
