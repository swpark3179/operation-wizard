import { useEffect, useRef } from "react";

/**
 * Auto-grow a `<textarea>` with the value it holds: it expands to fit its
 * content up to `maxPx`, then stops growing and scrolls. Returns the ref to
 * attach. When `value` shrinks (e.g. cleared after send) the height snaps back.
 */
export function useAutoGrow(value: string, maxPx: number) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto"; // reset so scrollHeight reflects the content
    el.style.height = `${Math.min(el.scrollHeight, maxPx)}px`;
  }, [value, maxPx]);
  return ref;
}
