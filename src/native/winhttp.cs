// Windows HTTP boundary through WinHTTP. This helper is embedded at build time.
// Stdin: base64 URL, base64 method, timeout milliseconds, header count,
// base64 name/value pairs, base64 body. Stdout: HTTP headers followed by body.
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Globalization;

public static class JecodeHttp {
    const uint NoProxy = 1;
    const uint AutomaticProxy = 4;
    const uint Secure = 0x00800000;
    const uint DisableFeature = 63;
    const uint DisableCookiesRedirectsAuth = 0x00000007;
    const uint EnableFeature = 79;
    const uint EnableSslRevocation = 0x00000001;
    const uint RawHeadersCrlf = 22;
    const uint AddHeader = 0x20000000;
    const int InsufficientBuffer = 122;
    const int RequestTimeout = 12002;

    [DllImport("winhttp.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr WinHttpOpen(string agent, uint access, string proxy, string bypass, uint flags);
    [DllImport("winhttp.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr WinHttpConnect(IntPtr session, string server, ushort port, uint reserved);
    [DllImport("winhttp.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr WinHttpOpenRequest(IntPtr connection, string method, string path,
        IntPtr version, IntPtr referrer, IntPtr acceptTypes, uint flags);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern bool WinHttpSetTimeouts(IntPtr session, int resolve, int connect, int send, int receive);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern bool WinHttpSetOption(IntPtr handle, uint option, ref uint value, uint size);
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

    public static int Main() {
        IntPtr session = IntPtr.Zero, connection = IntPtr.Zero, request = IntPtr.Zero;
        try {
            using (var input = new StreamReader(Console.OpenStandardInput(), Encoding.ASCII)) {
                string url = DecodeText(Required(input));
                string method = DecodeText(Required(input));
                int timeout = int.Parse(Required(input));
                int count = int.Parse(Required(input));
                if (timeout < 1 || count < 0 || count > 1000 || !Token(method))
                    throw new InvalidDataException();
                var uri = new Uri(url, UriKind.Absolute);
                if ((uri.Scheme != "http" && uri.Scheme != "https") ||
                    uri.UserInfo.Length != 0 || uri.Host.Length == 0)
                    throw new InvalidDataException();
                session = WinHttpOpen(null, uri.IsLoopback ? NoProxy : AutomaticProxy, null, null, 0);
                Require(session, "open");
                int receiveTimeout = Math.Min(timeout, 120000);
                Check(WinHttpSetTimeouts(session, 10000, 10000, receiveTimeout, receiveTimeout), "timeouts");
                connection = WinHttpConnect(session, uri.Host, checked((ushort)uri.Port), 0);
                Require(connection, "connect");
                uint flags = uri.Scheme == "https" ? Secure : 0;
                request = WinHttpOpenRequest(connection, method, uri.PathAndQuery, IntPtr.Zero,
                    IntPtr.Zero, IntPtr.Zero, flags);
                Require(request, "request");
                uint disabled = DisableCookiesRedirectsAuth;
                Check(WinHttpSetOption(request, DisableFeature, ref disabled, sizeof(uint)), "features");
                if (uri.Scheme == "https") {
                    uint revocation = EnableSslRevocation;
                    Check(WinHttpSetOption(request, EnableFeature, ref revocation, sizeof(uint)), "revocation");
                }
                for (int i = 0; i < count; i++) {
                    string name = DecodeText(Required(input));
                    string value = DecodeText(Required(input));
                    if (!Token(name) || value.IndexOfAny(new char[] { '\r', '\n', '\0' }) >= 0)
                        throw new InvalidDataException();
                    Check(WinHttpAddRequestHeaders(request, name + ": " + value + "\r\n",
                        UInt32.MaxValue, AddHeader), "header");
                }
                byte[] body = Convert.FromBase64String(Required(input));
                if (body.LongLength > UInt32.MaxValue) throw new InvalidDataException();
                GCHandle pinned = default(GCHandle);
                try {
                    IntPtr pointer = IntPtr.Zero;
                    if (body.Length != 0) {
                        pinned = GCHandle.Alloc(body, GCHandleType.Pinned);
                        pointer = pinned.AddrOfPinnedObject();
                    }
                    uint length = (uint)body.Length;
                    Check(WinHttpSendRequest(request, null, 0, pointer, length, length, IntPtr.Zero), "send");
                    Check(WinHttpReceiveResponse(request, IntPtr.Zero), "receive");
                } finally {
                    if (pinned.IsAllocated) pinned.Free();
                }
                using (var output = Console.OpenStandardOutput()) {
                    ulong? expected;
                    byte[] headers = ResponseHeaders(request, out expected);
                    output.Write(headers, 0, headers.Length);
                    output.Flush();
                    if (method != "HEAD") {
                        byte[] buffer = new byte[16384];
                        ulong received = 0;
                        for (;;) {
                            uint available;
                            Check(WinHttpQueryDataAvailable(request, out available), "availability");
                            uint read;
                            uint wanted = available == 0 ? 1 : Math.Min(available, (uint)buffer.Length);
                            Check(WinHttpReadData(request, buffer, wanted, out read), "body");
                            if (read == 0) break;
                            received += read;
                            if (expected.HasValue && received > expected.Value)
                                throw new InvalidDataException("Native HTTP response length is invalid");
                            output.Write(buffer, 0, (int)read);
                            output.Flush();
                        }
                        if (expected.HasValue && received != expected.Value)
                            throw new InvalidDataException("Native HTTP response is incomplete");
                    }
                }
            }
            return 0;
        } catch (Exception error) {
            Console.Error.WriteLine(error is IOException && error.Message.StartsWith("Native HTTP")
                ? error.Message : "Native HTTP request failed: " + error.GetType().Name);
            return 1;
        } finally {
            if (request != IntPtr.Zero) WinHttpCloseHandle(request);
            if (connection != IntPtr.Zero) WinHttpCloseHandle(connection);
            if (session != IntPtr.Zero) WinHttpCloseHandle(session);
        }
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

    static void Require(IntPtr handle, string stage) {
        if (handle == IntPtr.Zero) throw Failure(stage);
    }
    static void Check(bool success, string stage) {
        if (!success) throw Failure(stage);
    }
    static Exception Failure(string stage) {
        int code = Marshal.GetLastWin32Error();
        return new IOException(code == RequestTimeout
            ? "Native HTTP request timed out"
            : "Native HTTP " + stage + " failed (" + code + ")");
    }
    static string Required(StreamReader input) {
        string line = input.ReadLine();
        if (line == null) throw new EndOfStreamException("Native HTTP request is incomplete");
        return line;
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
