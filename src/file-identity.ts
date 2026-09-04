// Portable-enough identity checks for named files and directories.

export type FileIdentity = Readonly<{
  dev: bigint;
  ino: bigint;
  birthtimeNs: bigint;
}>;

export function fileIdentity(
  details: Readonly<{ dev: bigint; ino: bigint; birthtimeNs: bigint }>,
): FileIdentity {
  return Object.freeze({
    dev: details.dev,
    ino: details.ino,
    birthtimeNs: details.birthtimeNs,
  });
}

export function sameFileIdentity(left: FileIdentity, right: FileIdentity): boolean {
  return left.dev === right.dev && left.ino === right.ino &&
    left.birthtimeNs === right.birthtimeNs;
}
