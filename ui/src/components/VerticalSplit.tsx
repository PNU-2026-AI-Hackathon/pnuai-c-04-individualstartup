import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode
} from "react";

const splitRatios = new Map<string, number>();

export function VerticalSplit({
  storageKey,
  defaultRatio = 58,
  minRatio = 25,
  maxRatio = 75,
  upperLabel,
  lowerLabel,
  className = "",
  children
}: {
  storageKey: string;
  defaultRatio?: number;
  minRatio?: number;
  maxRatio?: number;
  upperLabel: string;
  lowerLabel: string;
  className?: string;
  children: [ReactNode, ReactNode];
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const activePointerIdRef = useRef<number | null>(null);
  const [ratio, setRatio] = useState(() => readRatio(storageKey, defaultRatio, minRatio, maxRatio));
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    splitRatios.set(storageKey, ratio);
    try {
      window.sessionStorage.setItem(storageKey, String(ratio));
    } catch {
      // In-memory persistence still covers environments without sessionStorage.
    }
  }, [ratio, storageKey]);

  useEffect(() => {
    if (!dragging) return;
    document.body.classList.add("split-resizing");
    return () => {
      document.body.classList.remove("split-resizing");
    };
  }, [dragging]);

  function updateFromClientY(clientY: number) {
    const bounds = containerRef.current?.getBoundingClientRect();
    if (!bounds?.height) return;
    setRatio(clamp(((clientY - bounds.top) / bounds.height) * 100, minRatio, maxRatio));
  }

  function handlePointerDown(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    activePointerIdRef.current = event.pointerId;
    setDragging(true);
    updateFromClientY(event.clientY);
  }

  function handlePointerMove(event: PointerEvent<HTMLDivElement>) {
    if (activePointerIdRef.current !== event.pointerId) return;
    updateFromClientY(event.clientY);
  }

  function handleEnd(event: PointerEvent<HTMLDivElement>) {
    if (activePointerIdRef.current !== event.pointerId) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    activePointerIdRef.current = null;
    setDragging(false);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const step = event.shiftKey ? 10 : 2;
    let nextRatio: number | undefined;
    if (event.key === "ArrowUp") nextRatio = ratio - step;
    if (event.key === "ArrowDown") nextRatio = ratio + step;
    if (event.key === "Home") nextRatio = minRatio;
    if (event.key === "End") nextRatio = maxRatio;
    if (nextRatio === undefined) return;
    event.preventDefault();
    setRatio(clamp(nextRatio, minRatio, maxRatio));
  }

  const style = {
    "--split-upper": `${ratio}fr`,
    "--split-lower": `${100 - ratio}fr`
  } as CSSProperties;

  return (
    <div
      className={`vertical-split ${dragging ? "is-dragging" : ""} ${className}`.trim()}
      data-split-key={storageKey}
      ref={containerRef}
      style={style}
    >
      <div className="vertical-split-pane vertical-split-upper">{children[0]}</div>
      <div
        aria-label={`Resize ${upperLabel} and ${lowerLabel}`}
        aria-orientation="horizontal"
        aria-valuemax={maxRatio}
        aria-valuemin={minRatio}
        aria-valuenow={Math.round(ratio)}
        className="vertical-split-handle"
        onKeyDown={handleKeyDown}
        onPointerCancel={handleEnd}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handleEnd}
        role="separator"
        tabIndex={0}
        title={`Drag or use arrow keys to resize ${upperLabel} and ${lowerLabel}`}
      >
        <span aria-hidden="true" />
      </div>
      <div className="vertical-split-pane vertical-split-lower">{children[1]}</div>
    </div>
  );
}

function readRatio(storageKey: string, fallback: number, min: number, max: number): number {
  const memoryValue = splitRatios.get(storageKey);
  if (memoryValue !== undefined) return clamp(memoryValue, min, max);
  try {
    const storedValue = window.sessionStorage.getItem(storageKey);
    if (storedValue !== null && storedValue.trim() !== "") {
      const parsedValue = Number(storedValue);
      if (Number.isFinite(parsedValue)) return clamp(parsedValue, min, max);
    }
  } catch {
    // Fall through to the default ratio.
  }
  return clamp(fallback, min, max);
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
