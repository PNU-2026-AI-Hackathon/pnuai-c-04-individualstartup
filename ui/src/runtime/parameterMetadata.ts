import type { CadParameter } from "../protocol";

export function parameterHashInput(parameters: CadParameter[]): string {
  return JSON.stringify(parameterValues(parameters));
}

export function parameterValues(parameters: CadParameter[]): Record<string, CadParameter["value"]> {
  const entries: Array<[string, CadParameter["value"]]> = parameters.map((parameter) => {
    assertParameterName(parameter.name);
    if (
      parameter.type === "number" &&
      (typeof parameter.value !== "number" || !Number.isFinite(parameter.value))
    ) {
      throw new Error(
        `Parameter metadata "${parameter.name}" must contain a finite number; received ${String(parameter.value)}.`
      );
    }
    return [parameter.name, parameter.value];
  });
  return Object.fromEntries(entries.sort((left, right) => left[0].localeCompare(right[0])));
}

function assertParameterName(name: string): void {
  if (/^[A-Za-z_]\w*$/.test(name)) return;
  throw new Error(`Parameter metadata name "${name}" is not a valid OpenSCAD identifier.`);
}
