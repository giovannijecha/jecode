// Windows HTTP boundary through WinHTTP. This helper is embedded at build time.
// It serves requests in order and exits when stdin closes or after an idle
// period, so consecutive requests reuse one WinHTTP session and its connections.
// Stdin, per request: base64 URL, base64 method, timeout milliseconds (0: no total
// limit), header count, base64 name/value pairs, body length, then the raw body.
// Stdout, per request: frames of one tag byte, a little-endian 32-bit length and
// that many bytes. 'A': the request arrived and nothing was sent yet. 'D': HTTP
// headers followed by the body. 'E': the response ended; empty on success,
// otherwise the failure message.
// WinHTTP uses the system proxy configuration (automatic proxy mode);
// HTTP(S)_PROXY variables are not read.
using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Globalization;

public static partial class JecodeHttp {
    const uint NoProxy = 1;
    const uint AutomaticProxy = 4;
    const uint Secure = 0x00800000;
    const uint DisableFeature = 63;
    const uint DisableCookiesRedirectsAuth = 0x00000007;
    const uint EnableFeature = 79;
    const uint EnableSslRevocation = 0x00000001;
    const uint EnableHttpProtocol = 133;
    const uint Http2 = 1;
    const uint RawHeadersCrlf = 22;
    const uint AddHeader = 0x20000000;
    const uint ConnectNotifications = 0x0000000C;
    const uint ConnectingToServer = 0x00000004;
    const int InsufficientBuffer = 122;
    const int RequestTimeout = 12002;
    const int ConnectionError = 12030;
    const int ConnectionReset = 12031;
    const int InvalidServerResponse = 12152;
    // Servers close idle connections on their own schedule, so a longer idle
    // helper would mostly keep a process alive for connections already gone.
    const int IdleLimit = 300000;
    const int ConnectionHandleLimit = 32;
    const int StaleRetries = 2;

    delegate void StatusCallback(IntPtr handle, IntPtr context, uint status, IntPtr info, uint length);

