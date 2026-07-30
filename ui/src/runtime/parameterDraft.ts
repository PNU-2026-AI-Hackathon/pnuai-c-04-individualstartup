import type { CadParameter } from "../protocol";

export class ParameterDraftError extends Error {
  constructor(readonly code: "undefined-parameter" | "invalid-parameter-name" | "invalid-number", message: string) {
    super(message);
  }
}

export function updateParameterDraft(
  parameters: CadParameter[],
  name: string,
  value: CadParameter["value"]
): CadParameter[] {
  assertParameterName(name);
  let updated = false;
  const nextParameters = parameters.map((parameter) => {
    assertParameterName(parameter.name);
    if (parameter.name !== name) return parameter;
    updated = true;
    return { ...parameter, value: normalizeParameterValue(parameter, value) };
  });
  if (!updated) {
    throw new ParameterDraftError("undefined-parameter", `Parameter "${name}" is not defined in this revision.`);
  }
  return nextParameters;
}

export function applyParameterValuesToSource(source: string, parameters: CadParameter[]): string {
  if (parameters.length === 0) return source;
  const values = new Map(parameters.map((parameter) => {
    assertParameterName(parameter.name);
    return [parameter.name, scadLiteral(parameter.value, parameter.name)];
  }));
  return source
    .split(/\r?\n/)
    .map((line) => {
      const match = /^(\s*([A-Za-z_]\w*)\s*=\s*)([^;]*)(;.*\/\/\s*@param\b.*)$/.exec(line);
      if (!match || !values.has(match[2])) return line;
      return `${match[1]}${values.get(match[2])}${match[4]}`;
    })
    .join("\n");
}

export function parameterHashInput(parameters: CadParameter[]): string {
  return JSON.stringify(parameterValues(parameters));
}

export function parameterValues(parameters: CadParameter[]): Record<string, CadParameter["value"]> {
  const entries: Array<[string, CadParameter["value"]]> = parameters.map((parameter) => {
    assertParameterName(parameter.name);
    if (parameter.type === "number") scadLiteral(parameter.value, parameter.name);
    return [parameter.name, parameter.value];
  });
  return Object.fromEntries(entries.sort((left, right) => left[0].localeCompare(right[0])));
}

function normalizeParameterValue(parameter: CadParameter, value: CadParameter["value"]): CadParameter["value"] {
  if (parameter.type !== "number") return value;
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return Number.isFinite(Number(parameter.value)) ? Number(parameter.value) : parameter.min ?? 0;
  const min = parameter.min;
  const max = parameter.max;
  if (typeof min === "number" && numeric < min) return min;
  if (typeof max === "number" && numeric > max) return max;
  return numeric;
}

export function scadLiteral(value: CadParameter["value"], parameterName = "parameter") {
  if (typeof value === "string") return JSON.stringify(value);
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw new ParameterDraftError(
        "invalid-number",
        `Parameter "${parameterName}" must be a finite number; received ${String(value)}.`
      );
    }
    return Object.is(value, -0) ? "0" : String(value);
  }
  return String(value);
}

function assertParameterName(name: string): void {
  if (/^[A-Za-z_]\w*$/.test(name)) return;
  throw new ParameterDraftError(
    "invalid-parameter-name",
    `Parameter "${name}" is not a valid OpenSCAD identifier.`
  );
}
