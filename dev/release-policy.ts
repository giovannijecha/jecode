// Shared release-channel rules for tag validation and package documentation.

const NEXT_INSTALL = "@giovannijecha/jecode@next";

export type ReleaseChannel = "latest" | "next";

export function releaseChannel(version: string): ReleaseChannel {
  return version.split("+", 1)[0]?.includes("-") === true ? "next" : "latest";
}

export function assertReleaseDocumentation(version: string, readme: string): void {
  if (releaseChannel(version) === "latest" && readme.includes(NEXT_INSTALL)) {
    throw new Error("stable release documentation must not advertise an inactive npm next tag");
  }
}