    [DllImport("winhttp.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr WinHttpOpen(string agent, uint access, string proxy, string bypass, uint flags);
    [DllImport("winhttp.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr WinHttpConnect(IntPtr session, string server, ushort port, uint reserved);
    [DllImport("winhttp.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr WinHttpOpenRequest(IntPtr connection, string method, string path,
        IntPtr version, IntPtr referrer, IntPtr acceptTypes, uint flags);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern bool WinHttpSetTimeouts(IntPtr handle, int resolve, int connect, int send, int receive);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern bool WinHttpSetOption(IntPtr handle, uint option, ref uint value, uint size);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern IntPtr WinHttpSetStatusCallback(IntPtr handle, StatusCallback callback, uint flags, IntPtr reserved);
    [DllImport("winhttp.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool WinHttpAddRequestHeaders(IntPtr request, string headers, uint length, uint modifiers);
    [DllImport("winhttp.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool WinHttpSendRequest(IntPtr request, string headers, uint headerLength,
        IntPtr body, uint bodyLength, uint totalLength, IntPtr context);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern bool WinHttpReceiveResponse(IntPtr request, IntPtr reserved);
    [DllImport("winhttp.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool WinHttpQueryHeaders(IntPtr request, uint info, IntPtr name,
        IntPtr buffer, ref uint size, IntPtr index);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern bool WinHttpReadData(IntPtr request, byte[] buffer, uint size, out uint read);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern bool WinHttpQueryDataAvailable(IntPtr request, out uint available);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern bool WinHttpCloseHandle(IntPtr handle);

    sealed class Exchange {
        public Uri Target;
        public string Method;
        public int Timeout;
        public readonly List<string[]> Headers = new List<string[]>();
        public byte[] Body;
        public string Invalid;
    }

    // Kept reachable for as long as WinHTTP may call it.
    static readonly StatusCallback Status = OnStatus;
    static readonly IntPtr[] Sessions = new IntPtr[2];
    static readonly Dictionary<string, IntPtr> Connections = new Dictionary<string, IntPtr>();
    // Whether the current attempt opened a connection. A failure without one
    // happened on a reused connection the server may already have closed.
    static bool opened;

    public static int Main(string[] arguments) {
        if (arguments.Length == 1 && arguments[0] == "socket") return ServeSocket();
        var queue = new BlockingCollection<Exchange>();
        var reader = new Thread(() => Receive(queue));
        reader.IsBackground = true;
        reader.Start();
        try {
            using (var output = Console.OpenStandardOutput()) {
                for (;;) {
                    Exchange exchange;
                    if (!queue.TryTake(out exchange, IdleLimit)) return 0;
                    Serve(exchange, output);
                    if (exchange.Invalid != null) return 1;
                }
            }
        } catch (IOException) {
            // The parent stopped reading; nothing is left to report to.
            return 1;
        }
    }

    // Owns stdin for the helper's whole life, so a parent that exits ends the
    // helper even during a long response.
    static void Receive(BlockingCollection<Exchange> queue) {
        using (var input = new BufferedStream(Console.OpenStandardInput())) {
            for (;;) {
                string first = Line(input);
                if (first == null) Environment.Exit(0);
                var exchange = new Exchange();
                try {
                    Parse(first, input, exchange);
                } catch (Exception error) {
                    exchange.Invalid = Describe(error);
                    queue.Add(exchange);
                    return;
                }
                queue.Add(exchange);
            }
        }
    }

    static void Parse(string first, Stream input, Exchange exchange) {
        string url = DecodeText(first);
        exchange.Method = DecodeText(Required(input));
        exchange.Timeout = Number(Required(input));
        int count = Number(Required(input));
        if (count > 1000 || !Token(exchange.Method))
            throw new InvalidDataException("Native HTTP request is invalid");
        exchange.Target = new Uri(url, UriKind.Absolute);
        Uri uri = exchange.Target;
        if ((uri.Scheme != "http" && uri.Scheme != "https") ||
            uri.UserInfo.Length != 0 || uri.Host.Length == 0)
            throw new InvalidDataException("Native HTTP URL is invalid");
        for (int i = 0; i < count; i++) {
            string name = DecodeText(Required(input));
            string value = DecodeText(Required(input));
            if (!Token(name) || value.IndexOfAny(new char[] { '\r', '\n', '\0' }) >= 0)
                throw new InvalidDataException("Native HTTP request header is invalid");
            exchange.Headers.Add(new string[] { name, value });
        }
        exchange.Body = new byte[Number(Required(input))];
        for (int filled = 0; filled < exchange.Body.Length;) {
            int read = input.Read(exchange.Body, filled, exchange.Body.Length - filled);
            if (read <= 0) throw new EndOfStreamException("Native HTTP request is incomplete");
            filled += read;
        }
    }

    static void Serve(Exchange exchange, Stream output) {
        Frame(output, 'A', null, 0);
        string failure = exchange.Invalid ?? "";
        if (exchange.Invalid == null) {
            try {
                Perform(exchange, output);
            } catch (Exception error) {
                failure = Describe(error);
            }
        }
        byte[] message = Encoding.UTF8.GetBytes(failure);
        Frame(output, 'E', message, message.Length);
    }

    static string Describe(Exception error) {
        bool described = error is IOException || error is InvalidDataException;
        return described && error.Message.StartsWith("Native HTTP")
            ? error.Message : "Native HTTP request failed: " + error.GetType().Name;
    }

    static void OnStatus(IntPtr handle, IntPtr context, uint status, IntPtr info, uint length) {
        if (status == ConnectingToServer) opened = true;
    }

    static IntPtr Session(Uri uri) {
        int index = uri.IsLoopback ? 0 : 1;
        if (Sessions[index] != IntPtr.Zero) return Sessions[index];
        IntPtr session = WinHttpOpen(null, uri.IsLoopback ? NoProxy : AutomaticProxy, null, null, 0);
        Require(session, "open");
        uint protocol = Http2;
        // Older systems reject the option and stay on HTTP/1.1.
        WinHttpSetOption(session, EnableHttpProtocol, ref protocol, sizeof(uint));
        WinHttpSetStatusCallback(session, Status, ConnectNotifications, IntPtr.Zero);
        Sessions[index] = session;
        return session;
    }

    static IntPtr Connection(Uri uri) {
        string key = uri.Host + ":" + uri.Port.ToString(CultureInfo.InvariantCulture);
        IntPtr connection;
        if (Connections.TryGetValue(key, out connection)) return connection;
        if (Connections.Count >= ConnectionHandleLimit) {
            foreach (IntPtr handle in Connections.Values) WinHttpCloseHandle(handle);
            Connections.Clear();
        }
        connection = WinHttpConnect(Session(uri), uri.Host, checked((ushort)uri.Port), 0);
        Require(connection, "connect");
        Connections[key] = connection;
        return connection;
    }

    static void Perform(Exchange exchange, Stream output) {
        Uri uri = exchange.Target;
        IntPtr connection = Connection(uri);
        // Without a total limit, the per-operation limits still bound silence.
        int connectTimeout = exchange.Timeout == 0 ? 10000 : Math.Min(exchange.Timeout, 10000);
        int receiveTimeout = exchange.Timeout == 0 ? 120000 : Math.Min(exchange.Timeout, 120000);
        uint flags = uri.Scheme == "https" ? Secure : 0;
        for (int attempt = 0;; attempt++) {
            IntPtr request = WinHttpOpenRequest(connection, exchange.Method, uri.PathAndQuery, IntPtr.Zero,
                IntPtr.Zero, IntPtr.Zero, flags);
            Require(request, "request");
            try {
                Check(WinHttpSetTimeouts(request, connectTimeout, connectTimeout,
                    receiveTimeout, receiveTimeout), "timeouts");
                uint disabled = DisableCookiesRedirectsAuth;
                Check(WinHttpSetOption(request, DisableFeature, ref disabled, sizeof(uint)), "features");
                if (uri.Scheme == "https") {
                    uint revocation = EnableSslRevocation;
                    Check(WinHttpSetOption(request, EnableFeature, ref revocation, sizeof(uint)), "revocation");
                }
                foreach (string[] header in exchange.Headers) {
                    Check(WinHttpAddRequestHeaders(request, header[0] + ": " + header[1] + "\r\n",
                        UInt32.MaxValue, AddHeader), "header");
                }
                opened = false;
                string stage = Transmit(request, exchange.Body);
                if (stage != null) {
                    // A failed receive does not prove that the server did not
                    // process a request. Only safe methods may be retried here.
                    int code = Marshal.GetLastWin32Error();
                    if (!opened && attempt < StaleRetries && Stale(code) &&
                        (exchange.Method == "GET" || exchange.Method == "HEAD")) continue;
                    throw Failure(stage, code);
                }
                Copy(request, exchange.Method, output);
                return;
            } finally {
                WinHttpCloseHandle(request);
            }
        }
    }

    // Returns the failed stage, or null once the response headers arrived.
    static string Transmit(IntPtr request, byte[] body) {
        GCHandle pinned = default(GCHandle);
        try {
            IntPtr pointer = IntPtr.Zero;
            if (body.Length != 0) {
                pinned = GCHandle.Alloc(body, GCHandleType.Pinned);
                pointer = pinned.AddrOfPinnedObject();
            }
            uint length = (uint)body.Length;
            if (!WinHttpSendRequest(request, null, 0, pointer, length, length, IntPtr.Zero)) return "send";
            if (!WinHttpReceiveResponse(request, IntPtr.Zero)) return "receive";
            return null;
        } finally {
            if (pinned.IsAllocated) pinned.Free();
        }
    }

    static void Copy(IntPtr request, string method, Stream output) {
        ulong? expected;
        byte[] headers = ResponseHeaders(request, out expected);
        Frame(output, 'D', headers, headers.Length);
        if (method == "HEAD") return;
        byte[] buffer = new byte[16384];
        ulong received = 0;
        // A framed response is complete at Content-Length; a
        // later connection close must not discard its body.
        while (!expected.HasValue || received < expected.Value) {
            uint available;
            Check(WinHttpQueryDataAvailable(request, out available), "availability");
            uint read;
            uint wanted = available == 0 ? 1 : Math.Min(available, (uint)buffer.Length);
            Check(WinHttpReadData(request, buffer, wanted, out read), "body");
            if (read == 0) break;
            received += read;
            if (expected.HasValue && received > expected.Value)
                throw new InvalidDataException("Native HTTP response length is invalid");
            Frame(output, 'D', buffer, (int)read);
        }
        if (expected.HasValue && received != expected.Value)
            throw new InvalidDataException("Native HTTP response is incomplete");
    }

    static void Frame(Stream output, char tag, byte[] bytes, int count) {
        byte[] frame = new byte[5 + count];
        frame[0] = (byte)tag;
        frame[1] = (byte)count;
        frame[2] = (byte)(count >> 8);
        frame[3] = (byte)(count >> 16);
        frame[4] = (byte)(count >> 24);
        if (count != 0) Buffer.BlockCopy(bytes, 0, frame, 5, count);
        output.Write(frame, 0, frame.Length);
        output.Flush();
    }

    static byte[] ResponseHeaders(IntPtr request, out ulong? expected) {
        uint size = 0;
        if (WinHttpQueryHeaders(request, RawHeadersCrlf, IntPtr.Zero, IntPtr.Zero, ref size, IntPtr.Zero) ||
            Marshal.GetLastWin32Error() != InsufficientBuffer || size < 4 || size > 128 * 1024)
            throw new InvalidDataException("Native HTTP response headers are invalid");
        IntPtr buffer = Marshal.AllocHGlobal((int)size);
        try {
            Check(WinHttpQueryHeaders(request, RawHeadersCrlf, IntPtr.Zero, buffer, ref size, IntPtr.Zero),
                "response headers");
            string text = Marshal.PtrToStringUni(buffer, (int)size / 2).TrimEnd('\0');
            int firstSpace = text.IndexOf(' ');
            int status;
            if (firstSpace < 0 || text.Length < firstSpace + 4 ||
                !Int32.TryParse(text.Substring(firstSpace + 1, 3), NumberStyles.None,
                    CultureInfo.InvariantCulture, out status))
                throw new InvalidDataException("Native HTTP response status is invalid");
            expected = null;
            bool chunked = false;
            foreach (string line in text.Split(new string[] { "\r\n" }, StringSplitOptions.None)) {
                int colon = line.IndexOf(':');
                if (colon < 0) continue;
                string name = line.Substring(0, colon);
                string value = line.Substring(colon + 1).Trim();
                if (name.Equals("Content-Length", StringComparison.OrdinalIgnoreCase)) {
                    ulong length;
                    if (!UInt64.TryParse(value, NumberStyles.None, CultureInfo.InvariantCulture, out length) ||
                        (expected.HasValue && expected.Value != length))
                        throw new InvalidDataException("Native HTTP response length is invalid");
                    expected = length;
                }
                if (name.Equals("Transfer-Encoding", StringComparison.OrdinalIgnoreCase))
                    chunked = true;
            }
            if (chunked && expected.HasValue)
                throw new InvalidDataException("Native HTTP response framing is invalid");
            if (status < 200 || status == 204 || status == 304) expected = null;
            text = text.TrimEnd('\r', '\n') + "\r\n\r\n";
            return Encoding.UTF8.GetBytes(text);
        } finally {
            Marshal.FreeHGlobal(buffer);
        }
    }

    static bool Stale(int code) {
        return code == ConnectionError || code == ConnectionReset || code == InvalidServerResponse;
    }
    static void Require(IntPtr handle, string stage) {
        if (handle == IntPtr.Zero) throw Failure(stage, Marshal.GetLastWin32Error());
    }
    static void Check(bool success, string stage) {
        if (!success) throw Failure(stage, Marshal.GetLastWin32Error());
    }
    static Exception Failure(string stage, int code) {
        return new IOException(code == RequestTimeout
            ? "Native HTTP request timed out"
            : "Native HTTP " + stage + " failed (" + code + ")");
    }
    // The next line, or null at the end of the input.
    static string Line(Stream input) {
        var line = new StringBuilder();
        for (;;) {
            int next = input.ReadByte();
            if (next < 0) return line.Length == 0 ? null : line.ToString();
            if (next == '\n') break;
            if (next != '\r') line.Append((char)next);
        }
        return line.ToString();
    }
    static string Required(Stream input) {
        string line = Line(input);
        if (line == null) throw new EndOfStreamException("Native HTTP request is incomplete");
        return line;
    }
    static int Number(string line) {
        int value;
        if (!Int32.TryParse(line, NumberStyles.None, CultureInfo.InvariantCulture, out value))
            throw new InvalidDataException("Native HTTP request is invalid");
        return value;
    }
    static string DecodeText(string line) {
        return new UTF8Encoding(false, true).GetString(Convert.FromBase64String(line));
    }
    static bool Token(string value) {
        if (value.Length == 0) return false;
        foreach (char c in value) {
            if (!((c >= '0' && c <= '9') || (c >= 'A' && c <= 'Z') ||
                  (c >= 'a' && c <= 'z') || "!#$%&'*+-.^_`|~".IndexOf(c) >= 0))
                return false;
        }
        return true;
    }
}
