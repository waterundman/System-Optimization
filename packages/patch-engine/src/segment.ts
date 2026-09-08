import type { DiffGranularity } from "@optimizer/protocol";

export interface TextUnit {
  readonly text: string;
  readonly from: number;
  readonly to: number;
}

const sentenceEnd = /[。！？!?；;]/u;
const sentenceCloser = /[”’」』】）》〉]/u;
const whitespace = /\s/u;
const han = /\p{Script=Han}/u;
const word = /[\p{Letter}\p{Number}_]/u;
const combining = /[\p{Mark}\uFE0E\uFE0F\u{1F3FB}-\u{1F3FF}]/u;

export function segmentText(text: string, granularity: DiffGranularity): readonly TextUnit[] {
  if (!text) return [];
  if (granularity === "paragraph") return segmentParagraphs(text);
  if (granularity === "sentence") return segmentSentences(text);
  return segmentTokens(text);
}

function segmentParagraphs(text: string): readonly TextUnit[] {
  const units: TextUnit[] = [];
  let from = 0;
  let index = 0;
  while (index < text.length) {
    if (text[index] === "\r" || text[index] === "\n") {
      index += text[index] === "\r" && text[index + 1] === "\n" ? 2 : 1;
      units.push({ text: text.slice(from, index), from, to: index });
      from = index;
    } else {
      index += codePointLengthAt(text, index);
    }
  }
  if (from < text.length) units.push({ text: text.slice(from), from, to: text.length });
  return units;
}

function segmentSentences(text: string): readonly TextUnit[] {
  const units: TextUnit[] = [];
  let from = 0;
  let index = 0;
  while (index < text.length) {
    const character = codePointAt(text, index);
    index += character.length;
    if (!sentenceEnd.test(character) && character !== "\r" && character !== "\n") continue;
    if (character === "\r" && text[index] === "\n") index += 1;
    while (index < text.length) {
      const closer = codePointAt(text, index);
      if (!sentenceCloser.test(closer)) break;
      index += closer.length;
    }
    units.push({ text: text.slice(from, index), from, to: index });
    from = index;
  }
  if (from < text.length) units.push({ text: text.slice(from), from, to: text.length });
  return units;
}

function segmentTokens(text: string): readonly TextUnit[] {
  const units: TextUnit[] = [];
  let index = 0;
  while (index < text.length) {
    const from = index;
    const character = codePointAt(text, index);
    if (whitespace.test(character)) {
      index += character.length;
      while (index < text.length && whitespace.test(codePointAt(text, index))) {
        index += codePointLengthAt(text, index);
      }
    } else if (word.test(character) && !han.test(character)) {
      index += character.length;
      while (index < text.length) {
        const next = codePointAt(text, index);
        if (!word.test(next) || han.test(next)) break;
        index += next.length;
      }
    } else if (han.test(character)) {
      index += character.length;
    } else {
      index += character.length;
      while (index < text.length && combining.test(codePointAt(text, index))) {
        index += codePointLengthAt(text, index);
      }
      while (text[index] === "\u200d" && index + 1 < text.length) {
        index += 1;
        index += codePointLengthAt(text, index);
        while (index < text.length && combining.test(codePointAt(text, index))) {
          index += codePointLengthAt(text, index);
        }
      }
    }
    units.push({ text: text.slice(from, index), from, to: index });
  }
  return units;
}

function codePointAt(text: string, offset: number): string {
  return String.fromCodePoint(text.codePointAt(offset) ?? 0);
}

function codePointLengthAt(text: string, offset: number): number {
  return codePointAt(text, offset).length;
}
