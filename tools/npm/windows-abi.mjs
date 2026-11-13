import fs from 'node:fs';
import {embeddedFiles, exceeds, requireFiles} from './common.mjs';

// A Windows executable records no minimum build, so its floor comes from what
// it loads. Every library and P/Invoke below was reviewed against Windows 10
// 1809 (build 17763); adding one requires checking its documented minimum first.
const SystemLibraries = /^(?:kernel32|ntdll|ws2_32|bcryptprimitives|mscoree|api-ms-win-core-synch-l1-2-0|api-ms-win-crt-[a-z]+-l1-1-0)\.dll$/i;
const SubsystemCeiling = '10.0';

// Functions the adapters bind at run time. processmodel.dll is the one optional
// library: it is newer than the floor and absence disables Workspace confinement.
// GetConsoleTitleW and SetConsoleTitleW require Windows 2000; SetLastError
// requires Windows XP, all below the Windows 10 1809 floor.
export const reviewedPInvokes = {
  'kernel32.dll': [
    'CloseHandle', 'ClosePseudoConsole', 'CreateFileW', 'CreateIoCompletionPort',
    'CreateJobObject', 'CreatePipe', 'CreateProcessW', 'CreatePseudoConsole',
    'DeleteProcThreadAttributeList', 'DuplicateHandle', 'FreeEnvironmentStringsW', 'GetConsoleMode',
    'GetConsoleOutputCP', 'GetConsoleScreenBufferInfo', 'GetConsoleScreenBufferInfoEx', 'GetConsoleTitleW', 'GetCurrentProcess',
    'GetEnvironmentStringsW', 'GetExitCodeProcess', 'GetNamedPipeServerProcessId',
    'GetNumberOfConsoleInputEvents', 'GetQueuedCompletionStatus', 'GetStdHandle',
    'InitializeProcThreadAttributeList', 'QueryInformationJobObject', 'ReadConsoleInputW',
    'ResizePseudoConsole', 'ResumeThread', 'SetConsoleMode', 'SetConsoleOutputCP', 'SetConsoleTitleW', 'SetHandleInformation',
    'SetInformationJobObject', 'SetLastError', 'SetStdHandle', 'TerminateJobObject', 'TerminateProcess',
    'UpdateProcThreadAttribute', 'WaitForSingleObject', 'WriteFile',
  ],
  'winhttp.dll': [
    'WinHttpAddRequestHeaders', 'WinHttpCloseHandle', 'WinHttpConnect', 'WinHttpOpen', 'WinHttpOpenRequest',
    'WinHttpQueryDataAvailable', 'WinHttpQueryHeaders', 'WinHttpReadData', 'WinHttpReceiveResponse',
    'WinHttpSendRequest', 'WinHttpSetOption', 'WinHttpSetStatusCallback', 'WinHttpSetTimeouts',
    'WinHttpWebSocketCompleteUpgrade', 'WinHttpWebSocketReceive', 'WinHttpWebSocketSend',
  ],
  'processmodel.dll': [
    'CloseProcessSecurityEnvironment', 'CreateProcessSecurityEnvironment',
    'IsProcessSecurityEnvironmentVersionSupported', 'QueryProcessSecurityEnvironmentSupport',
  ],
};

// The library and function each C# P/Invoke declares, as `library!function`.
export function pInvokes(source) {
  const found = [];
  const declaration = /DllImport\("([^"]+)"([^\]]*)\]\s*(?:\[[^\]]*\]\s*)*(?:(?:public|internal|private)\s+)?static\s+extern\s+[\w.<>\[\]]+\s+(\w+)\s*\(/g;
  for (const [, library, options, method] of source.matchAll(declaration)) {
    const entry = options.match(/EntryPoint\s*=\s*"([^"]+)"/)?.[1] ?? method;
    found.push(`${library.toLowerCase().replace(/(?<!\.dll)$/, '.dll')}!${entry}`);
  }
  return found;
}

// The PE optional header's subsystem version and imported library names.
export function peImage(bytes, file) {
  if (bytes.length < 64 || bytes.toString('latin1', 0, 2) !== 'MZ') throw new Error(`${file} is not a PE file.`);
  const header = bytes.readUInt32LE(0x3c);
  if (bytes.toString('latin1', header, header + 4) !== 'PE\0\0') throw new Error(`${file} is not a PE file.`);
  const optional = header + 24, wide = bytes.readUInt16LE(optional) === 0x20b;
  const sectionCount = bytes.readUInt16LE(header + 6);
  const firstSection = optional + bytes.readUInt16LE(header + 20);
  const sections = Array.from({length: sectionCount}, (_, index) => {
    const at = firstSection + index * 40;
    return {
      address: bytes.readUInt32LE(at + 12),
      size: Math.max(bytes.readUInt32LE(at + 8), bytes.readUInt32LE(at + 16)),
      raw: bytes.readUInt32LE(at + 20),
    };
  });
  const offset = address => {
    const section = sections.find(item => address >= item.address && address < item.address + item.size);
    if (!section) throw new Error(`${file} has an import outside its sections.`);
    return address - section.address + section.raw;
  };
  const libraries = [];
  const directories = optional + (wide ? 112 : 96);
  // The audit reads the import table; a delay-loaded library would escape it.
  if (bytes.readUInt32LE(directories + 13 * 8)) throw new Error(`${file} delay-loads libraries, which this audit cannot read.`);
  const directory = bytes.readUInt32LE(directories + 8);
  if (directory) {
    for (let entry = offset(directory); bytes.readUInt32LE(entry + 12); entry += 20) {
      const start = offset(bytes.readUInt32LE(entry + 12));
      libraries.push(bytes.toString('latin1', start, bytes.indexOf(0, start)));
    }
  }
  return {
    subsystem: `${bytes.readUInt16LE(optional + 48)}.${bytes.readUInt16LE(optional + 50)}`,
    libraries,
  };
}

export function checkImage({subsystem, libraries}, build, file) {
  if (exceeds(subsystem, SubsystemCeiling)) {
    throw new Error(`${file} requires Windows subsystem ${subsystem}, above ${SubsystemCeiling}.`);
  }
  for (const library of libraries) {
    if (!SystemLibraries.test(library)) {
      throw new Error(`${file} imports ${library}, not reviewed against Windows build ${build}. Check its documented minimum, then add it to tools/npm/windows-abi.mjs.`);
    }
  }
}

export function auditWindows(binary, mainBytes, build) {
  const files = embeddedFiles(binary, mainBytes, bytes => bytes.toString('latin1', 0, 2) === 'MZ');
  requireFiles(files, [
    'jecode-shell.exe', 'jecode-supervisor.exe', 'jecode-pty.exe', 'jecode-terminal.exe',
    'jecode-openai_crypto.exe', 'jecode-http.exe',
  ], 'Windows');
  files.set('jecode.exe', binary);
  for (const [name, file] of files) checkImage(peImage(fs.readFileSync(file), name), build, name);
  return [...files.keys()];
}
