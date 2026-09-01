export type Launch = Readonly<{
  kind: "new" | "resume";
  latest: boolean;
  configArgs: string[];
}>;

export function parseLaunch(argv: readonly string[]): Launch {
  const first = argv[0];
  if (first !== undefined && !first.startsWith("--") && first !== "-h" && first !== "-v") {
    if (first !== "resume") throw new Error(`unknown command ${first}`);
    const rest = argv.slice(1);
    const latest = rest.filter((value) => value === "--latest").length;
    if (latest > 1) throw new Error("--latest may be passed only once");
    return {
      kind: "resume",
      latest: latest === 1,
      configArgs: rest.filter((value) => value !== "--latest"),
    };
  }
  if (argv.includes("--latest")) throw new Error("--latest requires `jecode resume`");
  return { kind: "new", latest: false, configArgs: [...argv] };
}
