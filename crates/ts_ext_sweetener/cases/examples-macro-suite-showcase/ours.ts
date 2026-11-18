

export interface MacroCard {
  readonly name: string;
  readonly kind: "useful" | "silly";
  readonly result: string;
}

const summonedMeaning = 42;

const shout = (value: string): string => value.toUpperCase();
const optionalSubtitle: string | undefined = undefined;
const copiedNumbers: ReadonlyArray<number> = [summonedMeaning, summonedMeaning];
const missingCardResult: string | undefined = undefined;
const fallbackResult = missingCardResult ?? "backup arrived";
const pipelineResult = shout("quiet words");
const surroundedResult = `${"🎩 "}${"tiny hat"}${" 🐸"}`;
const dramaticResult = `${"behold"}!!!`;
const confettiResult = `🎉 ${"ship it"} 🎉`;
const titleResult = shout("Sweetener macro carnival");
const subtitleResult = optionalSubtitle ?? "One source module, many frameworks";
const heroResult = `🎉 ${dramaticResult} 🎉`;

function runAudit(): string[] {
  const audit: string[] = [];
  for (let index = 0; index < 3; index += 1) {
    audit.push(`macro pass ${audit.length + 1}`);
  }
  if (!(copiedNumbers.length === 2)) {
    throw new Error("the clone army lost a clone");
  }
  return audit;
}

export const showcase = {
  title: titleResult,
  subtitle: subtitleResult,
  hero: heroResult,
  copiedNumbers,
  audit: runAudit(),
  cards: [{ name: "pipeline", kind: "useful", result: pipelineResult }, { name: "fallback", kind: "useful", result: fallbackResult }, { name: "surround", kind: "silly", result: surroundedResult }, { name: "dramatic", kind: "silly", result: dramaticResult }, { name: "confetti", kind: "silly", result: confettiResult }] satisfies readonly MacroCard[]
} as const;
