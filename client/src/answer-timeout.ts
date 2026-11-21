export const ANSWER_TIMEOUT_PRESETS = [0, 10, 30, 60, 300] as const;

export function answerTimeoutLabel(seconds: number): string {
  if (seconds === 0) return "Never";
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  if (minutes === 0) return `${seconds} s`;
  return remainder === 0 ? `${minutes} min` : `${minutes} min ${remainder} s`;
}

export function answerTimeoutPreset(value: string | undefined): number | undefined {
  return ANSWER_TIMEOUT_PRESETS.find((seconds) => String(seconds) === value);
}
