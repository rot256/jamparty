export type Bounds = [left: number, bottom: number, right: number, top: number];

export interface Run {
  bounds: Bounds;
  text: string;
  baseline: number;
  size: number;
  horizontal: boolean;
  rtl: boolean;
}

interface Paragraph {
  bounds: Bounds;
  last: Run;
  lines: string[];
  leading: number;
  isolated: boolean;
}

export function union(a: Bounds, b: Bounds): Bounds {
  return [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[2], b[2]), Math.max(a[3], b[3])];
}

export function linesFor(runs: Run[]): Run[] {
  const ordered = runs.sort((a, b) => b.baseline - a.baseline);
  const lines: Run[] = [];
  for (let start = 0; start < ordered.length;) {
    const first = ordered[start++];
    if (first === undefined) break;
    const row = [first];
    while (start < ordered.length) {
      const next = ordered[start];
      if (next === undefined) break;
      if (first.baseline - next.baseline > 0.25 * Math.min(first.size, next.size)) break;
      row.push(next);
      start++;
    }
    row.sort((a, b) => a.bounds[0] - b.bounds[0]);
    const [firstRun, ...rest] = row;
    let line: Run | undefined = firstRun;
    if (line === undefined) continue;
    for (const next of rest) {
      const gap: number = next.bounds[0] - line.bounds[2];
      // Keep a gutter even when the two columns share a baseline.
      if (gap > 1.25 * Math.min(line.size, next.size) ||
          gap < -0.5 * line.size || line.rtl !== next.rtl ||
          Math.abs(line.size - next.size) > 0.25 * line.size) {
        lines.push(line);
        line = next;
      } else {
        const space: string = gap > 0.12 * line.size ? " " : "";
        line = {
          ...line, bounds: union(line.bounds, next.bounds),
          text: line.rtl ? next.text + space + line.text : line.text + space + next.text,
        };
      }
    }
    lines.push(line);
  }
  return lines;
}

export function paragraphsFor(lines: Run[], view: readonly number[]): { bounds: Bounds; text: string }[] {
  const paragraphs: Paragraph[] = [];
  const left = view[0] ?? 0;
  const right = view[2] ?? 0;
  for (const line of lines) {
    const [l, , r] = line.bounds;
    const isolated = Math.abs(l + r - left - right) < 1.2 * line.size && r - l < 0.7 * (right - left);
    let match: Paragraph | undefined;
    let bestGap = Infinity;
    if (!isolated) {
      // Only recent paragraphs can be continued, including the other column
      // on the preceding baseline. Bound the search even on crowded pages.
      for (let i = paragraphs.length - 1; i >= Math.max(0, paragraphs.length - 64); i--) {
        const previous = paragraphs[i];
        if (previous === undefined) break;
        const last = previous.last;
        const gap = last.baseline - line.baseline;
        if (previous.isolated || last.rtl !== line.rtl || Math.abs(last.size - line.size) > 0.15 * line.size ||
            gap < 0.55 * line.size || gap > (previous.leading || line.size * 1.25) * 1.35 ||
            gap >= bestGap) continue;
        const alignment = line.rtl ? r - last.bounds[2] : l - last.bounds[0];
        if (Math.abs(alignment) > 0.65 * line.size) continue;
        match = previous;
        bestGap = gap;
      }
    }
    if (match === undefined) {
      paragraphs.push({ bounds: line.bounds, last: line, lines: [line.text], leading: 0, isolated });
    } else {
      match.bounds = union(match.bounds, line.bounds);
      match.last = line;
      match.lines.push(line.text);
      match.leading ||= bestGap;
    }
  }
  return paragraphs.map(({ bounds, lines }) => ({ bounds, text: lines.join(" ") }));
}
