import type { DiffGranularity } from "@optimizer/protocol";
import { segmentText, type TextUnit } from "./segment.ts";
import type {
  DiffOptions,
  DiffSegment,
  DiffSegmentKind,
  TextChange,
  TextDiff,
} from "./types.ts";

const levels: readonly DiffGranularity[] = ["paragraph", "sentence", "token"];
const detailRank: Readonly<Record<DiffGranularity, number>> = {
  paragraph: 0,
  sentence: 1,
  token: 2,
};

interface DiffState {
  readonly maxMatrixCells: number;
  readonly segments: DiffSegment[];
  readonly warnings: Set<string>;
}

export function diffText(
  original: string,
  replacement: string,
  options: DiffOptions = {},
): TextDiff {
  const maxMatrixCells = options.maxMatrixCells ?? 250_000;
  if (!Number.isInteger(maxMatrixCells) || maxMatrixCells < 1) {
    throw new RangeError("maxMatrixCells must be a positive integer");
  }
  const state: DiffState = { maxMatrixCells, segments: [], warnings: new Set() };
  diffRange(original, replacement, 0, 0, 0, state);
  return {
    segments: state.segments,
    changes: collectChanges(original, state.segments),
    warnings: [...state.warnings].sort(),
  };
}

function diffRange(
  original: string,
  replacement: string,
  oldBase: number,
  newBase: number,
  levelIndex: number,
  state: DiffState,
): void {
  const granularity = levels[Math.min(levelIndex, levels.length - 1)] ?? "token";
  if (original === replacement) {
    pushSegment(state, "equal", original, granularity, oldBase, oldBase + original.length, newBase, newBase + replacement.length);
    return;
  }
  if (!original) {
    pushSegment(state, "insert", replacement, granularity, oldBase, oldBase, newBase, newBase + replacement.length);
    return;
  }
  if (!replacement) {
    pushSegment(state, "delete", original, granularity, oldBase, oldBase + original.length, newBase, newBase);
    return;
  }

  const oldUnits = segmentText(original, granularity);
  const newUnits = segmentText(replacement, granularity);
  if ((oldUnits.length + 1) * (newUnits.length + 1) > state.maxMatrixCells) {
    state.warnings.add("DIFF_COMPLEXITY_FALLBACK");
    pushSegment(state, "delete", original, granularity, oldBase, oldBase + original.length, newBase, newBase);
    pushSegment(state, "insert", replacement, granularity, oldBase + original.length, oldBase + original.length, newBase, newBase + replacement.length);
    return;
  }

  const matches = longestCommonSubsequence(oldUnits, newUnits);
  if (matches.length === 0) {
    if (levelIndex + 1 < levels.length) {
      diffRange(original, replacement, oldBase, newBase, levelIndex + 1, state);
    } else {
      pushSegment(state, "delete", original, granularity, oldBase, oldBase + original.length, newBase, newBase);
      pushSegment(state, "insert", replacement, granularity, oldBase + original.length, oldBase + original.length, newBase, newBase + replacement.length);
    }
    return;
  }

  let oldCursor = 0;
  let newCursor = 0;
  for (const [oldIndex, newIndex] of matches) {
    const oldUnit = oldUnits[oldIndex];
    const newUnit = newUnits[newIndex];
    if (!oldUnit || !newUnit) continue;
    diffRange(
      original.slice(oldCursor, oldUnit.from),
      replacement.slice(newCursor, newUnit.from),
      oldBase + oldCursor,
      newBase + newCursor,
      levelIndex + 1,
      state,
    );
    pushSegment(
      state,
      "equal",
      oldUnit.text,
      granularity,
      oldBase + oldUnit.from,
      oldBase + oldUnit.to,
      newBase + newUnit.from,
      newBase + newUnit.to,
    );
    oldCursor = oldUnit.to;
    newCursor = newUnit.to;
  }
  diffRange(
    original.slice(oldCursor),
    replacement.slice(newCursor),
    oldBase + oldCursor,
    newBase + newCursor,
    levelIndex + 1,
    state,
  );
}

function longestCommonSubsequence(
  original: readonly TextUnit[],
  replacement: readonly TextUnit[],
): readonly (readonly [number, number])[] {
  const width = replacement.length + 1;
  const table = new Uint32Array((original.length + 1) * width);
  for (let left = original.length - 1; left >= 0; left -= 1) {
    for (let right = replacement.length - 1; right >= 0; right -= 1) {
      const index = left * width + right;
      table[index] = original[left]?.text === replacement[right]?.text
        ? 1 + (table[(left + 1) * width + right + 1] ?? 0)
        : Math.max(
            table[(left + 1) * width + right] ?? 0,
            table[left * width + right + 1] ?? 0,
          );
    }
  }

  const matches: Array<readonly [number, number]> = [];
  let left = 0;
  let right = 0;
  while (left < original.length && right < replacement.length) {
    if (original[left]?.text === replacement[right]?.text) {
      matches.push([left, right]);
      left += 1;
      right += 1;
    } else if (
      (table[(left + 1) * width + right] ?? 0)
      >= (table[left * width + right + 1] ?? 0)
    ) {
      left += 1;
    } else {
      right += 1;
    }
  }
  return matches;
}

function pushSegment(
  state: DiffState,
  kind: DiffSegmentKind,
  text: string,
  granularity: DiffGranularity,
  oldFrom: number,
  oldTo: number,
  newFrom: number,
  newTo: number,
): void {
  if (!text) return;
  const segment: DiffSegment = { kind, text, granularity, oldFrom, oldTo, newFrom, newTo };
  const previous = state.segments.at(-1);
  if (
    previous
    && previous.kind === segment.kind
    && previous.granularity === segment.granularity
    && previous.oldTo === segment.oldFrom
    && previous.newTo === segment.newFrom
  ) {
    state.segments[state.segments.length - 1] = {
      ...previous,
      text: previous.text + segment.text,
      oldTo: segment.oldTo,
      newTo: segment.newTo,
    };
  } else {
    state.segments.push(segment);
  }
}

function collectChanges(original: string, segments: readonly DiffSegment[]): readonly TextChange[] {
  const changes: TextChange[] = [];
  let pending: DiffSegment[] = [];
  const flush = () => {
    if (pending.length === 0) return;
    const from = Math.min(...pending.map((segment) => segment.oldFrom));
    const to = Math.max(...pending.map((segment) => segment.oldTo));
    const replacement = pending
      .filter((segment) => segment.kind === "insert")
      .map((segment) => segment.text)
      .join("");
    const granularity = pending
      .map((segment) => segment.granularity)
      .sort((left, right) => detailRank[right] - detailRank[left])[0] ?? "token";
    changes.push({
      from,
      to,
      original: original.slice(from, to),
      replacement,
      granularity,
    });
    pending = [];
  };

  for (const segment of segments) {
    if (segment.kind === "equal") flush();
    else pending.push(segment);
  }
  flush();
  return changes;
}
