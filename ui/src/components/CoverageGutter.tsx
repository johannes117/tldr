import type { CoverageLineState } from "../api";

export function CoverageGutter({ state }: { state: CoverageLineState | undefined }) {
  if (!state) return <span className="inline-block w-1.5" />;
  const color =
    state === "covered" ? "bg-green-500"
    : state === "uncovered" ? "bg-red-500"
    : "bg-slate-300";
  const title =
    state === "covered" ? "covered by tests"
    : state === "uncovered" ? "not covered by tests"
    : "not instrumented";
  return <span className={`inline-block w-1.5 h-4 align-middle ${color}`} title={title} />;
}
