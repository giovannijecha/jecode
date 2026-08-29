// Hand-written argument checks. A schema validator would be a dependency; a
// tool takes three arguments and this is the whole job.
//
// Every throw here becomes an is_error tool result the model can read and
// correct on the next step, so the messages are written for that reader.

export function requireString(args: Record<string, unknown>, name: string): string {
  const value = args[name];
  if (typeof value !== "string" || value === "") {
    throw new Error(`"${name}" is required and must be a non-empty string`);
  }
  return value;
}

export function optionalString(
  args: Record<string, unknown>,
  name: string,
): string | undefined {
  const value = args[name];
  if (value === undefined || value === null) return undefined;
  if (typeof value !== "string") throw new Error(`"${name}" must be a string`);
  return value;
}

export function optionalInt(
  args: Record<string, unknown>,
  name: string,
): number | undefined {
  const value = args[name];
  if (value === undefined || value === null) return undefined;
  const n = typeof value === "string" ? Number(value) : value;
  if (typeof n !== "number" || !Number.isInteger(n)) {
    throw new Error(`"${name}" must be an integer`);
  }
  return n;
}

export function optionalBool(
  args: Record<string, unknown>,
  name: string,
): boolean | undefined {
  const value = args[name];
  if (value === undefined || value === null) return undefined;
  if (typeof value !== "boolean") throw new Error(`"${name}" must be a boolean`);
  return value;
}
