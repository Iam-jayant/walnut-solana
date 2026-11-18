"use client";

import { useEffect, useState } from "react";

/** Current unix time in seconds, refreshed every `intervalMs`. */
export function useNowSeconds(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    const timer = setInterval(
      () => setNow(Math.floor(Date.now() / 1000)),
      intervalMs,
    );
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}
