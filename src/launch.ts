export type Launch = Readonly<{
  kind: "new" | "resume";
  latest: boolean;
  configArgs: string[];
}>;

export function parseLaunch(argv: readonly string[]): Launch {
  if (argv.includes("--latest")) {
    throw new Error("--latest has been renamed to --last; use `jecode -c` or `jecode resume --last`");
  }
  const first = argv[0];
  if (first === "resume" || first === "-c") {
    const rest = first === "-c" ? ["--last", ...argv.slice(1)] : argv.slice(1);
    const latest = rest.filter((value) => value === "--last").length;
    if (latest > 1) throw new Error("--last may be passed only once; -c already selects the last session");
    return {
      kind: "resume",
      latest: latest === 1,
      configArgs: rest.filter((value) => value !== "--last"),
    };
  }
  if (first !== undefined && !first.startsWith("--") && first !== "-h" && first !== "-v") {
    throw new Error(`unknown command ${first}`);
  }
  if (argv.includes("--last")) throw new Error("--last requires `jecode resume`; use `jecode -c` to continue directly");
  return { kind: "new", latest: false, configArgs: [...argv] };
}
