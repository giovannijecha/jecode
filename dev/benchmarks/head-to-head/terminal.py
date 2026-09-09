"""Small PTY driver for the two real TUIs; this is not a latency renderer."""

import codecs
import fcntl
import os
import pty
import re
import select
import signal
import struct
import termios
import time
import unicodedata

CSI = re.compile(r'\x1b\[([0-?]*)([ -/]*)([@-~])')
CSI_PREFIX = re.compile(r'\x1b\[[0-?]*[ -/]*\Z')
OSC_END = re.compile(r'\x07|\x1b\\')
MAX_PENDING = 4096


class Terminal:
    def __init__(self, command, cwd, env, log):
        self.rows, self.cols = 40, 140
        self.lines = [[' '] * self.cols for _ in range(self.rows)]
        self.row = self.col = 0
        self.pending = ''
        self.decoder = codecs.getincrementaldecoder('utf-8')('replace')
        self.log = log
        self.observations = {'chunks': 0, 'bytes': 0, 'maximumChunkBytes': 0,
                             'chunksOver16KiB': 0, 'feedMilliseconds': 0.0,
                             'maximumFeedMilliseconds': 0.0}
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(cwd)
            fcntl.ioctl(1, termios.TIOCSWINSZ, struct.pack('HHHH', self.rows, self.cols, 0, 0))
            os.execvpe(command[0], command, env)
        self.dead = False
        self.closed = False

    def send(self, text):
        data = text.encode()
        while data:
            count = os.write(self.fd, data)
            data = data[count:]

    def pump(self, seconds=0.1):
        if self.dead or not select.select([self.fd], [], [], seconds)[0]:
            return
        try:
            data = os.read(self.fd, 65536)
        except OSError:
            self.dead = True
            return
        if not data:
            self.dead = True
            return
        if not self.log.closed:
            self.log.write(data)
            self.log.flush()
        self.observations['chunks'] += 1
        self.observations['bytes'] += len(data)
        self.observations['maximumChunkBytes'] = max(self.observations['maximumChunkBytes'], len(data))
        self.observations['chunksOver16KiB'] += int(len(data) > 16384)
        started = time.perf_counter()
        try:
            self.feed(self.decoder.decode(data))
        finally:
            elapsed = (time.perf_counter() - started) * 1000
            self.observations['feedMilliseconds'] += elapsed
            self.observations['maximumFeedMilliseconds'] = max(
                self.observations['maximumFeedMilliseconds'], elapsed)

    def feed(self, text):
        data = self.pending + text
        self.pending = ''
        at = 0
        while at < len(data):
            if data.startswith('\x1b[', at):
                match = CSI.match(data, at)
                if not match:
                    if not CSI_PREFIX.fullmatch(data, at):
                        raise ValueError('malformed terminal CSI sequence')
                    self._retain(data, at)
                    return
                raw, final = match[1], match[3]
                at = match.end()
                if final == 'n' and raw == '6':
                    self.send(f'\x1b[{self.row+1};{self.col+1}R')
                elif final == 'c':
                    self.send('\x1b[?1;2c')
                elif final == 'u' and raw == '?':
                    self.send('\x1b[?0u')
                elif raw.startswith('?'):
                    continue
                else:
                    parts = [int(x) if x.isdigit() else 0 for x in raw.split(';')]
                    count = parts[0] or 1
                    if final in ('H','f'):
                        self.row = min(self.rows-1, max(0,count-1))
                        self.col = min(self.cols-1, max(0,(parts[1] if len(parts)>1 else 1)-1))
                    elif final == 'G': self.col = min(self.cols-1,count-1)
                    elif final == 'd': self.row = min(self.rows-1,count-1)
                    elif final == 'A': self.row = max(0,self.row-count)
                    elif final == 'B': self.row = min(self.rows-1,self.row+count)
                    elif final == 'C': self.col = min(self.cols-1,self.col+count)
                    elif final == 'D': self.col = max(0,self.col-count)
                    elif final == 'J' and parts[0] in (2,3): self.lines = [[' ']*self.cols for _ in range(self.rows)]
                    elif final == 'J' and parts[0] == 0:
                        self.lines[self.row][self.col:] = [' ']*(self.cols-self.col)
                        for row in range(self.row+1,self.rows): self.lines[row] = [' ']*self.cols
                    elif final == 'K':
                        start,end = (0,self.cols) if parts[0]==2 else ((0,self.col+1) if parts[0]==1 else (self.col,self.cols))
                        self.lines[self.row][start:end] = [' ']*(end-start)
                continue
            if data.startswith('\x1b]', at):
                match = OSC_END.search(data, at + 2)
                if not match:
                    self._retain(data, at)
                    return
                at = match.end()
                continue
            char = data[at]
            at += 1
            if char == '\x1b':
                if at == len(data):
                    self._retain(data, at - 1)
                    return
                at += 1
            elif char == '\r': self.col = 0
            elif char == '\n':
                self.row += 1
                if self.row == self.rows:
                    self.lines.pop(0); self.lines.append([' ']*self.cols); self.row -= 1
            elif char == '\b': self.col = max(0,self.col-1)
            elif char == '\t': self.col = min(self.cols-1, (self.col//8+1)*8)
            elif char >= ' ' and not unicodedata.combining(char):
                if self.col >= self.cols:
                    self.col = 0; self.row = min(self.rows-1,self.row+1)
                self.lines[self.row][self.col] = char
                self.col += 2 if unicodedata.east_asian_width(char) in ('W','F') else 1

    def _retain(self, data, at):
        # Fail the measurement instead of retaining unlimited output or dropping bytes.
        if len(data) - at > MAX_PENDING:
            raise ValueError('incomplete terminal sequence exceeds 4096 characters')
        self.pending = data[at:]

    def text(self):
        return '\n'.join(''.join(row).rstrip() for row in self.lines)

    def wait_for(self, pattern, timeout=20):
        deadline = time.monotonic()+timeout
        while time.monotonic()<deadline:
            self.pump()
            if re.search(pattern,self.text()): return
            if self.dead: break
        raise RuntimeError(f'TUI did not reach expected state: {pattern}')

    def pause(self, seconds):
        deadline = time.monotonic()+seconds
        while time.monotonic()<deadline and not self.dead: self.pump()

    def close(self):
        if self.closed:
            return
        failure = None
        try:
            if not self.dead:
                self.send('\x03'); self.pause(0.5)
                self.send('\x03'); self.pause(0.5)
        except (OSError, ValueError) as error:
            failure = error
        try: os.killpg(self.pid,signal.SIGTERM)
        except ProcessLookupError: pass
        deadline = time.monotonic()+2
        while time.monotonic()<deadline:
            result,_ = os.waitpid(self.pid,os.WNOHANG)
            if result: break
            try: self.pump()
            except (OSError, ValueError) as error: failure = failure or error
        else:
            try: os.killpg(self.pid,signal.SIGKILL)
            except ProcessLookupError: pass
            os.waitpid(self.pid,0)
        os.close(self.fd)
        self.closed = self.dead = True
        if failure is not None:
            raise failure
