export const ANSWER_TIMEOUT_PRESETS = [0, 10, 30, 60, 300] as const;

export function answerTimeoutLabel(seconds: number): string {
  if (seconds === 0) return "Never";
  // Whole minutes read as minutes; anything else stays exact, and short.
  return seconds % 60 === 0 ? `${seconds / 60} min` : `${seconds} s`;
}

export function answerTimeoutPreset(value: string | undefined): number | undefined {
  return ANSWER_TIMEOUT_PRESETS.find((seconds) => String(seconds) === value);
}
